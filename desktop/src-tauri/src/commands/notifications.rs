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

#![forbid(unsafe_code)]

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
        windows::show(app, title, body, target).await
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (&app, &title, &body, &target);
        Err("show_native_notification is not supported on this platform".to_string())
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn ensure_startup_registration(app: &tauri::AppHandle) -> Result<(), String> {
    windows::ensure_startup_registration(app)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn ensure_startup_registration(_app: &tauri::AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "windows")]
#[tauri::command]
pub async fn windows_notification_permission_state(
    app: tauri::AppHandle,
) -> Result<String, String> {
    windows::permission_state(app).await.map(str::to_string)
}

#[cfg(target_os = "windows")]
#[tauri::command]
pub fn take_pending_windows_activations() -> Result<Vec<serde_json::Value>, String> {
    windows::take_pending_windows_activations()
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

// ── Windows ────────────────────────────────────────────────────────────────
//
// Uses `tauri-winrt-notification` to post Windows toast notifications. This
// registers the app with Windows Settings > System > Notifications (so the
// user can control per-app notification preferences) and surfaces click
// actions through the WinRT `Activated` handler, which we forward to the
// frontend via the same `native-notification-activated` event that Linux uses.

#[cfg(target_os = "windows")]
mod windows {
    use super::NATIVE_NOTIFICATION_ACTIVATED_EVENT;
    use std::collections::VecDeque;
    use std::sync::{Mutex, OnceLock};
    use tauri::Emitter;
    use tauri_winrt_notification::{Duration, Toast};
    use windows::{
        core::HSTRING,
        UI::Notifications::{NotificationSetting, ToastNotificationManager},
    };
    use winsafe::{co, prelude::*, IPersistFile, IPropertyStore, IShellLink, RegistryValue, HKEY};

    const MAX_PENDING_ACTIVATIONS: usize = 64;
    static STARTUP_REGISTRATION: OnceLock<Result<(), String>> = OnceLock::new();
    static PENDING_ACTIVATIONS: OnceLock<Mutex<VecDeque<serde_json::Value>>> = OnceLock::new();

    pub fn ensure_startup_registration(app: &tauri::AppHandle) -> Result<(), String> {
        STARTUP_REGISTRATION
            .get_or_init(|| {
                let app_id = app.config().identifier.clone();
                set_process_aumid(&app_id)?;
                write_aumid_registry_entry(app, &app_id)?;
                write_notification_settings_entry(&app_id)?;
                ensure_start_menu_shortcut(app, &app_id)
            })
            .clone()
    }

    fn set_process_aumid(app_id: &str) -> Result<(), String> {
        winsafe::SetCurrentProcessExplicitAppUserModelID(app_id)
            .map_err(|error| format!("failed to set Windows process AUMID: {error}"))
    }

    fn write_notification_settings_entry(app_id: &str) -> Result<(), String> {
        initialize_notification_settings(&format!(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Notifications\\Settings\\{app_id}"
        ))
    }

    fn initialize_notification_settings(subkey: &str) -> Result<(), String> {
        let (key, disposition) = HKEY::CURRENT_USER
            .RegCreateKeyEx(
                subkey,
                None,
                co::REG_OPTION::NON_VOLATILE,
                co::KEY::WRITE,
                None,
            )
            .map_err(|error| format!("could not open Windows notification settings: {error}"))?;
        if disposition == co::REG_DISPOSITION::OPENED_EXISTING_KEY {
            return Ok(());
        }
        for name in ["ShowInActionCenter", "Enabled"] {
            key.RegSetValueEx(Some(name), RegistryValue::Dword(1))
                .map_err(|error| {
                    format!("could not initialize notification setting {name}: {error}")
                })?;
        }
        Ok(())
    }

    fn write_aumid_registry_entry(app: &tauri::AppHandle, app_id: &str) -> Result<(), String> {
        let display_name = app
            .config()
            .product_name
            .clone()
            .unwrap_or_else(|| "Buzz".to_string());
        let icon_path = std::env::current_exe()
            .map_err(|error| format!("could not resolve current executable: {error}"))?;
        write_aumid_metadata(
            &format!("Software\\Classes\\AppUserModelId\\{app_id}"),
            &display_name,
            &icon_path.to_string_lossy(),
        )
    }

    fn write_aumid_metadata(
        subkey: &str,
        display_name: &str,
        icon_path: &str,
    ) -> Result<(), String> {
        let (key, _) = HKEY::CURRENT_USER
            .RegCreateKeyEx(
                subkey,
                None,
                co::REG_OPTION::NON_VOLATILE,
                co::KEY::WRITE,
                None,
            )
            .map_err(|error| format!("could not open Windows AUMID metadata: {error}"))?;
        for (name, value) in [("DisplayName", display_name), ("IconUri", icon_path)] {
            key.RegSetValueEx(Some(name), RegistryValue::Sz(value.to_string()))
                .map_err(|error| format!("could not write AUMID {name}: {error}"))?;
        }
        Ok(())
    }

    fn ensure_start_menu_shortcut(app: &tauri::AppHandle, app_id: &str) -> Result<(), String> {
        let product_name = app
            .config()
            .product_name
            .clone()
            .unwrap_or_else(|| "Buzz".to_string());
        let executable = std::env::current_exe()
            .map_err(|error| format!("could not resolve current executable: {error}"))?;
        let programs =
            winsafe::SHGetKnownFolderPath(&co::KNOWNFOLDERID::Programs, co::KF::CREATE, None)
                .map_err(|error| format!("could not find Start Menu programs: {error}"))?;
        ensure_shortcut_in(
            std::path::Path::new(&programs),
            &product_name,
            &executable,
            app_id,
        )
    }

    fn ensure_shortcut_in(
        programs: &std::path::Path,
        product_name: &str,
        executable: &std::path::Path,
        app_id: &str,
    ) -> Result<(), String> {
        let _apartment = winsafe::CoInitializeEx(co::COINIT::APARTMENTTHREADED)
            .map_err(|error| format!("could not initialize shortcut COM apartment: {error}"))?;
        let subfolder = programs.join(product_name);
        let nested_path = subfolder.join(format!("{product_name}.lnk"));
        let flat_path = programs.join(format!("{product_name}.lnk"));
        let shortcut_path = if nested_path.exists() {
            nested_path
        } else if flat_path.exists() {
            flat_path
        } else {
            std::fs::create_dir_all(&subfolder)
                .map_err(|error| format!("could not create Start Menu folder: {error}"))?;
            nested_path
        };
        let link = winsafe::CoCreateInstance::<IShellLink>(
            &co::CLSID::ShellLink,
            None::<&winsafe::IUnknown>,
            co::CLSCTX::INPROC_SERVER,
        )
        .map_err(|error| format!("could not create ShellLink: {error}"))?;
        let persist: IPersistFile = link
            .QueryInterface()
            .map_err(|error| format!("could not query shortcut persistence: {error}"))?;
        if shortcut_path.exists() {
            persist
                .Load(&shortcut_path.to_string_lossy(), co::STGM::READWRITE)
                .map_err(|error| format!("could not load Start Menu shortcut: {error}"))?;
        } else {
            link.SetPath(&executable.to_string_lossy())
                .map_err(|error| format!("could not set shortcut path: {error}"))?;
            link.SetIconLocation(&executable.to_string_lossy(), 0)
                .map_err(|error| format!("could not set shortcut icon: {error}"))?;
        }
        let properties: IPropertyStore = link
            .QueryInterface()
            .map_err(|error| format!("could not query shortcut property store: {error}"))?;
        properties
            .SetValue(
                &co::PKEY::AppUserModel_ID,
                &winsafe::PropVariant::from_str(app_id),
            )
            .map_err(|error| format!("could not set shortcut AUMID: {error}"))?;
        properties
            .Commit()
            .map_err(|error| format!("could not commit shortcut properties: {error}"))?;
        persist
            .Save(Some(&shortcut_path.to_string_lossy()), true)
            .map_err(|error| format!("could not save Start Menu shortcut: {error}"))
    }

    fn queue_activation(target: Option<serde_json::Value>) {
        let Some(target) = target else {
            return;
        };
        let queue = PENDING_ACTIVATIONS.get_or_init(Default::default);
        let Ok(mut queue) = queue.lock() else {
            eprintln!("buzz-desktop: Windows activation queue is unavailable");
            return;
        };
        if queue.len() == MAX_PENDING_ACTIVATIONS {
            queue.pop_front();
        }
        queue.push_back(target);
    }

    pub fn take_pending_windows_activations() -> Result<Vec<serde_json::Value>, String> {
        let queue = PENDING_ACTIVATIONS.get_or_init(Default::default);
        let mut queue = queue
            .lock()
            .map_err(|_| "Windows activation queue is unavailable".to_string())?;
        Ok(queue.drain(..).collect())
    }

    fn permission_state_label(setting: NotificationSetting) -> &'static str {
        if setting == NotificationSetting::Enabled {
            "granted"
        } else {
            "denied"
        }
    }

    pub async fn permission_state(app: tauri::AppHandle) -> Result<&'static str, String> {
        let app_id = app.config().identifier.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();

        app.run_on_main_thread(move || {
            let result =
                ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))
                    .and_then(|notifier| notifier.Setting())
                    .map(permission_state_label)
                    .map_err(|error| {
                        format!("failed to query Windows notification setting: {error}")
                    });
            let _ = sender.send(result);
        })
        .map_err(|error| format!("failed to schedule Windows notification query: {error}"))?;

        receiver
            .await
            .map_err(|_| "Windows notification query ended before completing".to_string())?
    }

    pub async fn show(
        app: tauri::AppHandle,
        title: String,
        body: Option<String>,
        target: Option<serde_json::Value>,
    ) -> Result<(), String> {
        // The Tauri identifier (e.g. "xyz.block.buzz.app") is the
        // AppUserModelID that Windows uses to group notifications and
        // surface the app in Settings > Notifications.
        let app_id = app.config().identifier.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let notification_app = app.clone();

        // Tauri's main thread owns an initialized Windows apartment. Construct
        // and post WinRT notifications there, then report the real result.
        app.run_on_main_thread(move || {
            let activation_app = notification_app.clone();
            let result = Toast::new(&app_id)
                .title(&title)
                .text1(body.as_deref().unwrap_or(""))
                .sound(None)
                .duration(Duration::Short)
                .on_activated(move |_action| {
                    // _action is None for the default (body) click and
                    // Some(arg) for button clicks. We only use the default
                    // click, matching the Linux behaviour.
                    queue_activation(target.clone());
                    let _ = activation_app.emit(NATIVE_NOTIFICATION_ACTIVATED_EVENT, &target);
                    Ok(())
                })
                .show()
                .map_err(|error| format!("failed to post Windows notification: {error}"));
            let _ = sender.send(result);
        })
        .map_err(|error| format!("failed to schedule Windows notification: {error}"))?;

        receiver
            .await
            .map_err(|_| "Windows notification task ended before posting".to_string())?
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        struct TestRegistryKey(String);

        impl TestRegistryKey {
            fn new() -> Self {
                Self(format!(
                    "Software\\BuzzNotificationTest-{}",
                    uuid::Uuid::new_v4()
                ))
            }

            fn open(&self) -> winsafe::guard::RegCloseKeyGuard {
                HKEY::CURRENT_USER
                    .RegOpenKeyEx(
                        Some(&self.0),
                        co::REG_OPTION::default(),
                        co::KEY::READ | co::KEY::WRITE,
                    )
                    .expect("open isolated test key")
            }
        }

        impl Drop for TestRegistryKey {
            fn drop(&mut self) {
                HKEY::CURRENT_USER
                    .RegDeleteTree(Some(&self.0))
                    .expect("remove isolated test key");
            }
        }

        #[test]
        fn notification_settings_preserve_existing_opt_out() {
            let test_key = TestRegistryKey::new();
            initialize_notification_settings(&test_key.0).expect("initialize settings");
            let key = test_key.open();
            for name in ["Enabled", "ShowInActionCenter"] {
                assert!(matches!(
                    key.RegQueryValueEx(Some(name)),
                    Ok(RegistryValue::Dword(1))
                ));
                key.RegSetValueEx(Some(name), RegistryValue::Dword(0))
                    .expect("disable setting");
            }
            initialize_notification_settings(&test_key.0).expect("repair settings");
            for name in ["Enabled", "ShowInActionCenter"] {
                assert!(matches!(
                    key.RegQueryValueEx(Some(name)),
                    Ok(RegistryValue::Dword(0))
                ));
            }
        }

        #[test]
        fn aumid_metadata_is_repaired() {
            let test_key = TestRegistryKey::new();
            write_aumid_metadata(&test_key.0, "Old Buzz", "old.exe").expect("initial metadata");
            write_aumid_metadata(&test_key.0, "Buzz", "new.exe").expect("repair metadata");
            let key = test_key.open();
            assert!(
                matches!(key.RegQueryValueEx(Some("DisplayName")), Ok(RegistryValue::Sz(value)) if value == "Buzz")
            );
            assert!(
                matches!(key.RegQueryValueEx(Some("IconUri")), Ok(RegistryValue::Sz(value)) if value == "new.exe")
            );
        }

        #[test]
        fn shortcut_aumid_is_repaired_without_changing_arguments() {
            let directory = tempfile::tempdir().expect("temporary programs folder");
            let executable = std::env::current_exe().expect("test executable");
            ensure_shortcut_in(directory.path(), "Buzz", &executable, "buzz.test.old")
                .expect("create shortcut");
            let path = directory.path().join("Buzz").join("Buzz.lnk");
            let _apartment =
                winsafe::CoInitializeEx(co::COINIT::APARTMENTTHREADED).expect("COM apartment");
            let load = || {
                let link = winsafe::CoCreateInstance::<IShellLink>(
                    &co::CLSID::ShellLink,
                    None::<&winsafe::IUnknown>,
                    co::CLSCTX::INPROC_SERVER,
                )
                .expect("ShellLink");
                link.QueryInterface::<IPersistFile>()
                    .expect("persist")
                    .Load(&path.to_string_lossy(), co::STGM::READWRITE)
                    .expect("load shortcut");
                link
            };
            {
                let link = load();
                link.SetArguments("--preserve-this").expect("arguments");
                link.QueryInterface::<IPersistFile>()
                    .expect("persist")
                    .Save(Some(&path.to_string_lossy()), true)
                    .expect("save arguments");
            }
            ensure_shortcut_in(directory.path(), "Buzz", &executable, "buzz.test.new")
                .expect("repair shortcut");
            let link = load();
            assert_eq!(
                link.GetArguments().expect("read arguments"),
                "--preserve-this"
            );
            let value = link
                .QueryInterface::<IPropertyStore>()
                .expect("properties")
                .GetValue(&co::PKEY::AppUserModel_ID)
                .expect("read AUMID");
            assert!(matches!(value, winsafe::PropVariant::Bstr(value) if value == "buzz.test.new"));
        }

        #[test]
        fn shortcut_failure_is_propagated() {
            let directory = tempfile::tempdir().expect("temporary programs folder");
            std::fs::write(directory.path().join("Buzz"), "blocks directory creation")
                .expect("block shortcut folder");
            assert!(ensure_shortcut_in(
                directory.path(),
                "Buzz",
                std::path::Path::new("buzz.exe"),
                "buzz.test"
            )
            .is_err());
        }

        #[test]
        fn only_enabled_notification_setting_is_granted() {
            assert_eq!(
                permission_state_label(NotificationSetting::Enabled),
                "granted"
            );
            for setting in [
                NotificationSetting::DisabledForApplication,
                NotificationSetting::DisabledForUser,
                NotificationSetting::DisabledByGroupPolicy,
                NotificationSetting::DisabledByManifest,
            ] {
                assert_eq!(permission_state_label(setting), "denied");
            }
        }

        #[test]
        fn activation_queue_is_bounded() {
            let _ = take_pending_windows_activations();
            for index in 0..=MAX_PENDING_ACTIVATIONS {
                queue_activation(Some(serde_json::json!({ "index": index })));
            }

            let activations = take_pending_windows_activations().expect("activation queue");
            assert_eq!(activations.len(), MAX_PENDING_ACTIVATIONS);
            assert_eq!(activations[0]["index"], 1);
        }
    }
}
