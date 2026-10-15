//! Portable settings: one JSON file in the launcher directory, next to
//! the `.minecraft` that [`crate::game`] auto-detects.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Launcher settings persisted between runs.
/// Legacy config files with removed fields (game_dir, version_id) still parse.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LauncherConfig {
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

const CONFIG_FILE_NAME: &str = "nova_launcher_config.json";

fn config_path_in(dir: &Path) -> PathBuf {
    dir.join(CONFIG_FILE_NAME)
}

/// No directory creation: the launcher directory already exists.
fn config_file() -> Result<PathBuf, String> {
    Ok(config_path_in(&crate::platform::launcher_dir()?))
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
pub(crate) fn load_config() -> Option<LauncherConfig> {
    load_config_from(&config_file().ok()?)
}

#[tauri::command]
pub(crate) fn save_config(config: LauncherConfig) -> Result<(), String> {
    save_config_to(&config_file()?, &config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::fixture;

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
}
