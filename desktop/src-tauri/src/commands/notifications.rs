//! Native desktop-notification workarounds.
//!
//! On Linux, `tauri-plugin-notification` posts a notification by calling
//! `notify_rust`'s `show()` and immediately dropping the returned
//! `NotificationHandle`. That handle owns the D-Bus connection used to post the
//! notification, and on GNOME 46+ (Ubuntu 24.04+, Fedora 41+) tearing that
//! connection down dismisses the notification the instant it appears. See
//! tauri-apps/plugins-workspace#2566 and hoodie/notify-rust#218.
//!
//! We side-step the plugin on Linux by holding the connection open in a
//! dedicated thread until the notification closes. On macOS, the plugin does
//! not deliver desktop click actions or preserve their routing metadata, so
//! targeted notifications use the native callback and forward their target to
//! the frontend.

#[cfg(any(target_os = "linux", target_os = "macos"))]
const ACTIVATE_EVENT: &str = "native-notification-activated";

/// Show a desktop notification through the platform's native workaround.
///
/// Linux uses the connection-preserving path described above. macOS uses a
/// native click callback so the frontend receives the notification target.
#[tauri::command]
pub fn show_native_notification(
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
        macos::show(app, title, body, target);
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (&app, &title, &body, &target);
        Err("show_native_notification is only supported on Linux and macOS".to_string())
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::ACTIVATE_EVENT;
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
                let _ = app.emit(ACTIVATE_EVENT, target);
            });
        });
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::ACTIVATE_EVENT;
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Once,
        },
        time::Duration,
    };
    use tauri::Emitter;

    const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60 * 60);
    const MAX_DEV_RESPONSE_THREADS: usize = 8;
    static ACTIVE_DEV_RESPONSE_THREADS: AtomicUsize = AtomicUsize::new(0);
    static CONFIGURE_APPLICATION: Once = Once::new();

    struct DevResponseThreadGuard;

    impl Drop for DevResponseThreadGuard {
        fn drop(&mut self) {
            ACTIVE_DEV_RESPONSE_THREADS.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn configure_application(app: &tauri::AppHandle) {
        CONFIGURE_APPLICATION.call_once(|| {
            let application = if tauri::is_dev() {
                "com.apple.Terminal".to_string()
            } else {
                app.config().identifier.clone()
            };
            match mac_notification_sys::set_application(&application) {
                Ok(())
                | Err(mac_notification_sys::error::Error::Application(
                    mac_notification_sys::error::ApplicationError::AlreadySet(_),
                )) => {}
                Err(error) => {
                    eprintln!(
                        "buzz-desktop: failed to configure macOS notification application: {error}"
                    );
                }
            }
        });
    }

    pub fn show(
        app: tauri::AppHandle,
        title: String,
        body: Option<String>,
        target: Option<serde_json::Value>,
    ) {
        configure_application(&app);

        if tauri::is_dev() {
            show_in_unbundled_dev(app, title, body, target);
        } else {
            show_with_async_callback(app, title, body, target);
        }
    }

    fn show_with_async_callback(
        app: tauri::AppHandle,
        title: String,
        body: Option<String>,
        target: Option<serde_json::Value>,
    ) {
        let notification = mac_usernotifications::Notification::new()
            .title(title)
            .message(body.unwrap_or_default())
            .timeout(RESPONSE_TIMEOUT);

        // The modern callback API keeps one process-wide worker thread and
        // awaits each response without blocking an OS thread. The timeout also
        // bounds callback state when a notification is never actioned.
        tauri::async_runtime::spawn(async move {
            let handle = match notification.send().await {
                Ok(handle) => handle,
                Err(error) => {
                    eprintln!("buzz-desktop: failed to post native notification: {error}");
                    return;
                }
            };

            match handle.response().await {
                Ok(response) if response.is_default_action() => {
                    let _ = app.emit(ACTIVATE_EVENT, target);
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!(
                        "buzz-desktop: failed to receive native notification response: {error}"
                    );
                }
            }
        });
    }

    fn show_in_unbundled_dev(
        app: tauri::AppHandle,
        title: String,
        body: Option<String>,
        target: Option<serde_json::Value>,
    ) {
        let reserved = ACTIVE_DEV_RESPONSE_THREADS
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |active| {
                (active < MAX_DEV_RESPONSE_THREADS).then_some(active + 1)
            })
            .is_ok();

        if !reserved {
            // UNUserNotificationCenter terminates unbundled binaries, so local
            // `tauri dev` uses the legacy API. Keep its blocking listeners
            // bounded; overflow notifications are still posted fire-and-forget.
            tauri::async_runtime::spawn_blocking(move || {
                let mut notification = mac_notification_sys::Notification::new();
                notification.title(&title).asynchronous(true);
                if let Some(body) = body.as_deref() {
                    notification.message(body);
                }
                if let Err(error) = notification.send() {
                    eprintln!("buzz-desktop: failed to post native notification: {error}");
                }
            });
            return;
        }

        std::thread::spawn(move || {
            let _guard = DevResponseThreadGuard;
            let mut notification = mac_notification_sys::Notification::new();
            notification.title(&title).wait_for_click(true);
            if let Some(body) = body.as_deref() {
                notification.message(body);
            }

            match notification.send() {
                Ok(mac_notification_sys::NotificationResponse::Click) => {
                    let _ = app.emit(ACTIVATE_EVENT, target);
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!("buzz-desktop: failed to post native notification: {error}");
                }
            }
        });
    }
}
