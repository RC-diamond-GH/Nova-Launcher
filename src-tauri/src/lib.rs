use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::SystemTime;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

/// Launcher settings persisted between runs.
/// Legacy config files with removed fields (game_dir, version_id) still parse.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LauncherConfig {
    pub java_path: String,
    pub username: String,
    pub memory_mb: u32,
    /// Collect and stream game output; takes effect on next launch.
    pub logs_enabled: bool,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            java_path: String::new(),
            username: "Player".to_string(),
            memory_mb: 2048,
            logs_enabled: true,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct VersionInfo {
    pub id: String,
    pub has_json: bool,
}

#[derive(Debug, Serialize)]
pub struct JavaInfo {
    pub version: String,
    pub is_64bit: bool,
    pub raw: String,
}

/// What `detect_setup` reports: sibling `.minecraft` + auto-picked version.
#[derive(Debug, Serialize)]
pub struct GameSetup {
    pub game_dir: String,
    pub version_id: String,
}

#[derive(Debug, Serialize)]
pub struct LaunchHandle {
    pub pid: u32,
    pub version_id: String,
}

/// PID of the running game process, if any.
#[derive(Default)]
pub struct RunningGame(Arc<Mutex<Option<u32>>>);

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

/// Portable launcher: settings live in the execution directory, next to the
/// `.minecraft` that `detect_game_dir` auto-detects.
const CONFIG_FILE_NAME: &str = "nova_launcher_config.json";

fn config_path_in(dir: &Path) -> PathBuf {
    dir.join(CONFIG_FILE_NAME)
}

/// No directory creation: the process working directory always exists.
fn config_file() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("无法获取程序运行目录: {e}"))?;
    Ok(config_path_in(&cwd))
}

fn load_config_from(path: &Path) -> Option<LauncherConfig> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_config_to(path: &Path, config: &LauncherConfig) -> Result<(), String> {
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("写入配置失败: {e}"))
}

#[tauri::command]
fn load_config() -> Option<LauncherConfig> {
    load_config_from(&config_file().ok()?)
}

#[tauri::command]
fn save_config(config: LauncherConfig) -> Result<(), String> {
    save_config_to(&config_file()?, &config)
}

/// The `.minecraft` directory is a sibling of the launcher's working directory.
fn detect_game_dir_at(cwd: &Path) -> Result<PathBuf, String> {
    let dir = cwd.join(".minecraft");
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err(format!(
            "当前目录（{}）下未找到 .minecraft，请将启动器与 .minecraft 放在同一目录",
            cwd.display()
        ))
    }
}

fn detect_game_dir() -> Result<PathBuf, String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("无法获取当前目录: {e}"))?;
    detect_game_dir_at(&cwd)
}

/// Auto-picks the version whose jar was modified most recently.
fn pick_version_in(game_dir: &Path) -> Result<String, String> {
    let root = game_dir.join("versions");
    let entries = fs::read_dir(&root)
        .map_err(|_| format!("该目录下没有 versions 子目录: {}", game_dir.display()))?;
    let mut best: Option<(SystemTime, String)> = None;
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(id) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let jar = dir.join(format!("{id}.jar"));
        if !jar.is_file() {
            continue;
        }
        let mtime = fs::metadata(&jar)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if best.as_ref().map_or(true, |(t, _)| mtime > *t) {
            best = Some((mtime, id.to_string()));
        }
    }
    best.map(|(_, id)| id)
        .ok_or_else(|| "versions 目录下没有可启动的版本".to_string())
}

#[tauri::command]
fn detect_setup() -> Result<GameSetup, String> {
    let game_dir = detect_game_dir()?;
    let version_id = pick_version_in(&game_dir)?;
    Ok(GameSetup {
        game_dir: game_dir.display().to_string(),
        version_id,
    })
}

