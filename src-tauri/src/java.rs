//! Probing a java executable with `-version` for the settings page's 检测 button.

use std::process::Command;

use serde::Serialize;

use crate::platform::no_window;

#[derive(Debug, Serialize)]
pub(crate) struct JavaInfo {
    pub version: String,
    pub is_64bit: bool,
    pub raw: String,
}

/// Runs `<java> -version` and parses the reported version / bitness.
pub(crate) fn validate_java_at(java_path: &str) -> Result<JavaInfo, String> {
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
pub(crate) fn validate_java(java_path: String) -> Result<JavaInfo, String> {
    validate_java_at(&java_path)
}
