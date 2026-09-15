//! OS-specific process/window hygiene.

use std::process::Command;

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