/// Runs `<java> -version` and parses the reported version / bitness.
pub fn validate_java_at(java_path: &str) -> Result<JavaInfo, String> {
    let mut cmd = Command::new(java_path);
    cmd.arg("-version");
    no_window(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("无法运行 {java_path}: {e}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .trim()
    .to_string();
    let version = text
        .lines()
        .find_map(|line| {
            let line = line.trim();
            let start = line.find('"')? + 1;
            let end = start + line[start..].find('"')?;
            Some(line[start..end].to_string())
        })
        .unwrap_or_else(|| "未知".to_string());
    Ok(JavaInfo {
        version,
        is_64bit: text.contains("64-Bit"),
        raw: text,
    })
}

#[tauri::command]
fn validate_java(java_path: String) -> Result<JavaInfo, String> {
    validate_java_at(&java_path)
}

/// Writes exported log text to a user-chosen path.
#[tauri::command]
fn write_log_file(path: String, content: String) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("日志为空，无可导出".to_string());
    }
    fs::write(&path, content).map_err(|e| format!("写入日志失败: {e}"))
}

/// Recursively collects every `*.jar` under `dir`.
fn collect_jars(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jars(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jar"))
        {
            out.push(path);
        }
    }
}

/// Natives live either in `.minecraft/natives` or in a `*natives*` folder
/// inside the version directory (vanilla-launcher layout).
fn find_natives(game_dir: &Path, version_id: &str) -> Option<PathBuf> {
    let direct = game_dir.join("natives");
    if direct.is_dir() {
        return Some(direct);
    }
    let version_dir = game_dir.join("versions").join(version_id);
    let mut candidates: Vec<PathBuf> = fs::read_dir(version_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    candidates.sort();
    candidates.into_iter().find(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.to_ascii_lowercase().contains("natives"))
    })
}

struct LaunchStyle {
    main_class: String,
    tweak_class: Option<String>,
}

fn read_version_json(game_dir: &Path, version_id: &str) -> Option<serde_json::Value> {
    let path = game_dir
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.json"));
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Main class + tweak class: taken from the version json when present,
/// otherwise inferred from the version id.
fn launch_style(json: Option<&serde_json::Value>, version_id: &str) -> LaunchStyle {
    if let Some(value) = json {
        let main = value
            .get("mainClass")
            .and_then(|m| m.as_str())
            .unwrap_or_default();
        let tweak = value
            .get("minecraftArguments")
            .and_then(|a| a.as_str())
            .and_then(|a| {
                a.split_whitespace()
                    .skip_while(|&t| t != "--tweakClass")
                    .nth(1)
                    .map(str::to_string)
            });
        if !main.is_empty() {
            return LaunchStyle {
                main_class: main.to_string(),
                tweak_class: tweak,
            };
        }
    }
    if version_id.to_ascii_lowercase().contains("forge") {
        LaunchStyle {
            main_class: "net.minecraft.launchwrapper.Launch".to_string(),
            tweak_class: Some("cpw.mods.fml.common.launcher.FMLTweaker".to_string()),
        }
    } else {
        LaunchStyle {
            main_class: "net.minecraft.client.main.Main".to_string(),
            tweak_class: None,
        }
    }
}

/// Official-launcher rule semantics: no rules → allowed; otherwise the first
/// rule whose os constraints match wins; no match → not allowed.
/// `os.version` regexes only appear on osx rules and are ignored.
fn rule_allows(rules: &[serde_json::Value]) -> bool {
    let current_os = std::env::consts::OS; // "windows" / "macos" / "linux"
    let current_arch = match std::env::consts::ARCH {
        "x86" => "x86",
        _ => "x64",
    };
    for rule in rules {
        let Some(action) = rule.get("action").and_then(|a| a.as_str()) else {
            continue;
        };
        let matches = match rule.get("os") {
            None => true,
            Some(os) => {
                let name_ok = os
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map_or(true, |n| n == current_os);
                let arch_ok = os
                    .get("arch")
                    .and_then(|a| a.as_str())
                    .map_or(true, |a| a == current_arch);
                name_ok && arch_ok
            }
        };
        if matches {
            return action == "allow";
        }
    }
    false
}

/// `group:artifact:version[:classifier]` → maven repository path.
fn maven_path(name: &str) -> Option<PathBuf> {
    let mut parts = name.split(':');
    let group = parts.next()?;
    let artifact = parts.next()?;
    let version = parts.next()?;
    let file = match parts.next() {
        Some(classifier) => format!("{artifact}-{version}-{classifier}.jar"),
        None => format!("{artifact}-{version}.jar"),
    };
    Some(
        PathBuf::from(group.replace('.', "/"))
            .join(artifact)
            .join(version)
            .join(file),
    )
}

/// Classpath from the version json `libraries` list: skips natives entries,
/// evaluates os rules, and skips jars missing on disk. This prevents stale
/// jars lying around `libraries/` (e.g. an old jopt-simple) from shadowing
/// the versions the json actually declares.
fn classpath_from_json(game_dir: &Path, json: &serde_json::Value) -> Vec<PathBuf> {
    let Some(libs) = json.get("libraries").and_then(|l| l.as_array()) else {
        return Vec::new();
    };
    libs.iter()
        .filter(|lib| lib.get("natives").is_none())
        .filter(|lib| {
            lib.get("rules")
                .and_then(|r| r.as_array())
                .map_or(true, |r| rule_allows(r))
        })
        .filter_map(|lib| lib.get("name").and_then(|n| n.as_str()))
        .filter_map(maven_path)
        .map(|rel| game_dir.join("libraries").join(rel))
        .filter(|p| p.is_file())
        .collect()
}

/// 32 hex chars — unique offline session token per launch.
fn offline_session() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u128)
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    format!("{nanos:016x}{pid:016x}")
}

