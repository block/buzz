//! Launch Buzz when the user signs in.
//!
//! Linux and Windows keep the process in the tray after the window closes, so
//! the same process must come back after a reboot. Autostart passes `--hidden`
//! so login does not pop the main window; the tray (or a second launch) shows
//! it. macOS already has Dock/reopen behavior and is left unchanged.

use tauri::{AppHandle, Runtime};

/// Extra argv the autostart entry passes so a login launch stays in the tray.
pub const HIDDEN_LAUNCH_ARG: &str = "--hidden";

pub fn install<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        builder.plugin(
            tauri_plugin_autostart::Builder::new()
                .args([HIDDEN_LAUNCH_ARG])
                .app_name("Buzz")
                .build(),
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        builder
    }
}

/// Register Buzz as a login item. Failures are logged; the tray still works
/// for the current session.
pub fn enable_on_setup<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        use tauri_plugin_autostart::ManagerExt;
        let autolaunch = app.autolaunch();
        match autolaunch.is_enabled() {
            Ok(true) => {}
            Ok(false) => {
                if let Err(error) = autolaunch.enable() {
                    eprintln!("buzz-desktop: failed to enable launch at login: {error}");
                }
            }
            Err(error) => {
                eprintln!("buzz-desktop: failed to read launch-at-login state: {error}");
            }
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = app;
    }
}

pub fn args_request_hidden<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .any(|arg| arg.as_ref() == HIDDEN_LAUNCH_ARG)
}

/// True when this process was started by the login-item with `--hidden`.
pub fn launched_hidden() -> bool {
    args_request_hidden(std::env::args())
}

#[cfg(test)]
mod tests {
    use super::{args_request_hidden, HIDDEN_LAUNCH_ARG};

    #[test]
    fn hidden_flag_is_detected_anywhere_in_argv() {
        assert!(args_request_hidden(["buzz-desktop", HIDDEN_LAUNCH_ARG]));
        assert!(args_request_hidden([HIDDEN_LAUNCH_ARG]));
        assert!(!args_request_hidden(["buzz-desktop"]));
        assert!(!args_request_hidden(["buzz-desktop", "--help"]));
    }
}
