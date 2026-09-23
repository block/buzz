# Vendored desktop patches

Pinned copies of crates.io `wry` 0.55.1 and `rfd` 0.16.0 with:

1. A nil-safe `NSOpenPanel` / `NSSavePanel` constructor.
2. A nil-safe `url_from_webview` (`WKWebView.URL` / `absoluteString`).

`+[NSOpenPanel openPanel]` can return nil (macOS 26 Tahoe, ad-hoc/unsigned
local `.app` builds, code-signature mismatch after an in-place replace).
The typed `objc2-app-kit` binding treats that as a programming error and
aborts the process. Composer attach goes through Wry's WKWebView upload
panel; `pick_and_upload_*` goes through rfd.

The panel patches retain the raw Objective-C return, cancel the picker when
it is nil, and promote the app to `NSApplicationActivationPolicyRegular`
before asking AppKit for the panel.

Separately, `WKWebView.URL()` / `NSURL.absoluteString()` can also be nil
(e.g. before first navigation or after teardown). Upstream `url_from_webview`
`unwrap()`s both; with `panic = "abort"` that kills the desktop app. The
in-tree patch returns `Err(Error::Io(...))` instead. Pin and playground
call sites already use `.url().ok()`, so the Err is handled.

Replace with a crates.io bump when tauri-apps/wry#1716 (or equivalent) is
released, rfd does the same for panels, and wry stops unwrapping nil
webview URLs.