pub struct LaunchPlan {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

/// Pure command-line construction, unit-testable without spawning anything.
/// `game_dir` / `version_id` come from auto-detection, not from the config.
pub fn build_launch_plan(
    config: &LauncherConfig,
    game_dir: &Path,
    version_id: &str,
) -> Result<LaunchPlan, String> {
    let java = Path::new(&config.java_path);
    if !java.is_file() {
        return Err(format!("Java 可执行文件不存在: {}", config.java_path));
    }
    if !game_dir.is_dir() {
        return Err(format!(".minecraft 目录不存在: {}", game_dir.display()));
    }
    if config.memory_mb < 512 {
        return Err("内存分配至少需要 512 MB".to_string());
    }
    let username = config.username.trim();
    if username.is_empty() {
        return Err("用户名不能为空".to_string());
    }
    let username = username.replace(' ', "_");

    let version_jar = game_dir
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.jar"));
    if !version_jar.is_file() {
        return Err(format!("版本 jar 不存在: {}", version_jar.display()));
    }

    let natives = find_natives(game_dir, version_id)
        .ok_or_else(|| "未找到 natives 目录（.minecraft/natives 或版本目录内）".to_string())?;

    let version_json = read_version_json(game_dir, version_id);

    // json-declared libraries; scan fallback only when the json is absent
    let mut classpath = match &version_json {
        Some(json) => classpath_from_json(game_dir, json),
        None => {
            let mut jars = Vec::new();
            collect_jars(&game_dir.join("libraries"), &mut jars);
            jars.sort();
            jars
        }
    };
    if classpath.is_empty() {
        return Err("classpath 为空：libraries 中没有 json 声明且存在的 jar".to_string());
    }
    classpath.push(version_jar);

    let style = launch_style(version_json.as_ref(), version_id);
    let sep = if cfg!(windows) { ";" } else { ":" };
    let classpath_str = classpath
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(sep);

    let mut args: Vec<String> = vec![
        format!("-Xmx{}m", config.memory_mb),
        "-Dfml.ignoreInvalidMinecraftCertificates=true".to_string(),
        "-Dfml.ignorePatchDiscrepancies=true".to_string(),
        format!("-Djava.library.path={}", natives.display()),
        "-cp".to_string(),
        classpath_str,
        style.main_class,
        "--username".to_string(),
        username,
        "--session".to_string(),
        offline_session(),
        "--version".to_string(),
        version_id.to_string(),
        "--gameDir".to_string(),
        game_dir.display().to_string(),
        "--assetsDir".to_string(),
        game_dir.join("assets").display().to_string(),
    ];
    if let Some(tweak) = style.tweak_class {
        args.push("--tweakClass".to_string());
        args.push(tweak);
    }

    Ok(LaunchPlan {
        program: config.java_path.clone(),
        args,
        cwd: game_dir.to_path_buf(),
    })
}

#[tauri::command]
fn launch(
    app: AppHandle,
    running: State<'_, RunningGame>,
    config: LauncherConfig,
) -> Result<LaunchHandle, String> {
    if running.0.lock().is_some() {
        return Err("游戏已在运行中".to_string());
    }

    let game_dir = detect_game_dir()?;
    let version_id = pick_version_in(&game_dir)?;
    let plan = build_launch_plan(&config, &game_dir, &version_id)?;

    let mut cmd = Command::new(&plan.program);
    cmd.args(&plan.args)
        .current_dir(&plan.cwd)
        .stdin(Stdio::null());

    if config.logs_enabled {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    no_window(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| format!("启动失败: {e}"))?;
    let pid = child.id();
    *running.0.lock() = Some(pid);

    // Stream game output to the frontend as `game-log` events.
    let streams: Vec<Option<Box<dyn std::io::Read + Send>>> = vec![
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ];
    for stream in streams.into_iter().flatten() {
        let app = app.clone();
        thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if app.emit("game-log", line).is_err() {
                    break; // window closed
                }
            }
        });
    }

    let app = app.clone();
    let slot = running.0.clone();
    thread::spawn(move || {
        let code = child.wait().ok().and_then(|s| s.code());
        *slot.lock() = None;
        let _ = app.emit("game-exit", code);
    });

    Ok(LaunchHandle { pid, version_id })
}

