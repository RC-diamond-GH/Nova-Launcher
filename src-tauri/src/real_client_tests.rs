//! Integration checks against the real local test client.
//! Both are `#[ignore]`d — they need the machine-local client + javaw.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use crate::config::LauncherConfig;
use crate::fixtures::arg_index;
use crate::game::{detect_game_dir, pick_version_in};
use crate::java::validate_java_at;
use crate::launch::build_launch_plan;
use crate::platform::no_window;

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
    assert_eq!(plan.args[cp + 2], "net.minecraft.launchwrapper.Launch");
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
