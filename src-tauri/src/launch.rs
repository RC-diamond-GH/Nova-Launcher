//! Building the java command line and supervising the spawned process.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::classpath::{classpath_from_json, collect_jars, find_natives};
use crate::config::LauncherConfig;
use crate::game::{detect_game_dir, pick_version_in};
use crate::platform::no_window;
use crate::version::{launch_style, read_version_json};

#[derive(Debug, Serialize)]
pub(crate) struct LaunchHandle {
    pub pid: u32,
    pub version_id: String,
}

/// PID of the running game process, if any.
#[derive(Default)]
pub(crate) struct RunningGame(Arc<Mutex<Option<u32>>>);

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

pub(crate) struct LaunchPlan {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

/// Pure command-line construction, unit-testable without spawning anything.
/// `game_dir` / `version_id` come from auto-detection, not from the config.
pub(crate) fn build_launch_plan(
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
pub(crate) fn launch(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{arg_index, config_for, forge_dir, touch};
    use std::fs;

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
}
