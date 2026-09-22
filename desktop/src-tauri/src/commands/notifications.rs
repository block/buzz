//! Native desktop-notification helpers.
//!
//! `tauri-plugin-notification` posts a notification by calling `notify_rust`'s
//! `show()` and then immediately dropping the returned `NotificationHandle`.
//! That handle owns the D-Bus connection used to post the notification, and on
//! GNOME 46+ (Ubuntu 24.04+, Fedora 41+) tearing that connection down dismisses
//! the notification the instant it appears — so notifications never show.
//! See tauri-apps/plugins-workspace#2566 and hoodie/notify-rust#218.
//!
//! We side-step the plugin on Linux and Windows by posting the notification
//! from a dedicated thread that holds the native activation handle until the
//! notification closes. The same wait surfaces a click and forwards it to the
//! frontend so it can focus the window and route to the notification target.

pub(crate) const NATIVE_NOTIFICATION_ACTIVATED_EVENT: &str = "native-notification-activated";

/// Show a desktop notification natively.
///
/// Linux uses the connection-preserving D-Bus path described above. Windows
/// uses a retained WinRT toast handle and reports posting failures instead of
/// losing them in the plugin's detached task. macOS uses one
/// application-lifetime `UNUserNotificationCenterDelegate`; it does not
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
        windows::show(app, title, body, target).await
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (&app, &title, &body, &target);
        Err("show_native_notification is unsupported on this platform".to_string())
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::NATIVE_NOTIFICATION_ACTIVATED_EVENT;
    use notify_rust::{Notification, NotificationResponse};
    use std::path::Path;
    use tauri::Emitter;

    pub async fn show(
        app: tauri::AppHandle,
        title: String,
        body: Option<String>,
        target: Option<serde_json::Value>,
    ) -> Result<(), String> {
        let (posted_tx, posted_rx) = tokio::sync::oneshot::channel();

        std::thread::spawn(move || {
            let mut builder = Notification::new();
            builder.summary(&title);
            if let Some(body) = body.as_deref() {
                builder.body(body);
            }
            let current_exe = tauri::utils::platform::current_exe().ok();
            if should_use_registered_app_id(current_exe.as_deref()) {
                builder.app_id(&app.config().identifier);
            }

            let handle = match builder.show() {
                Ok(handle) => {
                    let _ = posted_tx.send(Ok(()));
                    handle
                }
                Err(error) => {
                    let message = format!("failed to post native Windows notification: {error}");
                    eprintln!("buzz-desktop: {message}");
                    let _ = posted_tx.send(Err(message));
                    return;
                }
            };

            let _ = handle.wait_for_response(|response: &NotificationResponse| {
                if response.is_default_action() {
                    if let Some(target) = target.as_ref() {
                        let _ = app.emit(NATIVE_NOTIFICATION_ACTIVATED_EVENT, target);
                    }
                }
            });
        });

        posted_rx
            .await
            .map_err(|_| "native Windows notification worker stopped before posting".to_string())?
    }

    fn should_use_registered_app_id(executable: Option<&Path>) -> bool {
        let Some(directory) = executable.and_then(Path::parent) else {
            return false;
        };
        let directory = directory.to_string_lossy().replace('/', "\\");
        !directory.ends_with("\\target\\debug") && !directory.ends_with("\\target\\release")
    }

    #[cfg(test)]
    mod tests {
        use super::should_use_registered_app_id;
        use std::path::Path;

        #[test]
        fn development_executables_use_the_unregistered_fallback() {
            assert!(!should_use_registered_app_id(Some(Path::new(
                r"C:\repo\desktop\src-tauri\target\debug\buzz.exe",
            ))));
            assert!(!should_use_registered_app_id(Some(Path::new(
                r"C:\repo\desktop\src-tauri\target\release\buzz.exe",
            ))));
        }

        #[test]
        fn installed_executables_use_the_bundled_app_id() {
            assert!(should_use_registered_app_id(Some(Path::new(
                r"C:\Program Files\Buzz\buzz.exe",
            ))));
            assert!(!should_use_registered_app_id(None));
        }
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
