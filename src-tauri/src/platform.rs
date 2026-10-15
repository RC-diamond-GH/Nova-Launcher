//! OS-specific portable paths and process/window hygiene.

use std::path::PathBuf;
use std::process::Command;

/// Finder does not preserve the directory containing a macOS application.
/// Keep portable data beside the .app, never inside Contents or relative to Finder's cwd.
pub(crate) fn launcher_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let executable = std::env::current_exe().map_err(|e| format!("无法获取启动器路径: {e}"))?;
        if let Some(dir) = macos_bundle_parent(&executable) {
            return Ok(dir.to_path_buf());
        }
        // Unbundled development keeps the existing `tauri dev` cwd convention.
        if !cfg!(debug_assertions) {
            return executable
                .parent()
                .map(std::path::Path::to_path_buf)
                .ok_or_else(|| "无法获取启动器所在目录".to_string());
        }
    }
    std::env::current_dir().map_err(|e| format!("无法获取程序运行目录: {e}"))
}

#[cfg(target_os = "macos")]
fn macos_bundle_parent(executable: &std::path::Path) -> Option<&std::path::Path> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    if macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app"
    {
        bundle.parent()
    } else {
        None
    }
}

#[cfg(windows)]
pub(crate) fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub(crate) fn no_window(_cmd: &mut Command) {}

/// WebView2 keeps general autofill on by default, which puts a browser-style
/// suggestion dropdown on the launcher's text inputs; `autocomplete="off"` in
/// the page does not reliably suppress it. The profile switches are the
/// authoritative fix and take effect immediately for every webview sharing the
/// profile. The API needs WebView2 runtime 1.0.1823.32+, so failures are logged
/// and ignored rather than fatal.
#[cfg(windows)]
pub(crate) fn disable_webview2_autofill(app: &tauri::App) {
    use tauri::Manager;
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2_13, ICoreWebView2Profile6};
    use windows_core::Interface;

    let Some(window) = app.get_webview_window("main") else {
        eprintln!("nova: no `main` webview, WebView2 autofill left at its default");
        return;
    };

    let result = window.with_webview(|webview| {
        let configure = || -> windows_core::Result<()> {
            let core = unsafe { webview.controller().CoreWebView2()? };
            let profile = unsafe { core.cast::<ICoreWebView2_13>()?.Profile()? };
            let profile = profile.cast::<ICoreWebView2Profile6>()?;

            unsafe {
                profile.SetIsGeneralAutofillEnabled(false)?;
                profile.SetIsPasswordAutosaveEnabled(false)?;
            }

            Ok(())
        };

        if let Err(err) = configure() {
            eprintln!("nova: could not disable WebView2 autofill: {err}");
        }
    });

    if let Err(err) = result {
        eprintln!("nova: could not reach the `main` WebView2 controller: {err}");
    }
}

#[cfg(not(windows))]
pub(crate) fn disable_webview2_autofill(_app: &tauri::App) {}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn portable_data_lives_beside_bundle_not_inside_it() {
        let root = Path::new("/Users/player/Portable Client 空格");
        let executable = root.join("Nova Launcher.app/Contents/MacOS/nova-launcher");
        assert_eq!(macos_bundle_parent(&executable), Some(root));
    }

    #[test]
    fn nested_app_directory_does_not_replace_the_actual_bundle() {
        let root = Path::new("/Volumes/Clients.app/My Pack");
        let executable = root.join("Nova Launcher.app/Contents/MacOS/nova-launcher");
        assert_eq!(macos_bundle_parent(&executable), Some(root));
    }

    #[test]
    fn ordinary_executable_paths_are_not_mistaken_for_bundles() {
        assert_eq!(
            macos_bundle_parent(Path::new("/Users/player/My Pack/nova-launcher")),
            None
        );
        assert_eq!(
            macos_bundle_parent(Path::new("/Users/player/Client.app/bin/nova-launcher")),
            None
        );
    }
}
