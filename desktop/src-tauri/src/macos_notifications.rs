//! Modern macOS notification delivery and activation routing.
//!
//! Apple delivers every notification response through one process-wide
//! `UNUserNotificationCenterDelegate`. The delegate is installed once during
//! app setup and retained for the process lifetime. Notification targets live
//! in `userInfo`, so there are no per-notification listeners, waiter threads,
//! or request maps to leak when Notification Center clears a notification.

use block2::Block;
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_foundation::{NSDictionary, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNMutableNotificationContent, UNNotificationDefaultActionIdentifier,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use tauri::{AppHandle, Emitter};

const ACTIVATE_EVENT: &str = "native-notification-activated";
const TARGET_USER_INFO_KEY: &str = "buzzNotificationTarget";

struct NotificationDelegateIvars {
    app: AppHandle,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. The delegate is only
    // created and used on the main thread and is retained for the app lifetime.
    #[unsafe(super(NSObject))]
    #[name = "BuzzNotificationCenterDelegate"]
    #[thread_kind = MainThreadOnly]
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
            completion_handler
                .call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
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
                    if let Err(error) = self.ivars().app.emit(ACTIVATE_EVENT, target) {
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
    fn new(main_thread: MainThreadMarker, app: AppHandle) -> Retained<Self> {
        let delegate = main_thread
            .alloc()
            .set_ivars(NotificationDelegateIvars { app });
        unsafe { msg_send![super(delegate), init] }
    }
}

/// Install the one application-lifetime notification response delegate.
pub(crate) fn init(app: &AppHandle) -> tauri::Result<()> {
    let Some(main_thread) = MainThreadMarker::new() else {
        return Err(tauri::Error::FailedToReceiveMessage);
    };

    let delegate = NotificationDelegate::new(main_thread, app.clone());
    let delegate: Retained<ProtocolObject<dyn UNUserNotificationCenterDelegate>> =
        ProtocolObject::from_retained(delegate);
    UNUserNotificationCenter::currentNotificationCenter().setDelegate(Some(&delegate));

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
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, None);
    Ok(())
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
    use super::parse_target;

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
