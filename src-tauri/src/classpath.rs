//! Classpath and natives resolution: which jars the game actually starts with.

use std::fs;
use std::path::{Path, PathBuf};

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
pub(crate) fn classpath_from_json(game_dir: &Path, json: &serde_json::Value) -> Vec<PathBuf> {
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

/// Recursively collects every `*.jar` under `dir`.
pub(crate) fn collect_jars(dir: &Path, out: &mut Vec<PathBuf>) {
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
pub(crate) fn find_natives(game_dir: &Path, version_id: &str) -> Option<PathBuf> {
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
