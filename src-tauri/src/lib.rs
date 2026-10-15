//! Nova Launcher core, split by concern:
//!
//! - [`config`] — portable settings file
//! - [`game`] — `.minecraft` + version auto-detection
//! - [`java`] — java executable probing
//! - [`version`] — version json parsing (main class / tweak class)
//! - [`classpath`] — which jars and natives the game starts with
//! - [`launch`] — command-line construction and process supervision
//! - [`log_export`] — writing captured console output to disk
//! - [`platform`] — OS-specific portable paths and process/window hygiene

mod classpath;
mod config;
mod game;
mod java;
mod launch;
mod log_export;
mod platform;
mod version;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod real_client_tests;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(launch::RunningGame::default())
        .invoke_handler(tauri::generate_handler![
            config::load_config,
            config::save_config,
            game::detect_setup,
            java::validate_java,
            log_export::write_log_file,
            launch::launch
        ])
        .setup(|app| {
            // Windows only: the config window is built before this hook runs, so
            // the `main` webview already exists here.
            platform::disable_webview2_autofill(app);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
