//! Structured installation roots only: no full-disk search or directory-symlink recursion.

use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use super::run_bounded;
use super::{probe_java, JavaDiscovery, JavaInstallation, PROBE_TIMEOUT};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(super) use windows::discover;

const MAX_DIRECTORIES: usize = 4096;
const MAX_CANDIDATES: usize = 128;
#[cfg(windows)]
const HOME_EXECUTABLES: &[&str] = &[
    "bin/java.exe",
    "jre/bin/java.exe",
    "bin/javaw.exe",
    "jre/bin/javaw.exe",
];
#[cfg(not(windows))]
const HOME_EXECUTABLES: &[&str] = &[
    "bin/java",
    "Contents/Home/bin/java",
    "Contents/Home/jre/bin/java",
    "Home/bin/java",
    "jre/bin/java",
    "Commands/java",
];

struct Scanner {
    candidates: BTreeSet<PathBuf>,
    directories: BTreeSet<PathBuf>,
    warnings: BTreeSet<String>,
    placeholder: Option<PathBuf>,
}

impl Scanner {
    fn new(placeholder: Option<PathBuf>) -> Self {
        Self {
            candidates: BTreeSet::new(),
            directories: BTreeSet::new(),
            warnings: BTreeSet::new(),
            placeholder,
        }
    }

    fn warning(&mut self, path: &Path, error: impl std::fmt::Display) {
        self.warnings.insert(format!("{}: {error}", path.display()));
    }

