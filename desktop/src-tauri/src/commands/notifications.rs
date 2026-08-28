//! Native desktop-notification helpers.
//!
//! `tauri-plugin-notification` posts a notification by calling `notify_rust`'s
//! `show()` and then immediately dropping the returned `NotificationHandle`.
//! That handle owns the D-Bus connection used to post the notification, and on
//! GNOME 46+ (Ubuntu 24.04+, Fedora 41+) tearing that connection down dismisses
//! the notification the instant it appears — so notifications never show.
//! See tauri-apps/plugins-workspace#2566 and hoodie/notify-rust#218.
//!
//! We side-step the plugin on Linux and Windows by posting from a dedicated
//! thread that holds the handle until the notification is closed. On Linux
//! that keeps the D-Bus connection alive; on both platforms the wait surfaces
//! the default click action, which we forward to the frontend so it can focus
//! the window and route to the notification target.

pub(crate) const NATIVE_NOTIFICATION_ACTIVATED_EVENT: &str = "native-notification-activated";

/// Show a desktop notification natively.
///
/// Linux and Windows use the connection-preserving `notify_rust` path
/// described above. macOS uses one application-lifetime
/// `UNUserNotificationCenterDelegate`; it does not allocate a listener or
/// waiter for each notification.
#[tauri::command]
pub async fn show_native_notification(
    app: tauri::AppHandle,
    title: String,
    body: Option<String>,
    target: Option<serde_json::Value>,
) -> Result<(), String> {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        tauri::async_runtime::spawn_blocking(move || native::show(app, title, body, target))
            .await
            .map_err(|error| format!("native notification task failed: {error}"))?
    }

    #[cfg(target_os = "macos")]
    {
        let _ = app;
        crate::macos_notifications::show(title, body, target).await
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (&app, &title, &body, &target);
        Err("show_native_notification is only supported on Linux, macOS, and Windows".to_string())
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
mod native {
    use super::NATIVE_NOTIFICATION_ACTIVATED_EVENT;
    use tauri::Emitter;

    pub fn show(
        app: tauri::AppHandle,
        title: String,
        body: Option<String>,
        target: Option<serde_json::Value>,
    ) -> Result<(), String> {
        // notify_rust's `show()` blocks on D-Bus. Report that result to the
        // command, then keep the handle alive on a worker until the toast is
        // clicked, dismissed, or expires — dropping it immediately is what
        // makes GNOME 46+ hide the notification the instant it appears.
        let mut builder = notify_rust::Notification::new();
        builder.summary(&title);
        if let Some(body) = body.as_deref() {
            builder.body(body);
        }
        if let Some(name) = app.config().product_name.clone() {
            builder.appname(&name);
        }
        #[cfg(target_os = "linux")]
        {
            // Tie the notification to the installed desktop entry so GNOME shows
            // the app's name and icon and groups our notifications together.
            builder.hint(notify_rust::Hint::DesktopEntry(
                app.config().identifier.clone(),
            ));
            builder.auto_icon();
            // The app plays its own alert sound; a second OS chirp is noisy.
            builder.hint(notify_rust::Hint::SuppressSound(true));
        }
        #[cfg(target_os = "windows")]
        {
            // Packaged Buzz registers this AUMID. Without it, WinRT attributes
            // the toast to PowerShell and unpackaged `tauri dev` may drop it.
            builder.app_id(&app.config().identifier);
        }
        // Declaring a default action makes the whole notification clickable.
        builder.action("default", "Open");

        let handle = builder.show().map_err(|error| {
            eprintln!("buzz-desktop: failed to post native notification: {error}");
            error.to_string()
        })?;

        // Hold the handle on a background thread so GNOME 46+ does not
        // dismiss the toast when this command returns, and so Windows can
        // deliver the click. The wait ends when the notification is clicked,
        // dismissed, or expires.
        std::thread::spawn(move || {
            handle.wait_for_action(|action| {
                if action != "default" {
                    return;
                }

                // The frontend focuses the window on activation (the same path
                // every other platform uses), so we only forward the target.
                let _ = app.emit(NATIVE_NOTIFICATION_ACTIVATED_EVENT, target);
            });
        });
        Ok(())
    }
}
