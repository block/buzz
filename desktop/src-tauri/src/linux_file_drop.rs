//! Linux-only: recover OS file-manager drop paths from GTK.
//!
//! WebKitGTK often advertises `Files` / `text/uri-list` on dragover so the
//! composer overlay shows, then delivers an empty `dataTransfer` on drop
//! (GNOME Files + Wayland). The URI list still arrives on the GTK widget
//! via `drag-data-received`. We stash those paths and emit `os-file-drop`
//! only on `drag-drop` (button release), leaving HTML5 DnD enabled
//! (`dragDropEnabled: false`) so Windows/macOS File-object drops are unchanged.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
struct OsFileDropPayload {
    paths: Vec<String>,
}

/// Convert a `file://` URI or Unix absolute path into a local path.
///
/// Platform-independent so unit tests run everywhere. Rejects http(s) and
/// non-localhost file hosts.
pub fn file_uri_to_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }

    if trimmed.starts_with('/') && !trimmed.starts_with("//") {
        return Some(trimmed.to_string());
    }

    let parsed = url::Url::parse(trimmed).ok()?;
    if parsed.scheme() != "file" {
        return None;
    }
    let host = parsed.host_str().unwrap_or("");
    if !host.is_empty() && host != "localhost" {
        return None;
    }
    parsed
        .to_file_path()
        .ok()
        .and_then(|path| path.to_str().map(str::to_string))
}

/// Attach GTK listeners that emit `os-file-drop` when the user releases.
///
/// `drag-leave` fires on the way into a child widget *and* on a real leave,
/// so we defer the clear to an idle callback the same way wry does. A
/// following `drag-drop` still sees the stashed paths.
#[cfg(target_os = "linux")]
pub fn enable_os_file_drop<R: tauri::Runtime>(webview: &tauri::Webview<R>) {
    use gtk::prelude::WidgetExt;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use tauri::{Emitter, Manager};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum DragState {
        Entered,
        Leaving,
        Left,
    }

    let handle = webview.app_handle().clone();
    let result = webview.with_webview(move |platform_webview| {
        let gtk_webview = platform_webview.inner();
        // Ask GTK to fetch URI-list/text in addition to WebKit's HTML5 dest
        // so `drag-data-received` actually sees GNOME Files paths.
        gtk_webview.drag_dest_add_uri_targets();
        gtk_webview.drag_dest_add_text_targets();
        let pending = Rc::new(RefCell::new(Vec::<String>::new()));
        let state = Rc::new(Cell::new(DragState::Left));

        {
            let pending = pending.clone();
            let state = state.clone();
            gtk_webview.connect_drag_data_received(move |_, _, _, _, data, _, _| {
                let mut paths: Vec<String> = data
                    .uris()
                    .iter()
                    .filter_map(|uri| file_uri_to_path(uri.as_str()))
                    .collect();
                if paths.is_empty() {
                    if let Some(text) = data.text() {
                        paths = text.lines().filter_map(file_uri_to_path).collect();
                    }
                }
                *pending.borrow_mut() = paths;
                if state.get() != DragState::Entered {
                    state.set(DragState::Entered);
                }
            });
        }

        {
            let pending = pending.clone();
            let state = state.clone();
            let handle = handle.clone();
            gtk_webview.connect_drag_drop(move |_, _, _, _, _| {
                let paths = std::mem::take(&mut *pending.borrow_mut());
                state.set(DragState::Left);
                if !paths.is_empty() {
                    let _ = handle.emit("os-file-drop", OsFileDropPayload { paths });
                }
                false
            });
        }

        {
            let pending = pending.clone();
            let state = state.clone();
            gtk_webview.connect_drag_leave(move |_, _, _| {
                if state.get() == DragState::Left {
                    return;
                }
                state.set(DragState::Leaving);
                let pending = pending.clone();
                let state = state.clone();
                gtk::glib::idle_add_local_once(move || {
                    if state.get() == DragState::Leaving {
                        pending.borrow_mut().clear();
                        state.set(DragState::Left);
                    }
                });
            });
        }
    });

    if let Err(error) = result {
        eprintln!("buzz-desktop: could not attach WebKitGTK file-drop listener: {error}");
    }
}

#[cfg(not(target_os = "linux"))]
pub fn enable_os_file_drop<R: tauri::Runtime>(_webview: &tauri::Webview<R>) {}

#[cfg(test)]
mod tests {
    use super::file_uri_to_path;

    #[test]
    fn accepts_file_uri_and_absolute_path() {
        assert_eq!(
            file_uri_to_path("file:///tmp/photo.png").as_deref(),
            Some("/tmp/photo.png")
        );
        assert_eq!(
            file_uri_to_path("/tmp/photo.png").as_deref(),
            Some("/tmp/photo.png")
        );
        assert_eq!(
            file_uri_to_path("file://localhost/tmp/photo.png").as_deref(),
            Some("/tmp/photo.png")
        );
    }

    #[test]
    fn rejects_http_and_remote_hosts() {
        assert_eq!(file_uri_to_path("https://example.com/a.png"), None);
        assert_eq!(file_uri_to_path("file://nas/share/a.png"), None);
        assert_eq!(file_uri_to_path(""), None);
        assert_eq!(file_uri_to_path("# comment"), None);
    }
}