    fn candidate(&mut self, path: PathBuf) {
        // Console and GUI launchers belong to the same JVM. Prefer the console
        // binary for reliable redirected property output, including PATH entries.
        #[cfg(windows)]
        let path = if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("javaw.exe"))
            && path.with_file_name("java.exe").is_file()
        {
            path.with_file_name("java.exe")
        } else {
            path
        };
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == ErrorKind::NotFound => return,
            Err(e) => {
                self.warning(&path, e);
                return;
            }
            Ok(_) => {}
        }
        let real = match fs::canonicalize(&path) {
            Ok(real) => real,
            Err(e) => {
                self.warning(&path, format!("无法解析 Java 实际路径: {e}"));
                return;
            }
        };
        if self.placeholder.as_ref() == Some(&real)
            || cfg!(target_os = "macos") && path == Path::new("/usr/bin/java")
        {
            return;
        }
        match fs::metadata(&real) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                self.warning(&path, "Java 路径不是文件");
                return;
            }
            Err(e) => {
                self.warning(&path, e);
                return;
            }
        }
        if real.to_str().is_none() {
            self.warning(&real, "Java 路径不是有效 UTF-8，无法作为启动器配置保存");
            return;
        }
        if self.candidates.len() >= MAX_CANDIDATES && !self.candidates.contains(&real) {
            self.warnings.insert(format!(
                "Java 候选超过 {MAX_CANDIDATES} 个，已停止添加新候选"
            ));
            return;
        }
        self.candidates.insert(real);
    }

    fn home(&mut self, path: &Path) {
        for suffix in HOME_EXECUTABLES {
            self.candidate(path.join(suffix));
        }
    }

    fn entries(&mut self, path: &Path) -> Vec<PathBuf> {
        let directory = match fs::read_dir(path) {
            Ok(directory) => directory,
            Err(e) if e.kind() == ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                self.warning(path, format!("无法读取扫描目录: {e}"));
                return Vec::new();
            }
        };
        let mut entries = Vec::new();
        for entry in directory {
            match entry {
                Ok(entry) => entries.push(entry.path()),
                Err(e) => self.warning(path, format!("无法读取目录项: {e}")),
            }
        }
        entries.sort();
        entries
    }

    // The initial root may intentionally be an opt/current symlink. Below that
    // root, symlinked homes are inspected but never recursively traversed.
    fn tree(&mut self, path: &Path, depth: usize) {
        self.home(path);
        let canonical = match fs::canonicalize(path) {
            Ok(path) => path,
            Err(e) if e.kind() == ErrorKind::NotFound => return,
            Err(e) => {
                self.warning(path, e);
                return;
            }
        };
        if self.directories.contains(&canonical) || depth == 0 {
            return;
        }
        if self.directories.len() >= MAX_DIRECTORIES {
            self.warnings
                .insert(format!("扫描目录超过 {MAX_DIRECTORIES} 个，已停止深入扫描"));
            return;
        }
        self.directories.insert(canonical);
        for entry in self.entries(path) {
            match fs::symlink_metadata(&entry) {
                Ok(metadata) if is_directory_link(&metadata) => match fs::metadata(&entry) {
                    Ok(target) if target.is_dir() => self.home(&entry),
                    Ok(_) => {}
                    Err(e) if e.kind() == ErrorKind::NotFound => {}
                    Err(e) => self.warning(&entry, format!("无法读取目录链接: {e}")),
                },
                Ok(metadata) if metadata.is_dir() => self.tree(&entry, depth - 1),
                Ok(_) => {}
                Err(e) => self.warning(&entry, e),
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn brew(&mut self, root: &Path) {
        for formula in self.entries(root) {
            let name = formula
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            if name.contains("jdk")
                || name.contains("temurin")
                || name.contains("corretto")
                || name.contains("zulu")
                || name.contains("liberica")
                || name.contains("graalvm")
                || name.contains("sapmachine")
                || name.contains("semeru")
                || name == "java"
            {
                self.tree(&formula, 4);
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn java_home(&mut self) {
        match run_bounded(Path::new("/usr/libexec/java_home"), &["-V"], PROBE_TIMEOUT) {
            Ok(output) => {
                for home in java_home_paths(&output.text) {
                    self.home(&home);
                }
                if !output.status.success() {
                    let detail: String = output.text.chars().take(500).collect();
                    self.warnings.insert(format!(
                        "java_home -V 退出失败（{}）: {detail}",
                        output.status
                    ));
                }
            }
            Err(e) => {
                self.warnings.insert(format!("java_home -V: {e}"));
            }
        }
    }

    fn finish(mut self) -> JavaDiscovery {
        let mut installations = Vec::new();
        for path in &self.candidates {
            match probe_java(path, PROBE_TIMEOUT) {
                Ok(info) => {
                    // Oracle javapath launchers and a JDK's bundled JRE can
                    // resolve to distinct files while loading the same JVM.
                    #[cfg(windows)]
                    let runtime_path = super::property(&info.raw, "java.home")
                        .and_then(|home| fs::canonicalize(Path::new(home).join("bin/java.exe")).ok())
                        .filter(|path| path.is_file() && path.to_str().is_some());
                    #[cfg(windows)]
                    let path = runtime_path.as_deref().unwrap_or(path);
                    installations.push(JavaInstallation {
                        // Both candidate() and runtime_path checked UTF-8.
                        path: path.to_str().unwrap().to_string(),
                        info,
                    });
                }
                Err(e) => {
                    self.warnings.insert(e);
                }
            }
        }
        #[cfg(windows)]
        {
            installations.sort_unstable_by(|a, b| a.path.cmp(&b.path));
            installations.dedup_by(|a, b| a.path == b.path);
        }
        JavaDiscovery {
            supported: true,
            installations,
            warnings: self.warnings.into_iter().collect(),
        }
    }
}

fn is_directory_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Junctions and mount points are reparse points, not necessarily symlinks.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.is_symlink()
    }
}

#[cfg(any(target_os = "macos", test))]
fn java_home_paths(text: &str) -> BTreeSet<PathBuf> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with('/') {
                Some(PathBuf::from(line))
            } else {
                line.find(" /")
                    .map(|offset| PathBuf::from(line[offset + 1..].trim()))
            }
        })
        .collect()
}

