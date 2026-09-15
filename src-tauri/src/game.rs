//! Locating the game: the sibling `.minecraft` directory and the version to start.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

/// What `detect_setup` reports: sibling `.minecraft` + auto-picked version.
#[derive(Debug, Serialize)]
pub(crate) struct GameSetup {
    pub game_dir: String,
    pub version_id: String,
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

pub(crate) fn detect_game_dir() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("无法获取当前目录: {e}"))?;
    detect_game_dir_at(&cwd)
}

/// Auto-picks the version whose jar was modified most recently.
pub(crate) fn pick_version_in(game_dir: &Path) -> Result<String, String> {
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
pub(crate) fn detect_setup() -> Result<GameSetup, String> {
    let game_dir = detect_game_dir()?;
    let version_id = pick_version_in(&game_dir)?;
    Ok(GameSetup {
        game_dir: game_dir.display().to_string(),
        version_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{fixture, forge_dir, touch};
    use std::time::Duration;

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
}
