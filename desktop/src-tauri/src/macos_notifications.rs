//! Modern macOS notification delivery and activation routing.
//!
//! Apple delivers every notification response through one process-wide
//! `UNUserNotificationCenterDelegate`. The delegate is installed once during
//! app setup and retained for the process lifetime. Notification targets live
//! in `userInfo`, so there are no per-notification listeners, waiter threads,
//! or request maps to leak when Notification Center clears a notification.

use std::{
    collections::VecDeque,
    path::Path,
    ptr::NonNull,
    sync::{mpsc, Mutex, OnceLock},
    time::Duration,
};

use block2::{Block, RcBlock};
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, ProtocolObject},
    AnyThread, DefinedClass,
};
use objc2_foundation::{NSBundle, NSDictionary, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
    UNNotificationDefaultActionIdentifier, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNNotificationSetting, UNNotificationSettings,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use tauri::{AppHandle, Emitter};

use crate::commands::NATIVE_NOTIFICATION_ACTIVATED_EVENT;

const TARGET_USER_INFO_KEY: &str = "buzzNotificationTarget";
const MAX_PENDING_ACTIVATIONS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NotificationPermissionState {
    Default,
    Denied,
    Granted,
}

fn permission_state(status: UNAuthorizationStatus) -> NotificationPermissionState {
    match status {
        UNAuthorizationStatus::Denied => NotificationPermissionState::Denied,
        UNAuthorizationStatus::Authorized
        | UNAuthorizationStatus::Provisional
        | UNAuthorizationStatus::Ephemeral => NotificationPermissionState::Granted,
        _ => NotificationPermissionState::Default,
    }
}

static PENDING_ACTIVATIONS: OnceLock<Mutex<VecDeque<serde_json::Value>>> = OnceLock::new();

struct NotificationDelegateIvars {
    app: AppHandle,
}

define_class!(
    // SAFETY: NSObject permits AnyThread subclasses, and AppHandle is Send +
    // Sync. Apple does not guarantee a queue for notification delegate calls;
    // both Tauri operations used by the callbacks are thread-safe.
    #[unsafe(super(NSObject))]
    #[name = "BuzzNotificationCenterDelegate"]
    #[thread_kind = AnyThread]
    #[ivars = NotificationDelegateIvars]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present_notification(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &objc2_user_notifications::UNNotification,
            completion_handler: &Block<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            // Preserve the prior macOS behavior: keep foreground notifications
            // in Notification Center without interrupting the user with a banner.
            completion_handler.call((UNNotificationPresentationOptions::List,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive_notification_response(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion_handler: &Block<dyn Fn()>,
        ) {
            if &*response.actionIdentifier() == unsafe { UNNotificationDefaultActionIdentifier } {
                if let Some(target) = target_from_response(response) {
                    queue_activation(target);
                    crate::tray_menu::show_main_window(&self.ivars().app);
                    if let Err(error) = self
                        .ivars()
                        .app
                        .emit(NATIVE_NOTIFICATION_ACTIVATED_EVENT, ())
                    {
                        eprintln!(
                            "buzz-desktop: failed to emit macOS notification activation: {error}"
                        );
                    }
                }
            }

            // Apple requires this for every response, including dismissals and
            // malformed notifications that Buzz intentionally ignores.
            completion_handler.call(());
        }
    }
);

impl NotificationDelegate {
    fn new(app: AppHandle) -> Retained<Self> {
        let delegate = Self::alloc().set_ivars(NotificationDelegateIvars { app });
        unsafe { msg_send![super(delegate), init] }
    }
}

/// Install the one application-lifetime notification response delegate.
pub(crate) fn init(app: &AppHandle) -> tauri::Result<()> {
    if !is_bundled_application() {
        // UNUserNotificationCenter raises an Objective-C exception when the
        // current process has no application bundle (notably `tauri dev`).
        // objc2 cannot turn that exception into a Rust error, so do not call
        // into the framework at all in this environment.
        eprintln!(
            "buzz-desktop: macOS notifications disabled because the process is not running from an app bundle"
        );
        return Ok(());
    }

    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = NotificationDelegate::new(app.clone());
    let delegate: Retained<ProtocolObject<dyn UNUserNotificationCenterDelegate>> =
        ProtocolObject::from_retained(delegate);
    center.setDelegate(Some(&delegate));

    // UNUserNotificationCenter.delegate is weak. This object is deliberately
    // process-lifetime state, matching the application-lifetime delegate Apple
    // documents and avoiding mutable global or per-notification registrations.
    std::mem::forget(delegate);

    // Older releases registered alerts/sounds but omitted Badge. Repair only
    // that unregistered interaction, never an explicit user opt-out. This runs
    // once per bundled process without blocking startup or prompting new users.
    tauri::async_runtime::spawn_blocking(|| {
        let result =
            register_missing_badge(notification_settings_sync, request_notification_access_sync);
        if let Err(error) = result {
            // Leave the native setting unchanged; a later launch can retry.
            eprintln!("buzz-desktop: failed to register macOS badge authorization: {error}");
        }
    });
    Ok(())
}

fn ensure_bundled_application() -> Result<(), String> {
    if is_bundled_application() {
        Ok(())
    } else {
        Err(
            "macOS notifications are unavailable when Buzz is not running from an app bundle"
                .to_string(),
        )
    }
}

fn notification_settings_sync() -> Result<(UNAuthorizationStatus, UNNotificationSetting), String> {
    ensure_bundled_application()?;

    let (sender, receiver) = mpsc::sync_channel(1);
    let handler = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: Apple guarantees a live UNNotificationSettings object for
        // the duration of this completion handler.
        let settings = unsafe { settings.as_ref() };
        let _ = sender.send((settings.authorizationStatus(), settings.badgeSetting()));
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getNotificationSettingsWithCompletionHandler(&handler);

    receiver
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "macOS notification settings request timed out".to_string())
}

fn notification_permission_state_sync() -> Result<NotificationPermissionState, String> {
    notification_settings_sync().map(|(status, _)| permission_state(status))
}

fn register_missing_badge(
    settings: impl FnOnce() -> Result<(UNAuthorizationStatus, UNNotificationSetting), String>,
    request: impl FnOnce() -> Result<NotificationPermissionState, String>,
) -> Result<(), String> {
    let (permission, badge) = settings()?;
    if permission == UNAuthorizationStatus::Authorized
        && badge == UNNotificationSetting::NotSupported
    {
        request()?;
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn notification_permission_state() -> Result<NotificationPermissionState, String> {
    tokio::task::spawn_blocking(notification_permission_state_sync)
        .await
        .map_err(|error| format!("macOS notification settings task failed: {error}"))?
}

fn notification_authorization_options() -> UNAuthorizationOptions {
    // Register every interaction Buzz uses, including its Dock badge.
    UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge
}

fn request_notification_access_sync() -> Result<NotificationPermissionState, String> {
    ensure_bundled_application()?;

    let (sender, receiver) = mpsc::sync_channel(1);
    let handler = RcBlock::new(move |_granted: Bool, error: *mut NSError| {
        let result = match unsafe { error.as_ref() } {
            Some(error) => Err(format!("macOS notification authorization failed: {error}")),
            None => Ok(()),
        };
        let _ = sender.send(result);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .requestAuthorizationWithOptions_completionHandler(
            notification_authorization_options(),
            &handler,
        );

    receiver
        .recv_timeout(Duration::from_secs(60))
        .map_err(|_| "macOS notification authorization request timed out".to_string())??;
    notification_permission_state_sync()
}

#[tauri::command]
pub(crate) async fn request_notification_access() -> Result<NotificationPermissionState, String> {
    tokio::task::spawn_blocking(request_notification_access_sync)
        .await
        .map_err(|error| format!("macOS notification authorization task failed: {error}"))?
}

fn show_sync(
    title: String,
    body: Option<String>,
    target: Option<serde_json::Value>,
) -> Result<(), String> {
    ensure_bundled_application()?;
    if notification_permission_state_sync()? != NotificationPermissionState::Granted {
        return Err("macOS notification permission is not granted".to_string());
    }

    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&title));
    if let Some(body) = body {
        content.setBody(&NSString::from_str(&body));
    }

    if let Some(target) = target {
        let serialized = serde_json::to_string(&target)
            .map_err(|error| format!("failed to serialize notification target: {error}"))?;
        let key = NSString::from_str(TARGET_USER_INFO_KEY);
        let value = NSString::from_str(&serialized);
        let user_info = NSDictionary::<NSString, NSString>::from_slices(&[&*key], &[&*value]);
        // SAFETY: Both the key and value are property-list-safe NSString values.
        unsafe {
            let user_info =
                Retained::cast_unchecked::<NSDictionary<AnyObject, AnyObject>>(user_info);
            content.setUserInfo(&user_info);
        }
    }

    let identifier = NSString::from_str(&uuid::Uuid::new_v4().to_string());
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&identifier, &content, None);
    let (sender, receiver) = mpsc::sync_channel(1);
    let delivery_handler = RcBlock::new(move |error: *mut NSError| {
        let result = match unsafe { error.as_ref() } {
            Some(error) => Err(format!("failed to deliver macOS notification: {error}")),
            None => Ok(()),
        };
        let _ = sender.send(result);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&delivery_handler));

    receiver
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "macOS notification delivery request timed out".to_string())?
}

pub(crate) async fn show(
    title: String,
    body: Option<String>,
    target: Option<serde_json::Value>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || show_sync(title, body, target))
        .await
        .map_err(|error| format!("macOS notification delivery task failed: {error}"))?
}

