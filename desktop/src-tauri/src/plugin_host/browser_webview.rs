//! Direct raw-wry (`wry = "=0.56.1"`) adapter for the plugin browser surface.
//!
//! The remote page this webview renders has no route into Buzz: no Tauri IPC,
//! no injected initialization script, no custom protocol. Traversal uses
//! wry's own native history (`go_back`/`go_forward`); this module never
//! reads or writes page state by running script against it.
//!
//! The wry `WebView` lives only in this module, main-thread-owned, in one
//! thread-local slot (`LiveBrowserView`). Every `plugin_browser_*` command
//! (owned by the host, `plugin_host/commands.rs`) dispatches onto the main
//! thread before calling into the plain functions below, so every function in
//! this file assumes it is already running on the main thread.

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::RefCell;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use serde::Serialize;
    use tauri::{AppHandle, Emitter, Manager};
    use wry::WebViewBuilderExtDarwin;

    use crate::plugin_host::types::{Bounds, BrowserError, PageLoad, SessionId};

    const POLL_INTERVAL: Duration = Duration::from_millis(500);

    thread_local! {
        static LIVE_VIEW: RefCell<Option<LiveBrowserView>> = const { RefCell::new(None) };
    }

    /// Main-thread-owned wry webview for the plugin browser surface. Never
    /// named outside this module — the command layer only ever sees
    /// `SessionId` and `OpenedSurface`, never a wry handle. Opening a new
    /// session replaces whatever this slot held; only one browser surface is
    /// live at a time.
    struct LiveBrowserView {
        session: SessionId,
        generation: u64,
        app: AppHandle,
        view: wry::WebView,
        poll_active: Arc<AtomicBool>,
        // `WebView::url()` assumes a non-null WKWebView URL internally and can
        // panic before the first committed navigation; polling stays gated
        // until the first `Started` or `Finished` callback fires.
        armed: Arc<AtomicBool>,
        last_url: String,
        last_can_go_back: bool,
        last_can_go_forward: bool,
        // Set once the web content process terminates. A dead `wry::WebView`
        // stays in the slot until `close()` so `current_url`/history queries
        // keep a stable error, but no later command may act on it — in
        // particular `set_visible(true)` must never reshow a dead view.
        terminated: bool,
    }

    fn to_wry_rect(bounds: Bounds) -> wry::Rect {
        wry::Rect {
            position: wry::dpi::LogicalPosition::new(bounds.x, bounds.y).into(),
            size: wry::dpi::LogicalSize::new(bounds.width, bounds.height).into(),
        }
    }

    fn fixed_message(code: &str) -> &'static str {
        match code {
            "navigation-denied" => "That address isn't allowed.",
            "load-failed" => "The page failed to load.",
            "plugin-timeout" => "The plugin didn't respond in time.",
            "plugin-unavailable" => "The plugin is unavailable.",
            "plugin-protocol-error" => "The plugin returned an invalid response.",
            "plugin-contract-mismatch" => {
                "The plugin's response didn't match the expected contract."
            }
            "unsupported-platform" => "This feature isn't available on your platform.",
            _ => "An unexpected error occurred.",
        }
    }

    fn fixed_error(session: &SessionId, generation: u64, code: &str) -> BrowserError {
        BrowserError {
            session_id: session.0.clone(),
            generation,
            code: code.to_string(),
            message: fixed_message(code).to_string(),
        }
    }

    #[derive(Serialize, Clone)]
    #[serde(rename_all = "camelCase")]
    struct LocationPayload<'a> {
        session_id: &'a str,
        generation: u64,
        url: &'a str,
        can_go_back: bool,
        can_go_forward: bool,
    }

    fn emit_location(
        app: &AppHandle,
        session: &SessionId,
        generation: u64,
        url: &str,
        can_go_back: bool,
        can_go_forward: bool,
    ) {
        let payload = LocationPayload {
            session_id: &session.0,
            generation,
            url,
            can_go_back,
            can_go_forward,
        };
        let _ = app.emit("plugin-browser-location", payload);
    }

    fn emit_error(app: &AppHandle, session: &SessionId, generation: u64, code: &str) {
        let _ = app.emit(
            "plugin-browser-error",
            fixed_error(session, generation, code),
        );
    }

    /// Runs on the main thread. Reads the currently live view's URL and
    /// history state and emits `plugin-browser-location` when either
    /// changed. `WebView::url()` assumes a non-null WKWebView URL internally
    /// and can panic; the panic is contained here, polling stops, the view is
    /// hidden, the host's load deadline for this session is stopped (so a
    /// late `Started`/`Finished` callback cannot re-arm it after this
    /// terminal state), and a fixed `load-failed` error is reported — the
    /// same treatment given to a web content process termination.
    fn poll_tick(poll_active: &Arc<AtomicBool>) {
        if !poll_active.load(Ordering::Acquire) {
            return;
        }
        LIVE_VIEW.with(|cell| {
            let mut slot = cell.borrow_mut();
            let Some(live) = slot.as_mut() else {
                return;
            };
            if !Arc::ptr_eq(&live.poll_active, poll_active) {
                return;
            }
            if !live.armed.load(Ordering::Acquire) {
                // No main-document load has committed or finished yet;
                // `WebView::url()` can panic before that point.
                return;
            }
            let url = match catch_unwind(AssertUnwindSafe(|| live.view.url())) {
                Ok(Ok(url)) => url,
                _ => {
                    live.poll_active.store(false, Ordering::Release);
                    live.terminated = true;
                    let _ = live.view.set_visible(false);
                    live.app
                        .state::<crate::plugin_host::commands::LoadDeadlines>()
                        .stop_loading(&live.session.0);
                    emit_error(&live.app, &live.session, live.generation, "load-failed");
                    return;
                }
            };
            let can_go_back = live.view.can_go_back().unwrap_or(false);
            let can_go_forward = live.view.can_go_forward().unwrap_or(false);
            if url != live.last_url
                || can_go_back != live.last_can_go_back
                || can_go_forward != live.last_can_go_forward
            {
                live.last_url = url.clone();
                live.last_can_go_back = can_go_back;
                live.last_can_go_forward = can_go_forward;
                emit_location(
                    &live.app,
                    &live.session,
                    live.generation,
                    &url,
                    can_go_back,
                    can_go_forward,
                );
            }
        });
    }

    /// Runs on the main thread from the web content process termination
    /// delegate. Stops polling, hides the dead view, stops the host's load
    /// deadline for this session (see `commands::LoadDeadlines::stop_loading`),
    /// and reports the same fixed `load-failed` error the poll loop uses for
    /// a contained panic.
    ///
    /// `owner` is the `poll_active` `Arc` created for the specific session
    /// this delegate was registered for. A WKWebView's termination delegate
    /// can fire after this module has already replaced it with a new
    /// session's view (the old `wry::WebView`'s drop does not guarantee the
    /// OS has already delivered a pending termination notification for it).
    /// Without this check, a stale termination for an old, already-replaced
    /// session would incorrectly mark the *new* session's `LiveBrowserView`
    /// as terminated — the same staleness `poll_tick` already guards against
    /// via the identical `Arc::ptr_eq` comparison.
    fn handle_terminated(owner: &Arc<AtomicBool>) {
        LIVE_VIEW.with(|cell| {
            let mut slot = cell.borrow_mut();
            if let Some(live) = slot.as_mut() {
                if !Arc::ptr_eq(&live.poll_active, owner) {
                    return;
                }
                live.poll_active.store(false, Ordering::Release);
                live.terminated = true;
                let _ = live.view.set_visible(false);
                live.app
                    .state::<crate::plugin_host::commands::LoadDeadlines>()
                    .stop_loading(&live.session.0);
                emit_error(&live.app, &live.session, live.generation, "load-failed");
            }
        });
    }

    fn spawn_poll_loop(app: AppHandle, poll_active: Arc<AtomicBool>) {
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(POLL_INTERVAL).await;
                if !poll_active.load(Ordering::Acquire) {
                    break;
                }
                let tick_flag = Arc::clone(&poll_active);
                if app
                    .run_on_main_thread(move || poll_tick(&tick_flag))
                    .is_err()
                {
                    break;
                }
            }
        });
    }

    /// Runs a closure against the currently live view, only if its session
    /// matches and its content process has not terminated. There is no
    /// closed `BrowserError` code for "no such session"; a mismatch or empty
    /// slot reports `plugin-unavailable`, the closest fit in the fixed set.
    /// A terminated view reports `load-failed` — the same code
    /// `handle_terminated` already emitted — rather than operating on (or,
    /// for `set_visible(true)`, reshowing) a dead `wry::WebView`. The host is
    /// expected to only ever call these functions for a session it still
    /// considers live.
    fn with_live<T>(
        session: &SessionId,
        f: impl FnOnce(&mut LiveBrowserView) -> Result<T, BrowserError>,
    ) -> Result<T, BrowserError> {
        LIVE_VIEW.with(|cell| {
            let mut slot = cell.borrow_mut();
            match slot.as_mut() {
                Some(live) if &live.session == session && live.terminated => {
                    Err(fixed_error(session, live.generation, "load-failed"))
                }
                Some(live) if &live.session == session => f(live),
                _ => Err(fixed_error(session, 0, "plugin-unavailable")),
            }
        })
    }

    /// Builds an incognito child view with fixed permissions and decision observers.
    #[expect(
        clippy::too_many_arguments,
        reason = "Permission and download observers record independent WebKit decisions."
    )]
    pub fn build_child(
        window: &impl wry::raw_window_handle::HasWindowHandle,
        url: &str,
        bounds: Bounds,
        admit: Arc<dyn Fn(&str) -> bool + Send + Sync>,
        on_page_load: Arc<dyn Fn(PageLoad, String) + Send + Sync>,
        on_permission_decision: Arc<
            dyn Fn(wry::PermissionKind, wry::PermissionResponse) + Send + Sync,
        >,
        on_download_decision: Arc<dyn Fn(String, bool) + Send + Sync>,
        // Unique identity for this webview's content-process termination
        // delegate. The delegate can fire after this specific view has
        // already been replaced by a later session's; the callback compares
        // this token by pointer identity against whatever session is
        // currently live before acting, so a stale termination for a
        // since-replaced session cannot mark the new one dead. Callers with
        // only ever one session live at a time may pass any fresh, otherwise
        // unused `Arc<AtomicBool>`.
        terminate_owner: Arc<AtomicBool>,
    ) -> Result<wry::WebView, BrowserError> {
        wry::WebViewBuilder::new()
            .with_url(url)
            .with_bounds(to_wry_rect(bounds))
            .with_incognito(true)
            .with_navigation_handler(move |candidate: String| admit(&candidate))
            .with_on_page_load_handler(move |event, loaded_url| {
                let kind = match event {
                    wry::PageLoadEvent::Started => PageLoad::Started,
                    wry::PageLoadEvent::Finished => PageLoad::Finished,
                };
                on_page_load(kind, loaded_url);
            })
            .with_new_window_req_handler(|_url, _features| wry::NewWindowResponse::Deny)
            .with_download_started_handler(move |download_url, _path| {
                // Policy is fixed: every download is refused. The observer
                // only records that this exact decision fired — it never
                // chooses the outcome (test-only visibility into a decision
                // that otherwise has no externally observable side effect).
                let allowed = false;
                on_download_decision(download_url, allowed);
                allowed
            })
            .with_permission_handler(move |kind| {
                let response = wry::PermissionResponse::Deny;
                on_permission_decision(kind, response);
                response
            })
            .with_on_web_content_process_terminate_handler(move || {
                handle_terminated(&terminate_owner)
            })
            .build_as_child(window)
            .map_err(|error| {
                tracing::warn!(%error, "plugin browser webview construction failed");
                fixed_error(&SessionId(String::new()), 0, "plugin-unavailable")
            })
    }

    /// No-op observers for `build_child`'s test-visibility hooks. Production
    /// callers have nothing to observe; only the test harness
    /// (`tests/browser_native_smoke.rs`) supplies real ones, so it can assert
    /// the actual `with_permission_handler`/`with_download_started_handler`
    /// decision fired, recording only that this exact native decision
    /// occurred.
    fn noop_permission_observer(
    ) -> Arc<dyn Fn(wry::PermissionKind, wry::PermissionResponse) + Send + Sync> {
        Arc::new(|_, _| {})
    }

    fn noop_download_observer() -> Arc<dyn Fn(String, bool) + Send + Sync> {
        Arc::new(|_, _| {})
    }

    pub fn open(
        app: &AppHandle,
        session: SessionId,
        generation: u64,
        url: &str,
        bounds: Bounds,
        admit: Arc<dyn Fn(&str) -> bool + Send + Sync>,
        on_page_load: Arc<dyn Fn(PageLoad, String) + Send + Sync>,
    ) -> Result<(), BrowserError> {
        let Some(window) = app.get_webview_window("main") else {
            return Err(fixed_error(&session, generation, "plugin-unavailable"));
        };
        let armed = Arc::new(AtomicBool::new(false));
        let armed_for_wrapper = Arc::clone(&armed);
        let wrapped_on_page_load: Arc<dyn Fn(PageLoad, String) + Send + Sync> =
            Arc::new(move |kind, loaded_url| {
                armed_for_wrapper.store(true, Ordering::Release);
                on_page_load(kind, loaded_url);
            });
        // wry's navigation handler only blocks the candidate URL; it does not
        // itself tell the host a candidate was refused. Without this, an
        // in-page navigation the admission policy rejects (as opposed to a
        // rejection driven by the host's own `navigate` call) fails silently
        // — the URL just never changes.
        let app_for_admit = app.clone();
        let session_for_admit = session.clone();
        let wrapped_admit: Arc<dyn Fn(&str) -> bool + Send + Sync> = Arc::new(move |candidate| {
            let allowed = admit(candidate);
            if !allowed {
                emit_error(
                    &app_for_admit,
                    &session_for_admit,
                    generation,
                    "navigation-denied",
                );
            }
            allowed
        });
        // Created before `build_child` so its identity can scope the
        // content-process termination delegate registered inside it — see
        // `terminate_owner` on `build_child` and `handle_terminated`.
        let poll_active = Arc::new(AtomicBool::new(true));
        let view = build_child(
            &window,
            url,
            bounds,
            wrapped_admit,
            wrapped_on_page_load,
            noop_permission_observer(),
            noop_download_observer(),
            Arc::clone(&poll_active),
        )?;

        // Stop the previous session's poll loop immediately rather than
        // waiting for it to notice the thread-local slot was replaced, and
        // take the previous view out of the slot so its `Drop` (and any
        // delegate callback it might synchronously trigger, e.g.
        // `handle_terminated`) runs after this borrow is released rather
        // than while `RefCell::borrow_mut()` is still held — a callback that
        // re-enters `LIVE_VIEW.with(..).borrow_mut()` mid-drop would
        // otherwise panic on the reentrant borrow.
        let previous = LIVE_VIEW.with(|cell| {
            let mut slot = cell.borrow_mut();
            if let Some(previous) = slot.as_ref() {
                previous.poll_active.store(false, Ordering::Release);
            }
            slot.take()
        });

        spawn_poll_loop(app.clone(), Arc::clone(&poll_active));

        LIVE_VIEW.with(|cell| {
            *cell.borrow_mut() = Some(LiveBrowserView {
                session,
                generation,
                app: app.clone(),
                view,
                poll_active,
                armed,
                last_url: String::new(),
                last_can_go_back: false,
                last_can_go_forward: false,
                terminated: false,
            });
        });
        drop(previous);
        Ok(())
    }

    pub fn navigate(_app: &AppHandle, session: SessionId, url: &str) -> Result<(), BrowserError> {
        with_live(&session, |live| {
            live.view
                .load_url(url)
                .map_err(|_| fixed_error(&live.session, live.generation, "load-failed"))
        })
    }

    pub fn back(_app: &AppHandle, session: SessionId) -> Result<(), BrowserError> {
        with_live(&session, |live| {
            live.view
                .go_back()
                .map_err(|_| fixed_error(&live.session, live.generation, "load-failed"))
        })
    }

    pub fn forward(_app: &AppHandle, session: SessionId) -> Result<(), BrowserError> {
        with_live(&session, |live| {
            live.view
                .go_forward()
                .map_err(|_| fixed_error(&live.session, live.generation, "load-failed"))
        })
    }

    pub fn reload(_app: &AppHandle, session: SessionId) -> Result<(), BrowserError> {
        with_live(&session, |live| {
            live.view
                .reload()
                .map_err(|_| fixed_error(&live.session, live.generation, "load-failed"))
        })
    }

    pub fn set_bounds(
        _app: &AppHandle,
        session: SessionId,
        bounds: Bounds,
    ) -> Result<(), BrowserError> {
        with_live(&session, |live| {
            live.view
                .set_bounds(to_wry_rect(bounds))
                .map_err(|_| fixed_error(&live.session, live.generation, "plugin-unavailable"))
        })
    }

    pub fn set_visible(
        _app: &AppHandle,
        session: SessionId,
        visible: bool,
    ) -> Result<(), BrowserError> {
        with_live(&session, |live| {
            live.view
                .set_visible(visible)
                .map_err(|_| fixed_error(&live.session, live.generation, "plugin-unavailable"))
        })
    }

    pub fn close(_app: &AppHandle, session: SessionId) -> Result<(), BrowserError> {
        // `slot.take()` removes the view before dropping it, so the drop (and
        // any delegate callback it might synchronously trigger, e.g.
        // `handle_terminated`) runs after this borrow is released — see the
        // matching comment in `open`.
        let taken = LIVE_VIEW.with(|cell| {
            let mut slot = cell.borrow_mut();
            match slot.as_ref() {
                Some(live) if live.session == session => {
                    live.poll_active.store(false, Ordering::Release);
                    slot.take()
                }
                _ => None,
            }
        });
        if taken.is_some() {
            drop(taken);
            Ok(())
        } else {
            Err(fixed_error(&session, 0, "plugin-unavailable"))
        }
    }

    pub fn current_url(_app: &AppHandle, session: SessionId) -> Result<String, BrowserError> {
        with_live(&session, |live| {
            catch_unwind(AssertUnwindSafe(|| live.view.url()))
                .ok()
                .and_then(|r| r.ok())
                .ok_or_else(|| fixed_error(&live.session, live.generation, "load-failed"))
        })
    }
}

