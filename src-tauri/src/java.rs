//! Java executable probing and bounded macOS/Windows installation discovery.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::platform::no_window;

#[cfg(any(target_os = "macos", windows, test))]
mod discovery;

const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_OUTPUT: u64 = 128 * 1024;
static CAPTURE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize)]
pub(crate) struct JavaInfo {
    pub version: String,
    pub is_64bit: bool,
    pub architecture: String,
    pub vendor: String,
    pub raw: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct JavaInstallation {
    pub path: String,
    pub info: JavaInfo,
}

#[derive(Debug, Serialize)]
pub(crate) struct JavaDiscovery {
    pub supported: bool,
    pub installations: Vec<JavaInstallation>,
    pub warnings: Vec<String>,
}

// A file instead of output pipes prevents a noisy process (or an inherited pipe
// held by one of its children) from deadlocking either wait or output collection.
struct Capture {
    file: File,
    path: PathBuf,
}

impl Capture {
    fn new() -> Result<Self, String> {
        for _ in 0..16 {
            let id = CAPTURE_ID.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("nova-java-{}-{id}.output", std::process::id()));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => return Ok(Self { file, path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("无法创建 Java 探测输出文件: {e}")),
            }
        }
        Err("无法创建唯一的 Java 探测输出文件".to_string())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // Rust's Windows file handles include delete sharing; Unix can unlink
        // an open capture as well. No output-reader threads need to be joined.
        let _ = fs::remove_file(&self.path);
    }
}

struct CommandOutput {
    status: ExitStatus,
    text: String,
}

fn run_bounded(
    executable: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<CommandOutput, String> {
    let mut capture = Capture::new()?;
    let stdout = capture.file.try_clone().map_err(|e| e.to_string())?;
    let stderr = capture.file.try_clone().map_err(|e| e.to_string())?;
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    no_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法运行 {}: {e}", executable.display()))?;
    let started = Instant::now();
    let status = loop {
        if capture
            .file
            .metadata()
            .is_ok_and(|metadata| metadata.len() > MAX_OUTPUT)
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{} 输出过长，无法识别 Java 信息",
                executable.display()
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let killed = child.kill();
                let waited = child.wait();
                return Err(format!(
                    "{} 探测超时（{} 毫秒），已终止并回收进程{}{}",
                    executable.display(),
                    timeout.as_millis(),
                    killed
                        .err()
                        .map(|e| format!("；终止失败: {e}"))
                        .unwrap_or_default(),
                    waited
                        .err()
                        .map(|e| format!("；回收失败: {e}"))
                        .unwrap_or_default(),
                ));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("无法等待 {}: {e}", executable.display()));
            }
        }
    };
    capture
        .file
        .seek(SeekFrom::Start(0))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    (&mut capture.file)
        .take(MAX_OUTPUT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("无法读取 Java 探测结果: {e}"))?;
    if bytes.len() as u64 > MAX_OUTPUT {
        return Err(format!(
            "{} 输出过长，无法识别 Java 信息",
            executable.display()
        ));
    }
    Ok(CommandOutput {
        status,
        text: String::from_utf8_lossy(&bytes).trim().to_string(),
    })
}

fn property<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let (key, value) = line.trim().split_once('=')?;
        (key.trim() == name && !value.trim().is_empty()).then(|| value.trim())
    })
}

fn version_from_banner(text: &str) -> Option<&str> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let rest = line
            .strip_prefix("java version ")
            .or_else(|| line.strip_prefix("openjdk version "))
            .or_else(|| line.strip_prefix("openjdk "))?;
        let version = if let Some(quoted) = rest.strip_prefix('"') {
            quoted.split_once('"')?.0
        } else {
            rest.split_whitespace().next()?
        };
        version.chars().next().filter(|c| c.is_ascii_digit())?;
        Some(version)
    })
}

fn parse_java_info(text: String) -> Result<JavaInfo, String> {
    let version = property(&text, "java.version")
        .or_else(|| version_from_banner(&text))
        .ok_or_else(|| "程序成功退出，但未输出可识别的 Java 版本".to_string())?
        .to_string();
    let architecture = match property(&text, "os.arch") {
        Some("aarch64" | "arm64" | "arm64-v8a") => "arm64",
        Some("amd64" | "x86_64" | "x64") => "x86_64",
        Some("x86" | "i386" | "i486" | "i586" | "i686") => "x86",
        Some(other) => other,
        None => "未知",
    }
    .to_string();
    let is_64bit = match property(&text, "sun.arch.data.model")
        .or_else(|| property(&text, "com.ibm.vm.bitmode"))
    {
        Some("64") => true,
        Some("32") => false,
        _ => {
            matches!(
                architecture.as_str(),
                "arm64" | "x86_64" | "ppc64" | "ppc64le" | "s390x" | "riscv64"
            ) || text.contains("64-Bit")
                || text.contains("64-bit")
        }
    };
    let vendor = property(&text, "java.vendor")
        .map(str::to_string)
        .unwrap_or_else(|| {
            if text.contains("Temurin") {
                "Eclipse Adoptium"
            } else if text.contains("Corretto") {
                "Amazon"
            } else if text.contains("Zulu") {
                "Azul"
            } else if text.contains("OpenJDK") {
                "OpenJDK"
            } else if text.contains("Java(TM)") {
                "Oracle"
            } else {
                "未知"
            }
            .to_string()
        });
    Ok(JavaInfo {
        version,
        is_64bit,
        architecture,
        vendor,
        raw: text,
    })
}