fn queue_activation(target: serde_json::Value) {
    let queue = PENDING_ACTIVATIONS.get_or_init(Default::default);
    let Ok(mut queue) = queue.lock() else {
        eprintln!("buzz-desktop: macOS notification activation queue is unavailable");
        return;
    };
    if queue.len() == MAX_PENDING_ACTIVATIONS {
        queue.pop_front();
    }
    queue.push_back(target);
}

#[tauri::command]
pub(crate) fn take_pending_activations() -> Result<Vec<serde_json::Value>, String> {
    let queue = PENDING_ACTIVATIONS.get_or_init(Default::default);
    let mut queue = queue
        .lock()
        .map_err(|_| "macOS notification activation queue is unavailable".to_string())?;
    Ok(queue.drain(..).collect())
}

fn is_bundled_application() -> bool {
    let bundle = NSBundle::mainBundle();
    bundle.bundleIdentifier().is_some()
        && bundle.executablePath().is_some_and(|executable_path| {
            is_application_bundle_layout(
                Path::new(&bundle.bundlePath().to_string()),
                Path::new(&executable_path.to_string()),
            )
        })
}

fn is_application_bundle_layout(bundle_path: &Path, executable_path: &Path) -> bool {
    let Some(macos_path) = executable_path.parent() else {
        return false;
    };
    let Some(contents_path) = macos_path.parent() else {
        return false;
    };

    bundle_path
        .extension()
        .is_some_and(|extension| extension == "app")
        && macos_path.file_name() == Some("MacOS".as_ref())
        && contents_path.file_name() == Some("Contents".as_ref())
        && contents_path.parent() == Some(bundle_path)
}