#[cfg(target_os = "macos")]
pub(super) fn discover() -> JavaDiscovery {
    let mut scanner = Scanner::new(fs::canonicalize("/usr/bin/java").ok());
    scanner.java_home();
    for root in [
        "/Library/Java/JavaVirtualMachines",
        "/System/Library/Java/JavaVirtualMachines",
        "/System/Library/Frameworks/JavaVM.framework/Versions",
    ] {
        scanner.tree(Path::new(root), 2);
    }
    for home in [
        "/Library/Internet Plug-Ins/JavaAppletPlugin.plugin/Contents/Home",
        "/System/Library/Java/Support/Deploy.bundle/Contents/Home",
    ] {
        scanner.home(Path::new(home));
    }
    if let Some(home) = std::env::var_os("JAVA_HOME").filter(|home| !home.is_empty()) {
        let home = PathBuf::from(home);
        scanner.home(&home);
        if !HOME_EXECUTABLES
            .iter()
            .any(|suffix| home.join(suffix).is_file())
        {
            scanner.warning(&home, "JAVA_HOME 未指向包含 Java 可执行文件的目录");
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            scanner.candidate(directory.join("java"));
        }
    }
    for root in [
        "/opt/homebrew/opt",
        "/opt/homebrew/Cellar",
        "/usr/local/opt",
        "/usr/local/Cellar",
    ] {
        scanner.brew(Path::new(root));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for (suffix, depth) in [
            ("Library/Java/JavaVirtualMachines", 2),
            (".sdkman/candidates/java", 2),
            (".asdf/installs/java", 2),
            (".local/share/mise/installs/java", 2),
            (".local/share/rtx/installs/java", 2),
            (".jabba/jdk", 2),
            (".jenv/versions", 1),
            (".gradle/jdks", 2),
            ("Library/Application Support/minecraft/runtime", 4),
            ("Library/Application Support/PrismLauncher/java", 3),
            ("Library/Application Support/PolyMC/java", 3),
            (".minecraft/runtime", 4),
        ] {
            scanner.tree(&home.join(suffix), depth);
        }
        scan_apps(&mut scanner, &home.join("Applications"));
        scan_apps(&mut scanner, &home.join("Applications/JetBrains Toolbox"));
    }
    for (variable, suffix) in [
        ("SDKMAN_DIR", "candidates/java"),
        ("SDKMAN_CANDIDATES_DIR", "java"),
        ("ASDF_DATA_DIR", "installs/java"),
        ("MISE_DATA_DIR", "installs/java"),
        ("MISE_INSTALLS_DIR", "java"),
        ("JABBA_HOME", "jdk"),
    ] {
        if let Some(root) = std::env::var_os(variable).filter(|root| !root.is_empty()) {
            scanner.tree(&PathBuf::from(root).join(suffix), 2);
        }
    }
    // Only application bundles one grouping directory below Applications. Never
    // walk arbitrary application data, caches or user documents looking for JBR.
    scan_apps(&mut scanner, Path::new("/Applications"));
    scanner.finish()
}

