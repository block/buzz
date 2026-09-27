use objc2::DeclaredClass;
use objc2_foundation::{NSObjectProtocol, NSString};
use objc2_web_kit::{
  WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationResponse,
  WKNavigationResponsePolicy,
};

#[cfg(target_os = "ios")]
use crate::wkwebview::ios::WKWebView::WKWebView;
#[cfg(target_os = "macos")]
use objc2_web_kit::WKWebView;

use crate::PageLoadEvent;

use super::class::wry_navigation_delegate::WryNavigationDelegate;

pub(crate) fn did_commit_navigation(
  this: &WryNavigationDelegate,
  webview: &WKWebView,
  _navigation: &WKNavigation,
) {
  unsafe {
    // Call on_load_handler
    if let Some(on_page_load) = &this.ivars().on_page_load_handler {
      on_page_load(PageLoadEvent::Started);
    }

    // Inject scripts
    let mut pending_scripts = this.ivars().pending_scripts.lock().unwrap();
    if let Some(scripts) = &*pending_scripts {
      for script in scripts {
        webview.evaluateJavaScript_completionHandler(&NSString::from_str(script), None);
      }
      *pending_scripts = None;
    }
  }
}

pub(crate) fn did_finish_navigation(
  this: &WryNavigationDelegate,
  _webview: &WKWebView,
  _navigation: &WKNavigation,
) {
  if let Some(on_page_load) = &this.ivars().on_page_load_handler {
    on_page_load(PageLoadEvent::Finished);
  }
}

// Navigation handler
pub(crate) fn navigation_policy(
  this: &WryNavigationDelegate,
  _webview: &WKWebView,
  action: &WKNavigationAction,
  handler: &block2::Block<dyn Fn(WKNavigationActionPolicy)>,
) {
  unsafe {
    // <https://developer.apple.com/documentation/webkit/wknavigationaction/shouldperformdownload>
    // Available: macOS 11.3+, iOS 14.5+
    let can_download = action.respondsToSelector(objc2::sel!(shouldPerformDownload));
    let should_download: bool = if can_download {
      action.shouldPerformDownload()
    } else {
      false
    };
    let request = action.request();
    let url = request
      .URL()
      .and_then(|u| u.absoluteString())
      .map(|s| s.to_string())
      .unwrap_or_default();

    // target=_blank / window.open: targetFrame is nil.
    // Middle-click (buttonNumber 2) also means "open in new tab" even when
    // targetFrame is the current frame.
    // When the request already has a concrete URL, emit + Cancel here so
    // left-click cannot fall through as a no-op after createWebView Deny.
    // When the URL is still empty / about:blank (common on the first
    // decidePolicy pass), Allow so createWebView can run with the real
    // request — Canceling an empty URL blocks createWebView and leaves
    // left-click dead while right-click "Open Link in New Window" still
    // works (it often hits createWebView directly).
    #[cfg(target_os = "macos")]
    {
      let is_new_window_target = action.targetFrame().is_none();
      let is_middle_click = action.respondsToSelector(objc2::sel!(buttonNumber))
        && action.buttonNumber() == 2;
      if is_new_window_target || is_middle_click {
        let usable = !url.is_empty() && url != "about:blank";
        if usable {
          if let Some(ref on_new_window) = this.ivars().new_window_url_handler {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
              on_new_window(url.clone());
            }));
          }
          (*handler).call((WKNavigationActionPolicy::Cancel,));
          return;
        }
        if is_new_window_target {
          // Fall through → Allow → createWebView emits + Deny.
        } else {
          // Middle-click with blank URL: do not hijack same-frame policy.
        }
      }
    }

    if should_download {
      let has_download_handler = this.ivars().has_download_handler;
      if has_download_handler {
        (*handler).call((WKNavigationActionPolicy::Download,));
      } else {
        (*handler).call((WKNavigationActionPolicy::Cancel,));
      }
    } else {
      let function = &this.ivars().navigation_policy_function;
      match function(url) {
        true => (*handler).call((WKNavigationActionPolicy::Allow,)),
        false => (*handler).call((WKNavigationActionPolicy::Cancel,)),
      };
    }
  }
}

// Navigation handler
pub(crate) fn navigation_policy_response(
  this: &WryNavigationDelegate,
  _webview: &WKWebView,
  response: &WKNavigationResponse,
  handler: &block2::Block<dyn Fn(WKNavigationResponsePolicy)>,
) {
  unsafe {
    let can_show_mime_type = response.canShowMIMEType();

    if !can_show_mime_type {
      let has_download_handler = this.ivars().has_download_handler;
      if has_download_handler {
        (*handler).call((WKNavigationResponsePolicy::Download,));
        return;
      }
    }

    (*handler).call((WKNavigationResponsePolicy::Allow,));
  }
}

pub(crate) fn web_content_process_did_terminate(
  this: &WryNavigationDelegate,
  _webview: &WKWebView,
) {
  if let Some(on_web_content_process_terminate) =
    &this.ivars().on_web_content_process_terminate_handler
  {
    on_web_content_process_terminate();
  }
}
