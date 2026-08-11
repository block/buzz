//! Native desktop-notification helpers.
//!
//! `tauri-plugin-notification` posts a notification by calling `notify_rust`'s
//! `show()` and then immediately dropping the returned `NotificationHandle`.
//! That handle owns the D-Bus connection used to post the notification, and on
//! GNOME 46+ (Ubuntu 24.04+, Fedora 41+) tearing that connection down dismisses
//! the notification the instant it appears — so notifications never show.
//! See tauri-apps/plugins-workspace#2566 and hoodie/notify-rust#218.
//!
//! We side-step the plugin on Linux by posting the notification from a
//! dedicated thread that holds the connection open (via `wait_for_action`)
//! until the notification is closed. The same wait surfaces the default click
//! action, which we forward to the frontend so it can focus the window and
//! route to the notification target.

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) const NATIVE_NOTIFICATION_ACTIVATED_EVENT: &str = "native-notification-activated";

/// Show a desktop notification natively.
///
/// Linux uses the connection-preserving D-Bus path described above. macOS uses
/// one application-lifetime `UNUserNotificationCenterDelegate`; it does not
/// allocate a listener or waiter for each notification.
#[tauri::command]
pub async fn show_native_notification(
    app: tauri::AppHandle,
    title: String,
    body: Option<String>,
    target: Option<serde_json::Value>,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        linux::show(app, title, body, target);
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let _ = app;
        crate::macos_notifications::show(title, body, target).await
    }

    #[cfg(target_os = "windows")]
    {
        windows::show(app, title, body, target);
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (&app, &title, &body, &target);
        Err("show_native_notification is not supported on this platform".to_string())
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::sync::Once;

    static AUMID_REGISTERED: Once = Once::new();

    pub fn show(
        app: tauri::AppHandle,
        title: String,
        body: Option<String>,
        _target: Option<serde_json::Value>,
    ) {
        let app_id = app.config().identifier.clone();
        ensure_aumid_registered(&app, &app_id);

        std::thread::spawn(move || {
            let mut toast = tauri_winrt_notification::Toast::new(&app_id).text1(&title);
            if let Some(body_text) = body.as_deref() {
                toast = toast.text2(body_text);
            }

            match toast.show() {
                Ok(_) => {}
                Err(error) => {
                    eprintln!("buzz-desktop: failed to post Windows native notification: {error}");
                }
            }
        });
    }

    /// Registers Buzz's AUMID so Windows will accept native toast
    /// notifications and list Buzz under Settings > Notifications.
    ///
    /// Buzz ships an unpackaged (non-MSIX) Win32 binary with no Start Menu
    /// shortcut carrying a `System.AppUserModel.ID` property, so the shell
    /// has no other way to learn Buzz's AUMID or icon. Without both of the
    /// steps below, `ToastNotifier` silently drops every toast:
    ///   1. The current process must explicitly claim the AUMID it will pass
    ///      to `Toast::new` (`SetCurrentProcessExplicitAppUserModelID`).
    ///   2. That same AUMID must be registered under
    ///      `HKCU\Software\Classes\AppUserModelId\<aumid>` with a display
    ///      name and icon, or Windows has nothing to show in the
    ///      notification settings list and rejects the toast.
    ///
    /// Runs at most once per process.
    fn ensure_aumid_registered(app: &tauri::AppHandle, app_id: &str) {
        AUMID_REGISTERED.call_once(|| {
            set_process_aumid(app_id);
            if let Err(error) = write_aumid_registry_entry(app, app_id) {
                eprintln!(
                    "buzz-desktop: failed to register AUMID for Windows notifications: {error}"
                );
            }
        });
    }

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn set_process_aumid(app_id: &str) {
        use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

        let wide_app_id = to_wide(app_id);
        // SAFETY: `wide_app_id` is a valid, NUL-terminated UTF-16 buffer that
        // outlives this call.
        let hresult =
            unsafe { SetCurrentProcessExplicitAppUserModelID(wide_app_id.as_ptr()) };
        if hresult < 0 {
            eprintln!(
                "buzz-desktop: SetCurrentProcessExplicitAppUserModelID failed: 0x{hresult:08X}"
            );
        }
    }

    fn write_aumid_registry_entry(app: &tauri::AppHandle, app_id: &str) -> Result<(), String> {
        use windows_sys::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE,
            REG_OPTION_NON_VOLATILE, REG_SZ,
        };

        let display_name = app
            .config()
            .product_name
            .clone()
            .unwrap_or_else(|| "Buzz".to_string());
        // The exe's own icon resource doubles as the AUMID icon; Windows
        // accepts a path to an executable here just as it does a .ico file.
        let icon_path = std::env::current_exe()
            .map_err(|error| format!("could not resolve current exe path: {error}"))?
            .to_string_lossy()
            .into_owned();

        let subkey = to_wide(&format!("Software\\Classes\\AppUserModelId\\{app_id}"));
        let display_name_value = to_wide("DisplayName");
        let display_name_data = to_wide(&display_name);
        let icon_uri_value = to_wide("IconUri");
        let icon_uri_data = to_wide(&icon_path);

        // SAFETY: every buffer passed below is a NUL-terminated UTF-16
        // string that outlives the corresponding call, `hkey` is only used
        // after a successful `RegCreateKeyExW`, and it is closed exactly
        // once before returning.
        unsafe {
            let mut hkey: HKEY = std::ptr::null_mut();
            let create_status = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                std::ptr::null(),
                &mut hkey,
                std::ptr::null_mut(),
            );
            if create_status != 0 {
                return Err(format!("RegCreateKeyExW failed with status {create_status}"));
            }

            let display_name_bytes = std::slice::from_raw_parts(
                display_name_data.as_ptr().cast::<u8>(),
                display_name_data.len() * 2,
            );
            let display_status = RegSetValueExW(
                hkey,
                display_name_value.as_ptr(),
                0,
                REG_SZ,
                display_name_bytes.as_ptr(),
                display_name_bytes.len() as u32,
            );

            let icon_uri_bytes = std::slice::from_raw_parts(
                icon_uri_data.as_ptr().cast::<u8>(),
                icon_uri_data.len() * 2,
            );
            let icon_status = RegSetValueExW(
                hkey,
                icon_uri_value.as_ptr(),
                0,
                REG_SZ,
                icon_uri_bytes.as_ptr(),
                icon_uri_bytes.len() as u32,
            );

            RegCloseKey(hkey);

            if display_status != 0 {
                return Err(format!("RegSetValueExW(DisplayName) failed with status {display_status}"));
            }
            if icon_status != 0 {
                return Err(format!("RegSetValueExW(IconUri) failed with status {icon_status}"));
            }
        }

        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::NATIVE_NOTIFICATION_ACTIVATED_EVENT;
    use tauri::Emitter;

    pub fn show(
        app: tauri::AppHandle,
        title: String,
        body: Option<String>,
        target: Option<serde_json::Value>,
    ) {
        // notify_rust's `show()` blocks on D-Bus and the returned handle must
        // outlive the notification, so this runs on its own thread rather than
        // the async runtime.
        std::thread::spawn(move || {
            let mut builder = notify_rust::Notification::new();
            builder.summary(&title);
            if let Some(body) = body.as_deref() {
                builder.body(body);
            }
            if let Some(name) = app.config().product_name.clone() {
                builder.appname(&name);
            }
            // Tie the notification to the installed desktop entry so GNOME shows
            // the app's name and icon and groups our notifications together.
            builder.hint(notify_rust::Hint::DesktopEntry(
                app.config().identifier.clone(),
            ));
            builder.auto_icon();
            // Match the silent posting used on other platforms; the app does its
            // own unread cues and a per-message sound would be noisy.
            builder.hint(notify_rust::Hint::SuppressSound(true));
            // Declaring a default action makes the whole notification clickable.
            builder.action("default", "Open");

            let handle = match builder.show() {
                Ok(handle) => handle,
                Err(error) => {
                    eprintln!("buzz-desktop: failed to post native notification: {error}");
                    return;
                }
            };

            // Block until the notification is actioned or closed. Holding the
            // handle keeps its D-Bus connection alive, which is what stops
            // GNOME 46+ from dismissing the notification immediately. The wait
            // also returns when the notification expires or is dismissed, so
            // the thread does not leak.
            handle.wait_for_action(|action| {
                if action != "default" {
                    return;
                }

                // The frontend focuses the window on activation (the same path
                // every other platform uses), so we only forward the target.
                let _ = app.emit(NATIVE_NOTIFICATION_ACTIVATED_EVENT, target);
            });
        });
    }
}
