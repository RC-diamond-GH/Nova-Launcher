//! Writing the frontend's captured console text to a user-chosen path.

use std::fs;

/// Writes exported log text to a user-chosen path.
#[tauri::command]
pub(crate) fn write_log_file(path: String, content: String) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("日志为空，无可导出".to_string());
    }
    fs::write(&path, content).map_err(|e| format!("写入日志失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::fixture;

    #[test]
    fn write_log_file_roundtrip() {
        let root = fixture("log");
        let p = root.join("game.log").display().to_string();
        write_log_file(p.clone(), "line1\nline2".to_string()).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "line1\nline2");
        assert!(write_log_file(p, "   ".to_string()).is_err());
    }
}
