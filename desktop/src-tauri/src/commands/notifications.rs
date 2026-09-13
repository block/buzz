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
pub(crate) fn ensure_startup_registration(app: &tauri::AppHandle) {
    windows::ensure_startup_registration(app);
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn ensure_startup_registration(_app: &tauri::AppHandle) {}

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
    use std::sync::{Mutex, Once, OnceLock};
    use tauri::Emitter;
    use tauri_winrt_notification::{Duration, Toast};
    use windows::{
        core::HSTRING,
        UI::Notifications::{NotificationSetting, ToastNotificationManager},
    };

    const MAX_PENDING_ACTIVATIONS: usize = 64;
    static STARTUP_REGISTRATION: Once = Once::new();
    static PENDING_ACTIVATIONS: OnceLock<Mutex<VecDeque<serde_json::Value>>> = OnceLock::new();

    pub fn ensure_startup_registration(app: &tauri::AppHandle) {
        STARTUP_REGISTRATION.call_once(|| {
            let app = app.clone();
            let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
            std::thread::spawn(move || {
                let app_id = app.config().identifier.clone();
                set_process_aumid(&app_id);
                if let Err(error) = write_aumid_registry_entry(&app, &app_id) {
                    eprintln!("buzz-desktop: failed to register Windows AUMID: {error}");
                }
                if let Err(error) = write_notification_settings_entry(&app_id) {
                    eprintln!(
                        "buzz-desktop: failed to register Windows notification settings: {error}"
                    );
                }
                if let Err(error) = ensure_start_menu_shortcut(&app, &app_id) {
                    eprintln!("buzz-desktop: failed to repair Windows shortcut AUMID: {error}");
                }
                let _ = ready_sender.send(());
            });
            let _ = ready_receiver.recv();
        });
    }

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn set_process_aumid(app_id: &str) {
        use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

        let app_id = to_wide(app_id);
        let result = unsafe { SetCurrentProcessExplicitAppUserModelID(app_id.as_ptr()) };
        if result < 0 {
            eprintln!("buzz-desktop: failed to set Windows process AUMID: 0x{result:08X}");
        }
    }

    fn write_notification_settings_entry(app_id: &str) -> Result<(), String> {
        use windows_sys::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
            KEY_READ, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE,
        };

        let subkey = to_wide(&format!(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Notifications\\Settings\\{app_id}"
        ));
        unsafe {
            let mut existing: HKEY = std::ptr::null_mut();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                KEY_READ,
                &mut existing,
            ) == 0
            {
                RegCloseKey(existing);
                return Ok(());
            }

            let mut key: HKEY = std::ptr::null_mut();
            let status = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            );
            if status != 0 {
                return Err(format!("RegCreateKeyExW failed with status {status}"));
            }

            let show_name = to_wide("ShowInActionCenter");
            let enabled_name = to_wide("Enabled");
            let value: u32 = 1;
            let show_status = RegSetValueExW(
                key,
                show_name.as_ptr(),
                0,
                REG_DWORD,
                (&value as *const u32).cast(),
                std::mem::size_of::<u32>() as u32,
            );
            let enabled_status = RegSetValueExW(
                key,
                enabled_name.as_ptr(),
                0,
                REG_DWORD,
                (&value as *const u32).cast(),
                std::mem::size_of::<u32>() as u32,
            );
            RegCloseKey(key);
            if show_status != 0 {
                return Err(format!(
                    "RegSetValueExW(ShowInActionCenter) failed: {show_status}"
                ));
            }
            if enabled_status != 0 {
                return Err(format!("RegSetValueExW(Enabled) failed: {enabled_status}"));
            }
        }
        Ok(())
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
        let icon_path = std::env::current_exe()
            .map_err(|error| format!("could not resolve current executable: {error}"))?
            .to_string_lossy()
            .into_owned();
        let subkey = to_wide(&format!("Software\\Classes\\AppUserModelId\\{app_id}"));
        let display_name_key = to_wide("DisplayName");
        let display_name_value = to_wide(&display_name);
        let icon_key = to_wide("IconUri");
        let icon_value = to_wide(&icon_path);

        unsafe {
            let mut key: HKEY = std::ptr::null_mut();
            let status = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            );
            if status != 0 {
                return Err(format!("RegCreateKeyExW failed with status {status}"));
            }
            let display_status = RegSetValueExW(
                key,
                display_name_key.as_ptr(),
                0,
                REG_SZ,
                display_name_value.as_ptr().cast(),
                (display_name_value.len() * 2) as u32,
            );
            let icon_status = RegSetValueExW(
                key,
                icon_key.as_ptr(),
                0,
                REG_SZ,
                icon_value.as_ptr().cast(),
                (icon_value.len() * 2) as u32,
            );
            RegCloseKey(key);
            if display_status != 0 {
                return Err(format!(
                    "RegSetValueExW(DisplayName) failed: {display_status}"
                ));
            }
            if icon_status != 0 {
                return Err(format!("RegSetValueExW(IconUri) failed: {icon_status}"));
            }
        }
        Ok(())
    }

    fn ensure_start_menu_shortcut(app: &tauri::AppHandle, app_id: &str) -> Result<(), String> {
        use windows::core::{Interface, PCWSTR};
        use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
        use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READWRITE,
        };
        use windows::Win32::UI::Shell::{
            FOLDERID_Programs, IShellLinkW, PropertiesSystem::IPropertyStore, SHGetKnownFolderPath,
            ShellLink, KF_FLAG_CREATE,
        };

        let product_name = app
            .config()
            .product_name
            .clone()
            .unwrap_or_else(|| "Buzz".to_string());
        let executable = std::env::current_exe()
            .map_err(|error| format!("could not resolve current executable: {error}"))?;
        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if initialized.is_err() {
            return Err(format!("CoInitializeEx failed: {initialized:?}"));
        }

        let result = (|| -> Result<(), String> {
            let programs_path_ptr = unsafe {
                SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_CREATE, None)
                    .map_err(|error| format!("SHGetKnownFolderPath failed: {error}"))?
            };
            let programs_path_result = unsafe { programs_path_ptr.to_string() };
            unsafe {
                CoTaskMemFree(Some(programs_path_ptr.0.cast()));
            }
            let programs_path = programs_path_result
                .map_err(|error| format!("invalid Start Menu path: {error}"))?;

            let subfolder = format!("{programs_path}\\{product_name}");
            let nested_path = format!("{subfolder}\\{product_name}.lnk");
            let flat_path = format!("{programs_path}\\{product_name}.lnk");
            let shortcut_path = if std::path::Path::new(&nested_path).exists() {
                nested_path
            } else if std::path::Path::new(&flat_path).exists() {
                flat_path
            } else {
                std::fs::create_dir_all(&subfolder)
                    .map_err(|error| format!("could not create Start Menu folder: {error}"))?;
                nested_path
            };
            let shortcut_path = to_wide(&shortcut_path);
            let executable = to_wide(&executable.to_string_lossy());
            let link: IShellLinkW = unsafe {
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| format!("CoCreateInstance(ShellLink) failed: {error}"))?
            };
            let persist: IPersistFile = link
                .cast()
                .map_err(|error| format!("IShellLinkW -> IPersistFile failed: {error}"))?;
            if unsafe { persist.Load(PCWSTR(shortcut_path.as_ptr()), STGM_READWRITE) }.is_err() {
                unsafe {
                    link.SetPath(PCWSTR(executable.as_ptr()))
                        .map_err(|error| format!("IShellLinkW::SetPath failed: {error}"))?;
                    link.SetIconLocation(PCWSTR(executable.as_ptr()), 0)
                        .map_err(|error| format!("IShellLinkW::SetIconLocation failed: {error}"))?;
                }
            }
            let properties: IPropertyStore = link
                .cast()
                .map_err(|error| format!("IShellLinkW -> IPropertyStore failed: {error}"))?;
            let value = PROPVARIANT::from(app_id);
            unsafe {
                properties
                    .SetValue(&PKEY_AppUserModel_ID, &value)
                    .map_err(|error| format!("IPropertyStore::SetValue failed: {error}"))?;
                properties
                    .Commit()
                    .map_err(|error| format!("IPropertyStore::Commit failed: {error}"))?;
                persist
                    .Save(PCWSTR(shortcut_path.as_ptr()), true)
                    .map_err(|error| format!("IPersistFile::Save failed: {error}"))?;
            }
            Ok(())
        })();

        unsafe { CoUninitialize() };
        result
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