/// WebView2 keeps general autofill on by default, which puts a browser-style
/// suggestion dropdown on the launcher's text inputs; `autocomplete="off"` in
/// the page does not reliably suppress it. The profile switches are the
/// authoritative fix and take effect immediately for every webview sharing the
/// profile. The API needs WebView2 runtime 1.0.1823.32+, so failures are logged
/// and ignored rather than fatal.
#[cfg(windows)]
fn disable_webview2_autofill(app: &tauri::App) {
    use tauri::Manager;
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2_13, ICoreWebView2Profile6};
    use windows_core::Interface;

    let Some(window) = app.get_webview_window("main") else {
        eprintln!("nova: no `main` webview, WebView2 autofill left at its default");
        return;
    };

    let result = window.with_webview(|webview| {
        let configure = || -> windows_core::Result<()> {
            let core = unsafe { webview.controller().CoreWebView2()? };
            let profile = unsafe { core.cast::<ICoreWebView2_13>()?.Profile()? };
            let profile = profile.cast::<ICoreWebView2Profile6>()?;

            unsafe {
                profile.SetIsGeneralAutofillEnabled(false)?;
                profile.SetIsPasswordAutosaveEnabled(false)?;
            }

            Ok(())
        };

        if let Err(err) = configure() {
            eprintln!("nova: could not disable WebView2 autofill: {err}");
        }
    });

    if let Err(err) = result {
        eprintln!("nova: could not reach the `main` WebView2 controller: {err}");
    }
}

