//! Reading a version json: entry point class and tweak class.

use std::fs;
use std::path::Path;

pub(crate) struct LaunchStyle {
    pub main_class: String,
    pub tweak_class: Option<String>,
}

pub(crate) fn read_version_json(game_dir: &Path, version_id: &str) -> Option<serde_json::Value> {
    let path = game_dir
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.json"));
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Main class + tweak class: taken from the version json when present,
/// otherwise inferred from the version id.
pub(crate) fn launch_style(json: Option<&serde_json::Value>, version_id: &str) -> LaunchStyle {
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