#[cfg(target_os = "macos")]
fn scan_apps(scanner: &mut Scanner, root: &Path) {
    for entry in scanner.entries(root) {
        let is_bundle = entry
            .extension()
            .is_some_and(|extension| extension == "app");
        if is_bundle {
            for suffix in [
                "Contents/jbr",
                "Contents/jre",
                "Contents/runtime",
                "Contents/Home",
            ] {
                scanner.home(&entry.join(suffix));
            }
        } else if fs::symlink_metadata(&entry)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.is_symlink())
        {
            // Common JetBrains grouping folders; do not descend into app bundles.
            for app in scanner.entries(&entry) {
                if app.extension().is_some_and(|extension| extension == "app") {
                    for suffix in [
                        "Contents/jbr",
                        "Contents/jre",
                        "Contents/runtime",
                        "Contents/Home",
                    ] {
                        scanner.home(&app.join(suffix));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_home_output_preserves_spaces_and_deduplicates_default_home() {
        let text = "Matching Java Virtual Machines (2):\n    21.0.1 (x86_64) \"Vendor\" - \"JDK\" /Library/Java/JavaVirtualMachines/Test JDK.jdk/Contents/Home\n    1.8.503.01 (arm64) \"Oracle Corporation\" - \"Java\" /Library/Internet Plug-Ins/JavaAppletPlugin.plugin/Contents/Home\n/Library/Java/JavaVirtualMachines/Test JDK.jdk/Contents/Home\n";
        let paths = java_home_paths(text);
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(Path::new(
            "/Library/Java/JavaVirtualMachines/Test JDK.jdk/Contents/Home"
        )));
        assert!(paths.contains(Path::new(
            "/Library/Internet Plug-Ins/JavaAppletPlugin.plugin/Contents/Home"
        )));
    }

    #[test]
    fn absent_common_roots_are_not_warnings() {
        let root = crate::fixtures::fixture("java-no-root");
        let mut scanner = Scanner::new(None);
        scanner.tree(&root.join("missing"), 4);
        assert!(scanner.warnings.is_empty());
        assert!(scanner.candidates.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn console_and_gui_candidates_share_one_runtime_but_gui_only_is_kept() {
        let root = crate::fixtures::fixture("java-windows-launchers");
        let console = root.join("JDK 空格/bin/java.exe");
        let gui = root.join("JDK 空格/bin/javaw.exe");
        let gui_only = root.join("legacy/jre/bin/javaw.exe");
        for path in [&console, &gui, &gui_only] {
            crate::fixtures::touch(path);
        }
        let mut scanner = Scanner::new(None);
        scanner.home(&root.join("JDK 空格"));
        scanner.candidate(gui);
        scanner.home(&root.join("legacy"));
        assert_eq!(
            scanner.candidates,
            BTreeSet::from([
                fs::canonicalize(console).unwrap(),
                fs::canonicalize(gui_only).unwrap(),
            ])
        );
        assert!(scanner.warnings.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn junction_homes_are_inspected_without_recursing_into_the_target() {
        let root = crate::fixtures::fixture("java-windows-junction");
        let scanned = root.join("scanned");
        let target = root.join("outside");
        let direct = target.join("bin/java.exe");
        crate::fixtures::touch(&direct);
        crate::fixtures::touch(&target.join("unrelated/nested/bin/java.exe"));
        fs::create_dir_all(&scanned).unwrap();
        let link = scanned.join("linked");
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let mut scanner = Scanner::new(None);
        scanner.tree(&scanned, 8);
        assert_eq!(
            scanner.candidates,
            BTreeSet::from([fs::canonicalize(direct).unwrap()])
        );
        assert!(scanner.warnings.is_empty());
        fs::remove_dir(link).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn canonical_candidates_are_deduplicated_and_placeholder_is_skipped() {
        use std::os::unix::fs::symlink;
        let root = crate::fixtures::fixture("java-discovery-dedup");
        let executable = super::super::tests::script(
            &root,
            "jdk/bin/java",
            "echo 'java version \"21.0.1\"' >&2",
        );
        symlink(root.join("jdk"), root.join("alias")).unwrap();
        symlink(&root, root.join("jdk/cycle")).unwrap();
        let mut scanner = Scanner::new(None);
        scanner.tree(&root, 6);
        scanner.home(&root.join("alias"));
        assert_eq!(scanner.candidates.len(), 1);
        assert!(scanner.warnings.is_empty());
        let result = scanner.finish();
        assert_eq!(result.installations.len(), 1);
        assert_eq!(
            result.installations[0].path,
            fs::canonicalize(&executable).unwrap().to_str().unwrap()
        );
        let mut placeholder = Scanner::new(Some(fs::canonicalize(executable).unwrap()));
        placeholder.home(&root.join("alias"));
        assert!(placeholder.candidates.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn failed_candidates_warn_without_hiding_valid_installations_and_sort_stably() {
        let root = crate::fixtures::fixture("java-discovery-partial");
        super::super::tests::script(&root, "z/bin/java", "echo 'java version \"17\"' >&2");
        super::super::tests::script(&root, "a/bin/java", "echo 'java version \"21\"' >&2");
        super::super::tests::script(
            &root,
            "broken/bin/java",
            "echo 'java version \"99\"' >&2\nexit 2",
        );
        let mut scanner = Scanner::new(None);
        scanner.tree(&root, 2);
        let result = scanner.finish();
        assert_eq!(result.installations.len(), 2);
        assert!(result.installations[0].path < result.installations[1].path);
        assert_eq!(result.installations[0].info.version, "21");
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("broken/bin/java"));
        assert!(result.warnings[0].contains("退出失败"));
    }

    #[cfg(unix)]
    #[test]
    fn depth_bound_ignores_deep_unrelated_trees_but_finds_bundle_homes() {
        let root = crate::fixtures::fixture("java-discovery-depth");
        super::super::tests::script(&root, "JDK.app/Contents/Home/bin/java", "exit 0");
        super::super::tests::script(&root, "one/two/three/four/bin/java", "exit 0");
        let mut scanner = Scanner::new(None);
        scanner.tree(&root, 1);
        assert_eq!(scanner.candidates.len(), 1);
    }
}