#[cfg(not(windows))]
fn disable_webview2_autofill(_app: &tauri::App) {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(RunningGame::default())
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            detect_setup,
            validate_java,
            write_log_file,
            launch
        ])
        .setup(|app| {
            // Windows only: the config window is built before this hook runs, so
            // the `main` webview already exists here.
            disable_webview2_autofill(app);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn fixture(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nova-launcher-test-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"stub").unwrap();
    }

    /// json-declared libs on disk; rogue old jars intentionally present.
    fn forge_dir(tag: &str) -> PathBuf {
        let root = fixture(tag);
        touch(&root.join("versions/1.6.4-forge/1.6.4-forge.jar"));
        fs::write(
            root.join("versions/1.6.4-forge/1.6.4-forge.json"),
            r#"{"mainClass":"net.minecraft.launchwrapper.Launch","minecraftArguments":"--username ${auth_player_name} --session ${auth_session} --tweakClass cpw.mods.fml.common.launcher.FMLTweaker","libraries":[
                {"name":"optifine:OptiFine:1.6.4_HD_U_C5"},
                {"name":"net.sf.jopt-simple:jopt-simple:4.5"},
                {"name":"net.minecraft:launchwrapper:1.8"},
                {"name":"org.lwjgl.lwjgl:lwjgl:2.9.0","rules":[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]},
                {"name":"org.lwjgl.lwjgl:lwjgl:2.9.1-nightly-debug3","rules":[{"action":"allow","os":{"name":"osx"}}]},
                {"name":"org.lwjgl.lwjgl:lwjgl-platform:2.9.0","natives":{"windows":"natives-windows"}},
                {"name":"com.example:absent:9.9"}
            ]}"#,
        )
        .unwrap();
        // declared + present
        touch(&root.join("libraries/optifine/OptiFine/1.6.4_HD_U_C5/OptiFine-1.6.4_HD_U_C5.jar"));
        touch(&root.join("libraries/net/sf/jopt-simple/jopt-simple/4.5/jopt-simple-4.5.jar"));
        touch(&root.join("libraries/net/minecraft/launchwrapper/1.8/launchwrapper-1.8.jar"));
        touch(&root.join("libraries/org/lwjgl/lwjgl/lwjgl/2.9.0/lwjgl-2.9.0.jar"));
        // rogue stale jars that must NOT enter the classpath
        touch(&root.join("libraries/net/sf/jopt-simple/jopt-simple/4.4/jopt-simple-4.4.jar"));
        touch(&root.join("libraries/org/lwjgl/lwjgl/lwjgl/2.9.0-nightly-20130420/lwjgl-2.9.0-nightly-20130420.jar"));
        // natives jar on disk, but a natives json entry → excluded from classpath
        touch(&root.join("libraries/org/lwjgl/lwjgl/lwjgl-platform/2.9.0/lwjgl-platform-2.9.0-natives-windows.jar"));
        fs::create_dir_all(root.join("natives")).unwrap();
        fs::create_dir_all(root.join("assets")).unwrap();
        root
    }

    fn config_for(root: &Path) -> LauncherConfig {
        LauncherConfig {
            java_path: root
                .join("libraries/optifine/OptiFine/1.6.4_HD_U_C5/OptiFine-1.6.4_HD_U_C5.jar")
                .display()
                .to_string(), // any existing file
            username: "Test User".to_string(),
            memory_mb: 2048,
            logs_enabled: true,
        }
    }

    fn arg_index(plan: &LaunchPlan, flag: &str) -> usize {
        plan.args.iter().position(|a| a == flag).unwrap()
    }

    #[test]
    fn launch_plan_full() {
        let root = forge_dir("full");
        let plan = build_launch_plan(&config_for(&root), &root, "1.6.4-forge").unwrap();

        assert_eq!(
            plan.program,
            root.join("libraries/optifine/OptiFine/1.6.4_HD_U_C5/OptiFine-1.6.4_HD_U_C5.jar")
                .display()
                .to_string()
        );
        assert_eq!(plan.cwd, root);
        assert_eq!(plan.args[0], "-Xmx2048m");
        assert!(plan
            .args
            .contains(&"-Dfml.ignoreInvalidMinecraftCertificates=true".to_string()));
        assert!(plan
            .args
            .contains(&"-Dfml.ignorePatchDiscrepancies=true".to_string()));

        // classpath: json order, version jar last, exactly the declared set
        let cp = arg_index(&plan, "-cp");
        let classpath = &plan.args[cp + 1];
        let jars: Vec<&str> = classpath.split(';').collect();
        assert_eq!(jars.len(), 5, "got {jars:?}");
        assert!(jars[0].ends_with("OptiFine-1.6.4_HD_U_C5.jar"));
        assert!(jars[1].ends_with("jopt-simple-4.5.jar"));
        assert!(jars[2].ends_with("launchwrapper-1.8.jar"));
        assert!(jars[3].ends_with("lwjgl-2.9.0.jar"));
        assert!(jars[4].ends_with("1.6.4-forge.jar"));
        assert!(classpath.contains(';') || !cfg!(windows));

        // main class from json
        assert_eq!(plan.args[cp + 2], "net.minecraft.launchwrapper.Launch");

        // game args
        let u = arg_index(&plan, "--username");
        assert_eq!(plan.args[u + 1], "Test_User"); // space sanitized
        let s = arg_index(&plan, "--session");
        assert_eq!(plan.args[s + 1].len(), 32);
        assert!(plan.args[s + 1].chars().all(|c| c.is_ascii_hexdigit()));
        let v = arg_index(&plan, "--version");
        assert_eq!(plan.args[v + 1], "1.6.4-forge");
        let g = arg_index(&plan, "--gameDir");
        assert_eq!(plan.args[g + 1], root.display().to_string());
        let a = arg_index(&plan, "--assetsDir");
        assert_eq!(plan.args[a + 1], root.join("assets").display().to_string());
        let t = arg_index(&plan, "--tweakClass");
        assert_eq!(plan.args[t + 1], "cpw.mods.fml.common.launcher.FMLTweaker");

        // natives dir from .minecraft/natives
        let natives = plan
            .args
            .iter()
            .find(|a| a.starts_with("-Djava.library.path="))
            .unwrap();
        assert!(natives.ends_with("natives"));
    }

    /// Regression for the jopt-simple 4.4 shadowing bug: only json-declared
    /// jars enter the classpath; rogue/old/natives/missing entries never do.
    #[test]
    fn classpath_follows_json_not_disk() {
        let root = forge_dir("shadow");
        let plan = build_launch_plan(&config_for(&root), &root, "1.6.4-forge").unwrap();
        let cp = &plan.args[arg_index(&plan, "-cp") + 1];
        assert!(!cp.contains("jopt-simple-4.4"), "stale jopt-simple leaked: {cp}");
        assert!(!cp.contains("nightly"), "nightly jar leaked: {cp}");
        assert!(!cp.contains("lwjgl-platform"), "natives entry leaked: {cp}");
        assert!(!cp.contains("absent"), "missing-on-disk entry leaked: {cp}");
    }

    #[test]
    fn rejects_bad_configs() {
        let root = forge_dir("rejects");
        let base = config_for(&root);

        let mut c = base.clone();
        c.java_path = root.join("missing.exe").display().to_string();
        assert!(build_launch_plan(&c, &root, "1.6.4-forge").is_err());

        let mut c = base.clone();
        c.memory_mb = 256;
        assert!(build_launch_plan(&c, &root, "1.6.4-forge").is_err());

        let mut c = base.clone();
        c.username = "   ".to_string();
        assert!(build_launch_plan(&c, &root, "1.6.4-forge").is_err());

        assert!(build_launch_plan(&base, &root, "unknown").is_err());
        let nowhere = root.join("nowhere");
        assert!(build_launch_plan(&base, &nowhere, "1.6.4-forge").is_err());
    }

    #[test]
    fn vanilla_version_without_json_uses_heuristic() {
        let root = forge_dir("vanilla");
        touch(&root.join("versions/1.6.4/1.6.4.jar"));
        let plan = build_launch_plan(&config_for(&root), &root, "1.6.4").unwrap();
        let cp = arg_index(&plan, "-cp");
        assert_eq!(plan.args[cp + 2], "net.minecraft.client.main.Main");
        assert!(!plan.args.iter().any(|a| a == "--tweakClass"));
    }

    #[test]
    fn missing_natives_is_an_error() {
        let root = forge_dir("nonatives");
        fs::remove_dir_all(root.join("natives")).unwrap();
        assert!(build_launch_plan(&config_for(&root), &root, "1.6.4-forge").is_err());
    }

    #[test]
    fn empty_libraries_is_an_error() {
        let root = forge_dir("nolibs");
        fs::remove_dir_all(root.join("libraries")).unwrap();
        assert!(build_launch_plan(&config_for(&root), &root, "1.6.4-forge").is_err());
    }

    #[test]
    fn natives_fallback_to_version_dir() {
        let root = forge_dir("fallback");
        fs::remove_dir_all(root.join("natives")).unwrap();
        fs::create_dir_all(root.join("versions/1.6.4-forge/1.6.4-forge-natives-1234")).unwrap();

        let plan = build_launch_plan(&config_for(&root), &root, "1.6.4-forge").unwrap();
        let natives = plan
            .args
            .iter()
            .find(|a| a.starts_with("-Djava.library.path="))
            .unwrap();
        assert!(natives.contains("1.6.4-forge-natives-1234"));
    }

    #[test]
    fn detect_game_dir_at_finds_sibling_minecraft() {
        let base = fixture("detect");
        assert!(detect_game_dir_at(&base).is_err());
        let mc = base.join(".minecraft");
        fs::create_dir_all(&mc).unwrap();
        assert_eq!(detect_game_dir_at(&base).unwrap(), mc);
    }

    #[test]
    fn pick_version_prefers_newest_jar() {
        let root = forge_dir("pick");
        let old_jar = root.join("versions/1.6.4-vanilla/1.6.4-vanilla.jar");
        touch(&old_jar);
        let f = fs::OpenOptions::new().write(true).open(&old_jar).unwrap();
        f.set_modified(SystemTime::now() + Duration::from_secs(3600))
            .unwrap();
        assert_eq!(pick_version_in(&root).unwrap(), "1.6.4-vanilla");

        fs::remove_file(&old_jar).unwrap();
        assert_eq!(pick_version_in(&root).unwrap(), "1.6.4-forge");
    }

    #[test]
    fn write_log_file_roundtrip() {
        let root = fixture("log");
        let p = root.join("game.log").display().to_string();
        write_log_file(p.clone(), "line1\nline2".to_string()).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "line1\nline2");
        assert!(write_log_file(p, "   ".to_string()).is_err());
    }

    #[test]
    fn config_roundtrips_in_launcher_dir() {
        let dir = fixture("config");
        let path = config_path_in(&dir);
        assert_eq!(path, dir.join(CONFIG_FILE_NAME));

        let config = LauncherConfig {
            java_path: r"C:\Java\jre1.8\bin\javaw.exe".to_string(),
            username: "RoundTrip".to_string(),
            memory_mb: 4096,
            logs_enabled: false,
        };
        save_config_to(&path, &config).unwrap();
        assert!(path.is_file(), "no config at {}", path.display());

        let loaded = load_config_from(&path).unwrap();
        assert_eq!(loaded.java_path, config.java_path);
        assert_eq!(loaded.username, config.username);
        assert_eq!(loaded.memory_mb, config.memory_mb);
        assert_eq!(loaded.logs_enabled, config.logs_enabled);

        // unreadable/missing file → None, never a panic
        assert!(load_config_from(&dir.join("absent.json")).is_none());
    }

    #[test]
    fn config_tolerates_legacy_fields_and_defaults() {
        let old = r#"{"java_path":"C:\\j\\javaw.exe","game_dir":"D:\\mc","version_id":"1.6.4","username":"RC","memory_mb":1024}"#;
        let cfg: LauncherConfig = serde_json::from_str(old).unwrap();
        assert_eq!(cfg.username, "RC");
        assert_eq!(cfg.memory_mb, 1024);
        assert!(cfg.logs_enabled);

        let minimal = r#"{"username":"X"}"#;
        let cfg: LauncherConfig = serde_json::from_str(minimal).unwrap();
        assert_eq!(cfg.memory_mb, 2048);
        assert!(cfg.logs_enabled);
    }

    /// Integration check against the real local test client.
    #[test]
    #[ignore = "requires the real local game client"]
    fn real_client_launch_plan() {
        let client_root = Path::new(r"E:\Projects\Adventurer_Engine\Resources\testClient");
        let game_dir = client_root.join(".minecraft");
        let java = Path::new(r"C:\Program Files\Java\jre1.8.0_491\bin\javaw.exe");
        if !game_dir.is_dir() || !java.is_file() {
            eprintln!("skip: real client or javaw not present");
            return;
        }

        std::env::set_current_dir(client_root).unwrap();
        let detected = detect_game_dir().unwrap();
        assert_eq!(detected, game_dir);

        let version_id = pick_version_in(&detected).unwrap();
        assert!(
            version_id.to_ascii_lowercase().contains("forge"),
            "got {version_id}"
        );

        let plan = build_launch_plan(
            &LauncherConfig {
                java_path: java.display().to_string(),
                username: "RC".to_string(),
                memory_mb: 2048,
                logs_enabled: true,
            },
            &detected,
            &version_id,
        )
        .unwrap();

        assert_eq!(plan.program, java.display().to_string());
        assert_eq!(plan.cwd, game_dir);
        let cp = arg_index(&plan, "-cp");
        let classpath = &plan.args[cp + 1];
        let jars: Vec<String> = classpath.split(';').map(str::to_string).collect();
        // exactly the 24 jars the working bat uses, in json order, + version jar
        assert_eq!(jars.len(), 25, "got {} jars", jars.len());
        assert!(jars.iter().all(|j| j.starts_with(game_dir.display().to_string().as_str())));
        for expected in [
            "OptiFine-1.6.4_HD_U_C5.jar",
            "minecraftforge-9.11.1.916.jar",
            "launchwrapper-1.8.jar",
            "jopt-simple-4.5.jar",
            "lwjgl-2.9.0.jar",
            "jinput-2.0.5.jar",
            "gson-2.2.2.jar",
        ] {
            assert!(
                jars.iter().any(|j| j.to_lowercase().ends_with(&expected.to_lowercase())),
                "missing {expected}"
            );
        }
        for banned in [
            "jopt-simple-4.4",
            "launchwrapper-1.7",
            "launchwrapper-1.3",
            "launchwrapper-1.2",
            "lwjgl-2.9.0-nightly",
            "lwjgl-platform",
            "jinput-platform",
            "java3d",
            "trove4j",
            "icu4j",
        ] {
            assert!(
                !jars.iter().any(|j| j.to_lowercase().contains(&banned.to_lowercase())),
                "rogue jar {banned} leaked into classpath"
            );
        }
        assert!(jars[0].ends_with("OptiFine-1.6.4_HD_U_C5.jar"), "json order: first jar should be OptiFine");
        assert!(jars[24].ends_with(format!("{version_id}.jar").as_str()));
        assert_eq!(
            plan.args[cp + 2],
            "net.minecraft.launchwrapper.Launch"
        );
        assert_eq!(
            plan.args[arg_index(&plan, "--tweakClass") + 1],
            "cpw.mods.fml.common.launcher.FMLTweaker"
        );
        assert!(plan
            .args
            .iter()
            .any(|a| a.starts_with("-Djava.library.path=") && a.to_lowercase().ends_with("natives")));

        let info = validate_java_at(&java.display().to_string()).unwrap();
        assert!(info.version.starts_with("1.8"), "got {}", info.version);
        eprintln!("real javaw: {} (64bit={})", info.version, info.is_64bit);
    }

    /// Boots the real game through the real launch code path and kills it as
    /// soon as Forge logs its first line (well past the point where the
    /// jopt-simple shadowing bug crashed with NoSuchMethodError).
    #[test]
    #[ignore = "launches the real game briefly"]
    fn real_client_game_boot() {
        use std::sync::mpsc;
        let client_root = Path::new(r"E:\Projects\Adventurer_Engine\Resources\testClient");
        let game_dir = client_root.join(".minecraft");
        let java = Path::new(r"C:\Program Files\Java\jre1.8.0_491\bin\javaw.exe");
        if !game_dir.is_dir() || !java.is_file() {
            eprintln!("skip: real client or javaw not present");
            return;
        }

        let plan = build_launch_plan(
            &LauncherConfig {
                java_path: java.display().to_string(),
                username: "NovaTest".to_string(),
                memory_mb: 2048,
                logs_enabled: true,
            },
            &game_dir,
            "1.6.4-forge.916+optifineC4",
        )
        .unwrap();

        let mut cmd = Command::new(&plan.program);
        cmd.args(&plan.args)
            .current_dir(&plan.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        no_window(&mut cmd);
        let mut child = cmd.spawn().expect("spawn game");

        let (tx, rx) = mpsc::channel::<String>();
        let streams: Vec<Option<Box<dyn std::io::Read + Send>>> = vec![
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        ];
        for stream in streams.into_iter().flatten() {
            let tx = tx.clone();
            thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        let mut saw_fml = false;
        let mut head: Vec<String> = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
        while let Ok(line) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            let low = line.to_lowercase();
            if head.len() < 15 {
                head.push(line.clone());
            }
            if low.contains("forge mod loader") || low.contains("fml") || low.contains("lwjgl") {
                saw_fml = true;
                break;
            }
            if low.contains("exception in thread") || low.contains("nosuchmethoderror") {
                let _ = child.kill();
                let _ = child.wait();
                panic!("game crashed at startup:\n{}", head.join("\n"));
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        assert!(saw_fml, "no FML output within deadline:\n{}", head.join("\n"));
        eprintln!("game booted past classpath stage; first lines:\n{}", head.join("\n"));
    }
}