fn target_from_response(response: &UNNotificationResponse) -> Option<serde_json::Value> {
    let user_info = response.notification().request().content().userInfo();
    let key = NSString::from_str(TARGET_USER_INFO_KEY);
    let target = user_info.objectForKey(key.as_ref())?;
    let target = target.downcast::<NSString>().ok()?;
    parse_target(&target.to_string())
}

fn parse_target(serialized: &str) -> Option<serde_json::Value> {
    serde_json::from_str(serialized).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        is_application_bundle_layout, is_bundled_application, notification_authorization_options,
        parse_target, permission_state, queue_activation, register_missing_badge,
        take_pending_activations, NotificationPermissionState, MAX_PENDING_ACTIVATIONS,
    };
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNNotificationSetting,
    };
    use std::path::Path;

    #[test]
    fn registers_only_unrequested_badges_for_already_authorized_installs() {
        for permission in [
            UNAuthorizationStatus::NotDetermined,
            UNAuthorizationStatus::Denied,
            UNAuthorizationStatus::Authorized,
            UNAuthorizationStatus::Provisional,
            UNAuthorizationStatus::Ephemeral,
        ] {
            for badge in [
                UNNotificationSetting::NotSupported,
                UNNotificationSetting::Disabled,
                UNNotificationSetting::Enabled,
            ] {
                let mut requests = 0;
                register_missing_badge(
                    || Ok((permission, badge)),
                    || {
                        requests += 1;
                        Ok(NotificationPermissionState::Granted)
                    },
                )
                .expect("registration succeeds");
                assert_eq!(
                    requests,
                    usize::from(
                        permission == UNAuthorizationStatus::Authorized
                            && badge == UNNotificationSetting::NotSupported
                    ),
                    "permission={permission:?}, badge={badge:?}",
                );
            }
        }
    }

    #[test]
    fn badge_registration_propagates_settings_and_request_failures() {
        let error = register_missing_badge(
            || Err("settings unavailable".into()),
            || panic!("must not request when settings failed"),
        )
        .expect_err("settings failure");
        assert_eq!(error, "settings unavailable");
        let error = register_missing_badge(
            || {
                Ok((
                    UNAuthorizationStatus::Authorized,
                    UNNotificationSetting::NotSupported,
                ))
            },
            || Err("request failed".into()),
        )
        .expect_err("request failure");
        assert_eq!(error, "request failed");
    }

    #[test]
    fn bundled_startup_wires_native_badge_registration() {
        // Complement behavioral tests with a wiring guard: removing the startup
        // task must not leave the isolated operation tests green.
        let source = include_str!("macos_notifications.rs");
        let init = source
            .split("pub(crate) fn init(")
            .nth(1)
            .expect("init")
            .split("fn ensure_bundled_application")
            .next()
            .expect("init body");
        assert!(init.contains("tauri::async_runtime::spawn_blocking"));
        assert!(init.contains("register_missing_badge("));
        assert!(init.contains("notification_settings_sync,"));
        assert!(init.contains("request_notification_access_sync"));
    }

    #[test]
    fn requests_all_interactions_used_by_buzz() {
        let options = notification_authorization_options();
        assert!(options.contains(UNAuthorizationOptions::Alert));
        assert!(options.contains(UNAuthorizationOptions::Sound));
        assert!(options.contains(UNAuthorizationOptions::Badge));
    }

    #[test]
    fn activation_queue_is_bounded_and_drained() {
        let _ = take_pending_activations();
        for index in 0..=MAX_PENDING_ACTIVATIONS {
            queue_activation(serde_json::json!({ "index": index }));
        }

        let activations = take_pending_activations().expect("activation queue");
        assert_eq!(activations.len(), MAX_PENDING_ACTIVATIONS);
        assert_eq!(activations[0]["index"], 1);
        assert!(take_pending_activations()
            .expect("drained activation queue")
            .is_empty());
    }

    #[test]
    fn cargo_test_process_is_not_treated_as_bundled() {
        assert!(!is_bundled_application());
    }

    #[test]
    fn requires_the_executable_to_use_the_app_bundle_layout() {
        assert!(is_application_bundle_layout(
            Path::new("/Applications/Buzz.app"),
            Path::new("/Applications/Buzz.app/Contents/MacOS/buzz-desktop"),
        ));
        assert!(!is_application_bundle_layout(
            Path::new("/tmp/Fake.app"),
            Path::new("/tmp/Fake.app/buzz-desktop"),
        ));
        assert!(!is_application_bundle_layout(
            Path::new("/Users/developer/buzz/desktop/src-tauri/target/debug"),
            Path::new("/Users/developer/buzz/desktop/src-tauri/target/debug/buzz-desktop"),
        ));
        assert!(!is_application_bundle_layout(
            Path::new("/Applications/Buzz.app"),
            Path::new("/Applications/Other.app/Contents/MacOS/buzz-desktop"),
        ));
    }

    #[test]
    fn maps_native_authorization_states_to_frontend_contract() {
        assert_eq!(
            permission_state(UNAuthorizationStatus::NotDetermined),
            NotificationPermissionState::Default
        );
        assert_eq!(
            permission_state(UNAuthorizationStatus::Denied),
            NotificationPermissionState::Denied
        );
        for status in [
            UNAuthorizationStatus::Authorized,
            UNAuthorizationStatus::Provisional,
            UNAuthorizationStatus::Ephemeral,
        ] {
            assert_eq!(
                permission_state(status),
                NotificationPermissionState::Granted
            );
        }
    }

    #[test]
    fn parses_opaque_notification_target() {
        let target =
            parse_target(r#"{"channelId":"channel","eventId":"event","threadRootId":"root"}"#)
                .expect("valid target");

        assert_eq!(target["channelId"], "channel");
        assert_eq!(target["eventId"], "event");
        assert_eq!(target["threadRootId"], "root");
    }

    #[test]
    fn rejects_malformed_notification_target() {
        assert!(parse_target("not-json").is_none());
    }
}
