//! Modern macOS notification delivery and activation routing.
//!
//! Apple delivers every notification response through one process-wide
//! `UNUserNotificationCenterDelegate`. The delegate is installed once during
//! app setup and retained for the process lifetime. Notification targets live
//! in `userInfo`, so there are no per-notification listeners, waiter threads,
//! or request maps to leak when Notification Center clears a notification.

use std::sync::mpsc;

use block2::{Block, RcBlock};
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, ProtocolObject},
    AnyThread, DefinedClass,
};
use objc2_foundation::{NSBundle, NSDictionary, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationDefaultActionIdentifier,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use tauri::{AppHandle, Emitter};

use crate::commands::NATIVE_NOTIFICATION_ACTIVATED_EVENT;

const TARGET_USER_INFO_KEY: &str = "buzzNotificationTarget";

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
            // Preserve the prior macOS behavior: foreground notifications are
            // recorded by Notification Center without interrupting the user
            // with a banner while Buzz is active.
            completion_handler.call((UNNotificationPresentationOptions::empty(),));
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
                    crate::tray_menu::show_main_window(&self.ivars().app);
                    if let Err(error) = self
                        .ivars()
                        .app
                        .emit(NATIVE_NOTIFICATION_ACTIVATED_EVENT, target)
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
            "buzz-desktop: macOS notifications disabled because the process has no bundle identifier"
        );
        return Ok(());
    }

    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = NotificationDelegate::new(app.clone());
    let delegate: Retained<ProtocolObject<dyn UNUserNotificationCenterDelegate>> =
        ProtocolObject::from_retained(delegate);
    center.setDelegate(Some(&delegate));

    let authorization_handler = RcBlock::new(|granted: Bool, error: *mut NSError| {
        if let Some(error) = unsafe { error.as_ref() } {
            eprintln!("buzz-desktop: macOS notification authorization failed: {error}");
        } else if !granted.as_bool() {
            eprintln!("buzz-desktop: macOS notification authorization was denied");
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &authorization_handler,
    );

    // UNUserNotificationCenter.delegate is weak. This object is deliberately
    // process-lifetime state, matching the application-lifetime delegate Apple
    // documents and avoiding mutable global or per-notification registrations.
    std::mem::forget(delegate);
    Ok(())
}

pub(crate) fn show(
    title: String,
    body: Option<String>,
    target: Option<serde_json::Value>,
) -> Result<(), String> {
    if !is_bundled_application() {
        return Err(
            "macOS notifications are unavailable when Buzz is not running from an app bundle"
                .to_string(),
        );
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
        let result = if let Some(error) = unsafe { error.as_ref() } {
            Err(format!("failed to deliver macOS notification: {error}"))
        } else {
            Ok(())
        };
        let _ = sender.send(result);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&delivery_handler));
    receiver
        .recv()
        .map_err(|error| format!("macOS notification delivery callback failed: {error}"))?
}

fn is_bundled_application() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
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
    use super::{is_bundled_application, parse_target};

    #[test]
    fn cargo_test_process_is_not_treated_as_bundled() {
        assert!(!is_bundled_application());
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
