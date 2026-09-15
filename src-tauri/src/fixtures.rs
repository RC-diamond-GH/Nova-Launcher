//! Shared filesystem fixtures for the crate's unit tests.
//! Compiled only under `cfg(test)`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::LauncherConfig;
use crate::launch::LaunchPlan;

/// Fresh temp directory per `tag`, reused by tests sharing one process.
pub fn fixture(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nova-launcher-test-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"stub").unwrap();
}

/// json-declared libs on disk; rogue old jars intentionally present.
pub fn forge_dir(tag: &str) -> PathBuf {
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
    touch(
        &root.join(
            "libraries/org/lwjgl/lwjgl/lwjgl/2.9.0-nightly-20130420/lwjgl-2.9.0-nightly-20130420.jar",
        ),
    );
    // natives jar on disk, but a natives json entry → excluded from classpath
    touch(
        &root.join(
            "libraries/org/lwjgl/lwjgl/lwjgl-platform/2.9.0/lwjgl-platform-2.9.0-natives-windows.jar",
        ),
    );
    fs::create_dir_all(root.join("natives")).unwrap();
    fs::create_dir_all(root.join("assets")).unwrap();
    root
}

pub fn config_for(root: &Path) -> LauncherConfig {
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

pub fn arg_index(plan: &LaunchPlan, flag: &str) -> usize {
    plan.args.iter().position(|a| a == flag).unwrap()
}