fn probe_java(java_path: &Path, timeout: Duration) -> Result<JavaInfo, String> {
    let mut output = run_bounded(
        java_path,
        &["-XshowSettings:properties", "-version"],
        timeout,
    )?;
    if !output.status.success() {
        let lower = output.text.to_ascii_lowercase();
        let unsupported = lower.contains("xshowsettings")
            && [
                "unrecognized",
                "unrecognised",
                "unknown",
                "unsupported",
                "not supported",
            ]
            .iter()
            .any(|word| lower.contains(word));
        if unsupported {
            output = run_bounded(java_path, &["-version"], timeout)?;
        }
    }
    if !output.status.success() {
        let summary: String = output.text.chars().take(600).collect();
        return Err(format!(
            "{} 退出失败（{}）: {}",
            java_path.display(),
            output.status,
            summary
        ));
    }
    parse_java_info(output.text).map_err(|e| format!("{}: {e}", java_path.display()))
}

pub(crate) fn validate_java_at(java_path: &str) -> Result<JavaInfo, String> {
    probe_java(Path::new(java_path), PROBE_TIMEOUT)
}

#[tauri::command]
pub(crate) async fn validate_java(java_path: String) -> Result<JavaInfo, String> {
    tauri::async_runtime::spawn_blocking(move || validate_java_at(&java_path))
        .await
        .map_err(|e| format!("Java 探测任务失败: {e}"))?
}

#[tauri::command]
pub(crate) async fn discover_java() -> Result<JavaDiscovery, String> {
    #[cfg(any(target_os = "macos", windows))]
    {
        tauri::async_runtime::spawn_blocking(discovery::discover)
            .await
            .map_err(|e| format!("Java 扫描任务失败: {e}"))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Ok(JavaDiscovery {
            supported: false,
            installations: Vec::new(),
            warnings: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_report_jvm_architecture_not_host_architecture() {
        let info = parse_java_info("Property settings:\n    java.version = 1.8.0_503\n    java.vendor = Oracle Corporation\n    os.arch = aarch64\n    sun.arch.data.model = 64\njava version \"1.8.0_503\"".to_string()).unwrap();
        assert_eq!(info.version, "1.8.0_503");
        assert_eq!(info.architecture, "arm64");
        assert_eq!(info.vendor, "Oracle Corporation");
        assert!(info.is_64bit);
        let intel = parse_java_info(
            "java.version = 21.0.5\nos.arch = amd64\nsun.arch.data.model = 64".into(),
        )
        .unwrap();
        assert_eq!(intel.architecture, "x86_64");
        let x86 = parse_java_info(
            "java.version = 1.7.0\nos.arch = i386\nsun.arch.data.model = 32".into(),
        )
        .unwrap();
        assert_eq!(x86.architecture, "x86");
        assert!(!x86.is_64bit);
    }

    #[test]
    fn legacy_banner_does_not_guess_instruction_set_from_bitness() {
        let info =
            parse_java_info("java version \"1.6.0_65\"\nJava HotSpot(TM) 64-Bit Server VM".into())
                .unwrap();
        assert_eq!(info.version, "1.6.0_65");
        assert!(info.is_64bit);
        assert_eq!(info.architecture, "未知");
        assert!(parse_java_info("not a JVM: \"1.2\"".into()).is_err());
        assert_eq!(
            parse_java_info("openjdk 25 2025-09-16\nOpenJDK 64-Bit Server VM".into())
                .unwrap()
                .version,
            "25"
        );
    }

    #[cfg(unix)]
    pub(super) fn script(root: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn failed_exit_is_not_a_java_even_with_valid_banner() {
        let root = crate::fixtures::fixture("java-failed-exit");
        let executable = script(&root, "java", "echo 'java version \"17.0.1\"' >&2\nexit 7");
        let error = probe_java(&executable, PROBE_TIMEOUT).unwrap_err();
        assert!(error.contains("退出失败"));
        assert!(error.contains('7'));
    }

    #[cfg(unix)]
    #[test]
    fn old_java_falls_back_only_for_unsupported_settings() {
        let root = crate::fixtures::fixture("java-legacy-option");
        let executable = script(&root, "java", "if [ \"$1\" != '-version' ]; then\n echo 'Unrecognized option: -XshowSettings:properties' >&2\n exit 1\nfi\necho 'java version \"1.6.0_65\"' >&2");
        assert_eq!(
            probe_java(&executable, PROBE_TIMEOUT).unwrap().version,
            "1.6.0_65"
        );
    }

    #[cfg(unix)]
    #[test]
    fn hanging_process_is_killed_and_reaped() {
        let root = crate::fixtures::fixture("java-timeout");
        let executable = script(&root, "java", "exec /bin/sleep 10");
        let start = Instant::now();
        assert!(probe_java(&executable, Duration::from_millis(50))
            .unwrap_err()
            .contains("超时"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn inherited_output_handles_do_not_extend_probe_lifetime() {
        // Isolate inherited output handle behavior from startup of a newly
        // created executable by invoking the system shell directly.
        let start = Instant::now();
        assert_eq!(
            parse_java_info(
                run_bounded(
                    Path::new("/bin/sh"),
                    &["-c", "/bin/sleep 10 &\necho 'java version \"17\"' >&2"],
                    PROBE_TIMEOUT,
                )
                .unwrap()
                .text,
            )
            .unwrap()
            .version,
            "17"
        );
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn large_output_does_not_deadlock_pipes() {
        let root = crate::fixtures::fixture("java-large-output");
        let executable = script(&root, "java", "i=0\nwhile [ \"$i\" -lt 6000 ]; do\n echo 'abcdefghijklmnopqrstuvwxyz0123456789' >&2\n i=$((i + 1))\ndone\necho 'java version \"17\"' >&2");
        assert!(probe_java(&executable, PROBE_TIMEOUT)
            .unwrap_err()
            .contains("输出过长"));
    }
}
