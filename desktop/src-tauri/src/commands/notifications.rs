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

/// Claim this app's notification identity at startup, where the platform has
/// one to claim.
///
/// Exposed as a single cross-platform entry point so `lib.rs` can call it
/// unconditionally instead of carrying a `#[cfg]` block at the call site —
/// the same shape `mouse_nav.rs` uses for its macOS-only wiring. On Windows
/// this claims the AUMID and repairs the Start Menu shortcut so the OS
/// recognises Buzz as notification-capable; everywhere else it does nothing.
#[cfg(target_os = "windows")]
pub(crate) fn ensure_startup_registration(app: &tauri::AppHandle) {
    windows::ensure_startup_registration(app);
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn ensure_startup_registration(_app: &tauri::AppHandle) {}

#[cfg(target_os = "windows")]
pub(crate) mod windows {
    use std::sync::mpsc;
    use std::sync::Once;

    static AUMID_REGISTERED: Once = Once::new();
    static STARTUP_REGISTRATION_DONE: Once = Once::new();
    static PROCESS_MTA_READY: Once = Once::new();

    /// Establishes a process-wide multi-threaded COM apartment (MTA) that lives
    /// for the entire lifetime of the app, and blocks until it is ready.
    ///
    /// This is the load-bearing piece for Windows toast delivery.
    /// `tauri-winrt-notification`'s `Toast::show()` calls into WinRT
    /// (`XmlDocument::new`, `ToastNotificationManager::CreateToastNotifierWithId`,
    /// `ToastNotifier::Show`) and does no COM apartment init of its own —
    /// confirmed by reading the crate source. The upstream Tauri notification
    /// plugin gets away with that only because it posts from the tokio runtime,
    /// where some other thread has already created an MTA and the worker joins
    /// it via COM's "implicit MTA" rule (Raymond Chen,
    /// <https://devblogs.microsoft.com/oldnewthing/20130419-00/?p=4613>). Buzz
    /// posts toasts from a bare `std::thread`, so unless the process already
    /// has a live MTA, every one of those WinRT calls fails with
    /// `CO_E_NOTINITIALIZED`. `Toast::show()` maps that to `Err`, which the
    /// caller only `eprintln!`s — and release builds set
    /// `windows_subsystem = "windows"`, so there is no console and the error
    /// vanishes. Net effect: no toast ever appears, silently, on every launch,
    /// on every machine. That is exactly the bug this fixes.
    ///
    /// We make the dependency explicit and permanent: one dedicated thread
    /// initializes an MTA and then parks forever. It must never
    /// `CoUninitialize` or exit — holding the apartment open is the entire
    /// point. This does two things a per-notification `CoInitializeEx`/
    /// `CoUninitialize` pair (the tempting shorter fix) cannot:
    ///   1. Every notification thread joins the implicit MTA with zero per-call
    ///      init, so there is no `CoUninitialize`-then-exit race against the
    ///      Windows Notification Platform's async delivery
    ///      (microsoft/WindowsAppSDK#3437).
    ///   2. The `on_activated` click handler stays deliverable. In an MTA,
    ///      WinRT delivers `Activated`/`Dismissed` callbacks on COM worker
    ///      threads rather than a per-thread message pump, so the short-lived
    ///      thread that showed the toast can exit while the click handler still
    ///      fires later — as long as the apartment itself is still alive, which
    ///      this permanent thread guarantees. A per-call `CoUninitialize` +
    ///      thread exit would tear the sink down and silently drop every click.
    ///
    /// MTA (not STA/`COINIT_APARTMENTTHREADED`) is correct here: STA would
    /// require this thread to run a window message pump to deliver callbacks,
    /// which a parked thread does not. Microsoft's own WinRT guidance is to use
    /// MTA for non-UI threads and reserve STA for UI threads
    /// (<https://microsoft.github.io/MIDI/kb/threading-and-initialization/>).
    fn ensure_process_mta() {
        PROCESS_MTA_READY.call_once(|| {
            let (ready_tx, ready_rx) = mpsc::channel::<()>();
            std::thread::spawn(move || {
                use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
                // SAFETY: this thread performs no other COM work and never
                // uninitializes; the apartment is intentionally leaked for the
                // process lifetime. On a fresh thread this returns S_OK (or
                // S_FALSE if the process MTA already exists) — both are
                // non-negative, so `is_err()` is false. It cannot return
                // RPC_E_CHANGED_MODE because this thread has never been
                // initialized with a different model.
                let hresult = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                if hresult.is_err() {
                    eprintln!(
                        "buzz-desktop: failed to establish process MTA for notifications: {hresult:?}"
                    );
                }
                // Signal readiness regardless: even on the (unexpected) error
                // path, blocking callers must not hang.
                let _ = ready_tx.send(());
                // Park forever. Returning from this closure would end the
                // thread and tear the apartment down — never do that.
                loop {
                    std::thread::park();
                }
            });
            // Block until the apartment is confirmed up, so the very first
            // toast can never race ahead of it.
            let _ = ready_rx.recv();
        });
    }

    /// Runs Buzz's full Windows toast-notification eligibility setup once,
    /// early in app startup — call this from `lib.rs`'s `setup()`, not
    /// lazily on first notification. See `ensure_start_menu_shortcut` for
    /// why this needs to run regardless of whether the user has actually
    /// triggered a notification yet: Windows' Settings > Notifications page
    /// only lists an AUMID once it has a shell-visible identity (a Start
    /// Menu shortcut carrying that AUMID), and users reasonably expect to
    /// find Buzz there before the first notification would ever fire.
    pub(crate) fn ensure_startup_registration(app: &tauri::AppHandle) {
        STARTUP_REGISTRATION_DONE.call_once(|| {
            let app = app.clone();
            // Shell/COM calls can be slow (first-run disk I/O for the
            // Start Menu folder) — do this off the main setup thread so
            // it never delays window creation.
            std::thread::spawn(move || {
                // Bring the process MTA up at launch so the first toast never
                // has to wait on it. Idempotent and cheap after the first call.
                ensure_process_mta();
                let app_id = app.config().identifier.clone();
                set_process_aumid(&app_id);
                if let Err(error) = write_aumid_registry_entry(&app, &app_id) {
                    eprintln!(
                        "buzz-desktop: failed to register AUMID for Windows notifications: {error}"
                    );
                }
                if let Err(error) = write_notification_settings_entry(&app_id) {
                    eprintln!(
                        "buzz-desktop: failed to pre-register Windows notification settings entry: {error}"
                    );
                }
                if let Err(error) = ensure_start_menu_shortcut(&app, &app_id) {
                    eprintln!("buzz-desktop: failed to repair Start Menu shortcut AUMID: {error}");
                }
            });
        });
    }

    pub fn show(
        app: tauri::AppHandle,
        title: String,
        body: Option<String>,
        target: Option<serde_json::Value>,
    ) {
        use tauri::Emitter;

        let app_id = app.config().identifier.clone();
        ensure_aumid_registered(&app, &app_id);
        // `Toast::show()` below calls into WinRT, which requires the calling
        // thread to be in a COM apartment. See `ensure_process_mta` — this
        // guarantees a process-wide MTA is live before we spawn, so the bare
        // thread below joins it implicitly and the WinRT calls succeed.
        ensure_process_mta();

        std::thread::spawn(move || {
            let mut toast = tauri_winrt_notification::Toast::new(&app_id).text1(&title);
            if let Some(body_text) = body.as_deref() {
                toast = toast.text2(body_text);
            }

            // Without this, clicking the toast does nothing — Linux and
            // macOS both focus the window and route to `target` on click
            // (see `linux::show`/`macos_notifications.rs`); Windows never
            // did. `on_activated` fires with `None` for a plain click on
            // the toast body (no button involved), which is the only case
            // we handle today — the button-click `Some(action)` path is
            // free for future per-notification actions (e.g. "Mark read").
            let activation_app = app.clone();
            toast = toast.on_activated(move |action| {
                if action.is_none() {
                    let _ = activation_app.emit(
                        crate::commands::NATIVE_NOTIFICATION_ACTIVATED_EVENT,
                        target.clone(),
                    );
                }
                Ok(())
            });

            if let Err(error) = toast.show() {
                eprintln!("buzz-desktop: failed to post Windows native notification: {error}");
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
            if let Err(error) = write_notification_settings_entry(app_id) {
                eprintln!(
                    "buzz-desktop: failed to pre-register Windows notification settings entry: {error}"
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
        let hresult = unsafe { SetCurrentProcessExplicitAppUserModelID(wide_app_id.as_ptr()) };
        if hresult < 0 {
            eprintln!(
                "buzz-desktop: SetCurrentProcessExplicitAppUserModelID failed: 0x{hresult:08X}"
            );
        }
    }

    /// Pre-seeds the per-app notification Settings key so Buzz appears in
    /// Settings > System > Notifications immediately on a clean install,
    /// instead of only after the first toast is delivered.
    ///
    /// Create-if-absent by design. Windows owns the `Enabled` value under this
    /// key: once the key exists, its value reflects either Windows' own state
    /// or a choice the user made in Settings > Notifications. Rewriting
    /// `Enabled=1` on every launch (as the first version of this did) would
    /// silently turn Buzz notifications back on every time the app starts,
    /// overriding a user who had deliberately turned them off. So we only write
    /// the seed values when we are the ones creating the key for the first
    /// time; if it already exists we leave it entirely untouched.
    fn write_notification_settings_entry(app_id: &str) -> Result<(), String> {
        use windows_sys::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
            KEY_READ, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE,
        };

        let subkey = to_wide(&format!(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Notifications\\Settings\\{app_id}"
        ));

        // SAFETY: `subkey` is a NUL-terminated UTF-16 buffer that outlives
        // every call below; each key handle we obtain is closed exactly once
        // on every path.
        unsafe {
            // If the key already exists, respect whatever state is there —
            // never re-seed. See the doc comment above.
            let mut existing: HKEY = std::ptr::null_mut();
            let open_status =
                RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut existing);
            if open_status == 0 {
                RegCloseKey(existing);
                return Ok(());
            }

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
                return Err(format!(
                    "RegCreateKeyExW failed with status {create_status}"
                ));
            }

            let show_in_action_center_value = to_wide("ShowInActionCenter");
            let show_in_action_center_data: u32 = 1;
            let enabled_value = to_wide("Enabled");
            let enabled_data: u32 = 1;

            let show_status = RegSetValueExW(
                hkey,
                show_in_action_center_value.as_ptr(),
                0,
                REG_DWORD,
                (&show_in_action_center_data as *const u32).cast::<u8>(),
                std::mem::size_of::<u32>() as u32,
            );
            let enabled_status = RegSetValueExW(
                hkey,
                enabled_value.as_ptr(),
                0,
                REG_DWORD,
                (&enabled_data as *const u32).cast::<u8>(),
                std::mem::size_of::<u32>() as u32,
            );

            RegCloseKey(hkey);

            if enabled_status != 0 {
                return Err(format!(
                    "RegSetValueExW(Enabled) failed with status {enabled_status}"
                ));
            }
            if show_status != 0 {
                return Err(format!(
                    "RegSetValueExW(ShowInActionCenter) failed with status {show_status}"
                ));
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
                return Err(format!(
                    "RegCreateKeyExW failed with status {create_status}"
                ));
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
                return Err(format!(
                    "RegSetValueExW(DisplayName) failed with status {display_status}"
                ));
            }
            if icon_status != 0 {
                return Err(format!(
                    "RegSetValueExW(IconUri) failed with status {icon_status}"
                ));
            }
        }

        Ok(())
    }

    /// Creates or repairs the per-user Start Menu shortcut so it carries a
    /// `System.AppUserModel.ID` property matching `app_id`.
    ///
    /// This is the piece the registry/process-AUMID calls above cannot
    /// substitute for. For an unpackaged (non-MSIX) Win32 app, Windows only
    /// treats an AUMID as a real, notification-capable app identity once
    /// the shell can resolve it back to a Start Menu shortcut carrying that
    /// same AUMID as a shell-link property — that's what makes the app show
    /// up under Settings > Notifications and what makes `ToastNotifier`
    /// accept toasts at all, not just the registry DisplayName/IconUri
    /// metadata written above.
    ///
    /// The installer's NSIS template already sets this property on the
    /// shortcut it creates — but only for a *fresh* install. Tauri's
    /// generated NSIS `CreateOrUpdateStartMenuShortcut` macro skips
    /// recreating the shortcut on an in-place update (which is what every
    /// silent auto-update via the Tauri updater is), so any install whose
    /// shortcut predates this AUMID work — or was ever silently updated
    /// before this fix — never gets it patched by the installer. Repairing
    /// it here, unconditionally, on every app launch, makes this self-
    /// healing regardless of install history or update path.
    fn ensure_start_menu_shortcut(app: &tauri::AppHandle, app_id: &str) -> Result<(), String> {
        // NOTE: `IPersistFile` and `STGM_READWRITE` live under
        // `Win32::System::Com` (general COM persistence/storage types), not
        // `Win32::UI::Shell`, even though we only use `IPersistFile` here to
        // persist a shell link — double-check this against the actual crate
        // docs for the pinned `windows` version if this module fails to
        // resolve.
        use windows::core::{Interface, PCWSTR};
        // `PKEY_AppUserModel_ID` lives here, not under
        // `Win32::UI::Shell::PropertiesSystem` — that module only carries
        // `PKEY_PIDSTR_MAX`. Verified against the crate's own generated docs;
        // the wrong path fails to compile rather than silently misbehaving.
        use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED, STGM_READWRITE,
        };
        // No `InitPropVariantFromStringW` here — it doesn't exist in this
        // crate (or in any windows-rs version): the Win32 API of that name
        // is a header-only inline wrapper in propvarutil.h, not a real
        // exported symbol, so win32metadata never carries it (see
        // microsoft/windows-rs#976, still open). `PROPVARIANT: From<&str>`
        // is the crate's own replacement, and `PROPVARIANT: Drop` already
        // clears it — do not additionally call `PropVariantClear` on one of
        // these or it double-frees the string it owns.
        use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
        use windows::Win32::UI::Shell::{
            FOLDERID_Programs, IShellLinkW, PropertiesSystem::IPropertyStore, SHGetKnownFolderPath,
            ShellLink, KF_FLAG_CREATE,
        };

        let product_name = app
            .config()
            .product_name
            .clone()
            .unwrap_or_else(|| "Buzz".to_string());
        let exe_path = std::env::current_exe()
            .map_err(|error| format!("could not resolve current exe path: {error}"))?;

        // SAFETY: this thread does not otherwise touch COM; the apartment
        // is torn down before returning from this function on every path.
        // `CoInitializeEx` returns `S_OK` on first init and `S_FALSE` if this
        // thread already has an apartment (both non-negative, so `is_err()`
        // is false for either) — only a genuinely negative HRESULT here
        // means initialization failed.
        let init_result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if init_result.is_err() {
            return Err(format!("CoInitializeEx failed: {init_result:?}"));
        }
        let result = (|| -> Result<(), String> {
            let programs_dir_pwstr = unsafe {
                SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_CREATE, None)
                    .map_err(|error| format!("SHGetKnownFolderPath failed: {error}"))?
            };
            // `SHGetKnownFolderPath` hands back a COM-allocated buffer the
            // caller owns and must free — `windows-rs` does not do this
            // automatically for a raw returned `PWSTR`.
            let programs_dir_result = unsafe { programs_dir_pwstr.to_string() };
            unsafe {
                windows::Win32::System::Com::CoTaskMemFree(Some(
                    programs_dir_pwstr.0 as *const std::ffi::c_void,
                ));
            }
            let programs_dir =
                programs_dir_result.map_err(|error| format!("invalid Start Menu path: {error}"))?;

            // NSIS's `MUI_STARTMENU_GETFOLDER` (which Tauri's generated
            // installer.nsi uses — see its `$AppStartMenuFolder` var)
            // defaults to putting the shortcut in a subfolder named after
            // the product, i.e. `$SMPROGRAMS\Buzz\Buzz.lnk`, NOT flat at
            // `$SMPROGRAMS\Buzz.lnk`. Confirmed by reading the actual
            // generated installer.nsi from a real local build — it only
            // falls back to the flat path if the user explicitly cleared
            // the Start Menu folder field during install (installer.nsi
            // lines ~912-917). Try the subfolder path first, since it's the
            // default nearly everyone gets; fall back to flat if that's not
            // what's actually on disk; if neither exists yet, create at the
            // subfolder path to match what a fresh install would produce.
            let subfolder_path = format!("{programs_dir}\\{product_name}\\{product_name}.lnk");
            let flat_path = format!("{programs_dir}\\{product_name}.lnk");
            let shortcut_path = if std::path::Path::new(&subfolder_path).exists() {
                subfolder_path
            } else if std::path::Path::new(&flat_path).exists() {
                flat_path
            } else {
                // Neither exists — this is a from-scratch repair (no prior
                // install ever ran, or its shortcut was deleted). Ensure the
                // subfolder exists, then create there, matching the
                // installer's own default layout.
                std::fs::create_dir_all(format!("{programs_dir}\\{product_name}"))
                    .map_err(|error| format!("could not create Start Menu folder: {error}"))?;
                subfolder_path
            };
            let shortcut_path_wide = to_wide(&shortcut_path);
            let exe_path_wide = to_wide(&exe_path.to_string_lossy());

            let shell_link: IShellLinkW = unsafe {
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| format!("CoCreateInstance(ShellLink) failed: {error}"))?
            };
            let persist_file: IPersistFile = shell_link
                .cast()
                .map_err(|error| format!("IShellLinkW -> IPersistFile cast failed: {error}"))?;

            // If a shortcut already exists (the normal case — the installer
            // made one), load it so we preserve whatever else it sets
            // (working directory, description, etc.) and only touch the
            // AUMID property. If it doesn't exist yet, fall through and
            // build a fresh one below.
            let existing =
                unsafe { persist_file.Load(PCWSTR(shortcut_path_wide.as_ptr()), STGM_READWRITE) };
            if existing.is_err() {
                unsafe {
                    shell_link
                        .SetPath(PCWSTR(exe_path_wide.as_ptr()))
                        .map_err(|error| format!("IShellLinkW::SetPath failed: {error}"))?;
                    shell_link
                        .SetIconLocation(PCWSTR(exe_path_wide.as_ptr()), 0)
                        .map_err(|error| format!("IShellLinkW::SetIconLocation failed: {error}"))?;
                }
            }

            let props: IPropertyStore = shell_link
                .cast()
                .map_err(|error| format!("IShellLinkW -> IPropertyStore cast failed: {error}"))?;
            // `prop_value` owns the wide-string copy `SetValue` reads; it is
            // freed automatically (via `PROPVARIANT`'s `Drop`) when it goes
            // out of scope below, after `Commit`/`Save` are done with it.
            let prop_value = PROPVARIANT::from(app_id);
            unsafe {
                props
                    .SetValue(&PKEY_AppUserModel_ID, &prop_value)
                    .map_err(|error| format!("IPropertyStore::SetValue failed: {error}"))?;
                props
                    .Commit()
                    .map_err(|error| format!("IPropertyStore::Commit failed: {error}"))?;
                persist_file
                    .Save(PCWSTR(shortcut_path_wide.as_ptr()), true)
                    .map_err(|error| format!("IPersistFile::Save failed: {error}"))?;
            }

            Ok(())
        })();

        if init_result.is_ok() {
            unsafe { CoUninitialize() };
        }
        result
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