#[cfg(target_os = "macos")]
pub use macos::{
    back, build_child, close, current_url, forward, navigate, open, reload, set_bounds, set_visible,
};

#[cfg(not(target_os = "macos"))]
mod unsupported {
    use std::sync::Arc;

    use tauri::AppHandle;

    use crate::plugin_host::types::{Bounds, BrowserError, PageLoad, SessionId};

    fn unsupported(session: &SessionId, generation: u64) -> BrowserError {
        BrowserError {
            session_id: session.0.clone(),
            generation,
            code: "unsupported-platform".to_string(),
            message: "This feature isn't available on your platform.".to_string(),
        }
    }

    pub fn open(
        _app: &AppHandle,
        session: SessionId,
        generation: u64,
        _url: &str,
        _bounds: Bounds,
        _admit: Arc<dyn Fn(&str) -> bool + Send + Sync>,
        _on_page_load: Arc<dyn Fn(PageLoad, String) + Send + Sync>,
    ) -> Result<(), BrowserError> {
        Err(unsupported(&session, generation))
    }

    pub fn navigate(_app: &AppHandle, session: SessionId, _url: &str) -> Result<(), BrowserError> {
        Err(unsupported(&session, 0))
    }

    pub fn back(_app: &AppHandle, session: SessionId) -> Result<(), BrowserError> {
        Err(unsupported(&session, 0))
    }

    pub fn forward(_app: &AppHandle, session: SessionId) -> Result<(), BrowserError> {
        Err(unsupported(&session, 0))
    }

    pub fn reload(_app: &AppHandle, session: SessionId) -> Result<(), BrowserError> {
        Err(unsupported(&session, 0))
    }

    pub fn set_bounds(
        _app: &AppHandle,
        session: SessionId,
        _bounds: Bounds,
    ) -> Result<(), BrowserError> {
        Err(unsupported(&session, 0))
    }

    pub fn set_visible(
        _app: &AppHandle,
        session: SessionId,
        _visible: bool,
    ) -> Result<(), BrowserError> {
        Err(unsupported(&session, 0))
    }

    pub fn close(_app: &AppHandle, session: SessionId) -> Result<(), BrowserError> {
        Err(unsupported(&session, 0))
    }

    pub fn current_url(_app: &AppHandle, session: SessionId) -> Result<String, BrowserError> {
        Err(unsupported(&session, 0))
    }
}

#[cfg(not(target_os = "macos"))]
pub use unsupported::{
    back, close, current_url, forward, navigate, open, reload, set_bounds, set_visible,
};

#[cfg(test)]
#[path = "browser_webview_tests.rs"]
mod browser_webview_tests;
