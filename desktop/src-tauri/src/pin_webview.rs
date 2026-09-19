//! Embedded per-pin website webviews with isolated persistent profiles.

mod policy;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::webview::{PageLoadEvent, WebviewBuilder};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, Url, Webview, WebviewUrl,
};
use uuid::Uuid;

use policy::{
    bounds_are_usable, classify_pin_load, is_unusable_document_url, should_navigate_existing,
    PinLoadVerdict,
};

const PIN_LABEL_PREFIX: &str = "pin-";
const APP_WEBVIEW_LABEL: &str = "main";
const SESSION_FILE: &str = "session.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinNavState {
    pin_id: String,
    can_go_back: bool,
    can_go_forward: bool,
    current_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinPollResult {
    changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinInspectResult {
    webview_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinScreenshotResult {
    bytes: Vec<u8>,
    mime: String,
    filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinLoadState {
    pin_id: String,
    url: String,
    ok: bool,
    status: Option<u16>,
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedSession {
    last_url: Option<String>,
    history: Vec<String>,
    index: usize,
    etag: Option<String>,
    last_modified: Option<String>,
    body_hash: Option<String>,
}

struct PinSession {
    start_url: Url,
    history: Vec<Url>,
    index: usize,
    programmatic: bool,
    etag: Option<String>,
    last_modified: Option<String>,
    body_hash: Option<String>,
    last_load_failed: bool,
    last_bounds: Option<PinBounds>,
    pre_inspect_bounds: Option<PinBounds>,
}

impl PinSession {
    fn from_start(start_url: Url, persisted: PersistedSession) -> Self {
        let history: Vec<Url> = persisted
            .history
            .iter()
            .filter_map(|entry| Url::parse(entry).ok())
            .collect();
        let history = if history.is_empty() || history[0].origin() != start_url.origin() {
            vec![start_url.clone()]
        } else {
            let mut history = history;
            history[0] = start_url.clone();
            history
        };
        let index = persisted.index.min(history.len().saturating_sub(1));
        Self {
            start_url,
            history,
            index,
            programmatic: false,
            etag: persisted.etag,
            last_modified: persisted.last_modified,
            body_hash: persisted.body_hash,
            last_load_failed: false,
            last_bounds: None,
            pre_inspect_bounds: None,
        }
    }

    fn current_url(&self) -> &Url {
        self.history.get(self.index).unwrap_or(&self.start_url)
    }

    fn can_go_back(&self) -> bool {
        self.index > 0
    }

    fn can_go_forward(&self) -> bool {
        self.index + 1 < self.history.len()
    }

    fn push(&mut self, url: Url) {
        if self.history.get(self.index) == Some(&url) {
            return;
        }
        self.history.truncate(self.index + 1);
        self.history.push(url);
        self.index = self.history.len() - 1;
    }

    fn back(&mut self) -> Option<Url> {
        if self.index == 0 {
            return None;
        }
        self.index -= 1;
        self.history.get(self.index).cloned()
    }

    fn forward(&mut self) -> Option<Url> {
        if self.index + 1 >= self.history.len() {
            return None;
        }
        self.index += 1;
        self.history.get(self.index).cloned()
    }

    fn persist(&self) -> PersistedSession {
        PersistedSession {
            last_url: Some(self.current_url().to_string()),
            history: self.history.iter().map(ToString::to_string).collect(),
            index: self.index,
            etag: self.etag.clone(),
            last_modified: self.last_modified.clone(),
            body_hash: self.body_hash.clone(),
        }
    }

    fn nav_state(&self, pin_id: &str) -> PinNavState {
        PinNavState {
            pin_id: pin_id.to_string(),
            can_go_back: self.can_go_back(),
            can_go_forward: self.can_go_forward(),
            current_url: self.current_url().to_string(),
        }
    }
}

#[derive(Default)]
pub struct PinWebviewManager {
    sessions: Mutex<HashMap<String, PinSession>>,
}

fn sanitize_pin_id(pin_id: &str) -> Result<String, String> {
    if pin_id.is_empty() || pin_id.len() > 80 {
        return Err("invalid pin id".into());
    }
    if !pin_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("invalid pin id".into());
    }
    Ok(pin_id.to_string())
}

fn pin_label(pin_id: &str) -> String {
    format!("{PIN_LABEL_PREFIX}{pin_id}")
}

fn sanitize_window_label(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('-');
    if cleaned.is_empty() {
        APP_WEBVIEW_LABEL.to_string()
    } else {
        cleaned.chars().take(80).collect()
    }
}

fn normalize_window_label(window_label: Option<&str>) -> String {
    let raw = window_label
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(APP_WEBVIEW_LABEL);
    sanitize_window_label(raw)
}

/// Main-window labels stay `pin-{id}` so existing pins keep working.
/// Other parents use `pin-{id}--{window}` so the same pin can exist on main
/// and a thread/split pop-out without reparenting WKWebView.
fn pin_inspect_target_is_safe(webview_id: &str) -> bool {
    webview_id.starts_with(PIN_LABEL_PREFIX) && webview_id != APP_WEBVIEW_LABEL
}

fn pin_webview_label(pin_id: &str, window_label: &str) -> String {
    let window_label = normalize_window_label(Some(window_label));
    if window_label == APP_WEBVIEW_LABEL {
        pin_label(pin_id)
    } else {
        format!("{PIN_LABEL_PREFIX}{pin_id}--{window_label}")
    }
}

fn pin_parent_window_label(webview: &Webview) -> String {
    webview.window().label().to_string()
}

/// Logical offset of the HTML content view inside the native window frame.
/// `getBoundingClientRect()` is content-relative; `add_child` / `set_position`
/// are frame-relative. Decorated pop-outs have a titlebar (~28px on macOS);
/// overlay titlebars report inner == outer so this is (0, 0).
fn window_content_origin(window: &tauri::Window) -> LogicalPosition<f64> {
    let scale = window.scale_factor().unwrap_or(1.0);
    let Ok(outer) = window.outer_position() else {
        return LogicalPosition::new(0.0, 0.0);
    };
    let Ok(inner) = window.inner_position() else {
        return LogicalPosition::new(0.0, 0.0);
    };
    content_origin_from_inner_outer(inner.x, inner.y, outer.x, outer.y, scale)
}

fn content_origin_from_inner_outer(
    inner_x: i32,
    inner_y: i32,
    outer_x: i32,
    outer_y: i32,
    scale: f64,
) -> LogicalPosition<f64> {
    LogicalPosition::new(
        (inner_x - outer_x) as f64 / scale,
        (inner_y - outer_y) as f64 / scale,
    )
}

fn pin_webview_position(
    bounds: &PinBounds,
    origin: LogicalPosition<f64>,
) -> LogicalPosition<f64> {
    LogicalPosition::new(bounds.x + origin.x, bounds.y + origin.y)
}

/// Close pin child webviews parented to `window_label` (pop-out teardown).
pub fn close_pins_for_window(app: &AppHandle, window_label: &str) {
    let window_label = normalize_window_label(Some(window_label));
    for webview in app.webviews().into_values() {
        if !webview.label().starts_with(PIN_LABEL_PREFIX) {
            continue;
        }
        if pin_parent_window_label(&webview) != window_label {
            continue;
        }
        let _ = webview.close();
    }
}

fn pin_still_open_anywhere(app: &AppHandle, pin_id: &str) -> bool {
    let main_label = pin_label(pin_id);
    let prefix = format!("{PIN_LABEL_PREFIX}{pin_id}--");
    app.webviews().into_values().any(|webview| {
        let label = webview.label();
        label == main_label || label.starts_with(&prefix)
    })
}

fn pin_profile_dir(app: &AppHandle, pin_id: &str) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("pin-profiles")
        .join(pin_id))
}

fn pin_data_store_identifier(pin_id: &str) -> [u8; 16] {
    *Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("buzz-pin:{pin_id}").as_bytes(),
    )
    .as_bytes()
}

fn parse_https_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|error| format!("invalid url: {error}"))?;
    if url.scheme() != "https" {
        return Err("pin URL must use https".into());
    }
    if url.host_str().is_none() {
        return Err("pin URL must include a host".into());
    }
    Ok(url)
}

fn read_persisted(path: &PathBuf) -> PersistedSession {
    let Ok(bytes) = std::fs::read(path) else {
        return PersistedSession::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn write_persisted(path: &PathBuf, session: &PersistedSession) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(session).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

fn persist_session(app: &AppHandle, pin_id: &str, session: &PinSession) {
    let path = match pin_profile_dir(app, pin_id) {
        Ok(dir) => dir.join(SESSION_FILE),
        Err(_) => return,
    };
    let _ = write_persisted(&path, &session.persist());
}

fn hash_response_body(body: &[u8]) -> String {
    hex::encode(Sha256::digest(body))
}

fn emit_nav(app: &AppHandle, state: PinNavState) {
    if let Err(error) = app.emit("pin-webview-nav", state) {
        eprintln!("buzz-desktop: pin-webview-nav emit failed: {error}");
    }
}

fn emit_load(app: &AppHandle, state: PinLoadState) {
    if let Err(error) = app.emit("pin-webview-load", state) {
        eprintln!("buzz-desktop: pin-webview-load emit failed: {error}");
    }
}

fn session_surface(manager: &PinWebviewManager, pin_id: &str) -> (Option<Url>, bool) {
    match manager.sessions.lock() {
        Ok(sessions) => sessions
            .get(pin_id)
            .map(|session| (Some(session.start_url.clone()), session.last_load_failed))
            .unwrap_or((None, false)),
        Err(_) => (None, false),
    }
}

fn remember_session_flags(session: &mut PinSession, last_load_failed: bool) {
    session.last_load_failed = last_load_failed;
}

fn store_session(manager: &PinWebviewManager, pin_id: String, session: PinSession) {
    if let Ok(mut sessions) = manager.sessions.lock() {
        sessions.insert(pin_id, session);
    }
}

fn mark_load_ok(app: &AppHandle, manager: &PinWebviewManager, pin_id: &str, url: &Url) {
    if let Ok(mut sessions) = manager.sessions.lock() {
        if let Some(session) = sessions.get_mut(pin_id) {
            session.last_load_failed = false;
        }
    }
    emit_load(
        app,
        PinLoadState {
            pin_id: pin_id.to_string(),
            url: url.to_string(),
            ok: true,
            status: None,
            message: None,
        },
    );
}

fn mark_load_failed(
    app: &AppHandle,
    manager: &PinWebviewManager,
    pin_id: &str,
    url: &Url,
    status: Option<u16>,
    message: &str,
) {
    if let Ok(mut sessions) = manager.sessions.lock() {
        if let Some(session) = sessions.get_mut(pin_id) {
            session.last_load_failed = true;
        }
    }
    let main_label = pin_label(pin_id);
    let prefix = format!("{PIN_LABEL_PREFIX}{pin_id}--");
    for webview in app.webviews().into_values() {
        let label = webview.label();
        if label == main_label || label.starts_with(&prefix) {
            let _ = webview.hide();
        }
    }
    emit_load(
        app,
        PinLoadState {
            pin_id: pin_id.to_string(),
            url: url.to_string(),
            ok: false,
            status,
            message: Some(message.to_string()),
        },
    );
}

fn apply_load_verdict(
    app: &AppHandle,
    manager: &PinWebviewManager,
    pin_id: &str,
    url: &Url,
    verdict: PinLoadVerdict,
) {
    match verdict {
        PinLoadVerdict::Ok => mark_load_ok(app, manager, pin_id, url),
        PinLoadVerdict::Failed { message, status } => {
            mark_load_failed(app, manager, pin_id, url, status, &message);
        }
    }
}

/// Never evaluate JavaScript on a pin webview. Blank/failed loads are
/// classified from the navigation URL or a failed `navigate` Result.
fn classify_from_url(url: Option<&Url>) -> PinLoadVerdict {
    classify_pin_load(url)
}

fn navigate_pin_webview(
    app: &AppHandle,
    manager: &PinWebviewManager,
    pin_id: &str,
    webview: &Webview,
    url: Url,
) -> Result<(), String> {
    match webview.navigate(url.clone()) {
        Ok(()) => Ok(()),
        Err(error) => {
            mark_load_failed(app, manager, pin_id, &url, None, "The page did not load.");
            Err(error.to_string())
        }
    }
}

fn remember_bounds(manager: &PinWebviewManager, pin_id: &str, bounds: &PinBounds) {
    if let Ok(mut sessions) = manager.sessions.lock() {
        if let Some(session) = sessions.get_mut(pin_id) {
            // React page-host is authoritative. While Inspect is open, refresh the
            // clamp target so fullscreen settle / host resize stay under header.
            if session.pre_inspect_bounds.is_some() {
                session.pre_inspect_bounds = Some(bounds.clone());
            }
            session.last_bounds = Some(bounds.clone());
        }
    }
}

fn apply_bounds(
    app: &AppHandle,
    pin_id: &str,
    window_label: &str,
    bounds: &PinBounds,
) -> Result<(), String> {
    let Some(webview) = app.get_webview(&pin_webview_label(pin_id, window_label)) else {
        return Ok(());
    };
    let origin = app
        .get_window(window_label)
        .map(|window| window_content_origin(&window))
        .unwrap_or(LogicalPosition::new(0.0, 0.0));
    webview
        .set_position(pin_webview_position(bounds, origin))
        .map_err(|error| error.to_string())?;
    webview
        .set_size(LogicalSize::new(
            bounds.width.max(1.0),
            bounds.height.max(1.0),
        ))
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn record_navigation(app: &AppHandle, manager: &PinWebviewManager, pin_id: &str, url: &Url) {
    let mut sessions = match manager.sessions.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    let Some(session) = sessions.get_mut(pin_id) else {
        return;
    };
    if session.programmatic {
        session.programmatic = false;
        emit_nav(app, session.nav_state(pin_id));
        persist_session(app, pin_id, session);
        return;
    }
    session.push(url.clone());
    emit_nav(app, session.nav_state(pin_id));
    persist_session(app, pin_id, session);
}

fn reuse_existing_webview(
    app: &AppHandle,
    manager: &PinWebviewManager,
    pin_id: &str,
    window_label: &str,
    start_url: &Url,
    previous_start: Option<&Url>,
    session: &mut PinSession,
    webview: &Webview,
    bounds: &PinBounds,
) -> Result<PinNavState, String> {
    apply_bounds(app, pin_id, window_label, bounds)?;
    if session.pre_inspect_bounds.is_none() {
        session.last_bounds = Some(bounds.clone());
    }
    remember_bounds(manager, pin_id, bounds);
    let current = webview.url().ok();
    if should_navigate_existing(
        current.as_ref(),
        start_url,
        previous_start,
        session.last_load_failed,
    ) {
        session.programmatic = true;
        session.last_load_failed = false;
        navigate_pin_webview(app, manager, pin_id, webview, start_url.clone())?;
    } else if let Some(url) = current.as_ref() {
        apply_load_verdict(app, manager, pin_id, url, classify_from_url(Some(url)));
    }
    webview.show().map_err(|error| error.to_string())?;
    Ok(session.nav_state(pin_id))
}

#[tauri::command]
pub async fn pin_webview_show(
    app: AppHandle,
    manager: State<'_, PinWebviewManager>,
    pin_id: String,
    start_url: String,
    bounds: PinBounds,
    window_label: Option<String>,
) -> Result<PinNavState, String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    let start_url = parse_https_url(&start_url)?;
    let window_label = normalize_window_label(window_label.as_deref());
    let profile_dir = pin_profile_dir(&app, &pin_id)?;
    std::fs::create_dir_all(&profile_dir).map_err(|error| error.to_string())?;
    let persisted = read_persisted(&profile_dir.join(SESSION_FILE));
    let mut session = PinSession::from_start(start_url.clone(), persisted);
    let (previous_start, last_load_failed) = session_surface(&manager, &pin_id);
    remember_session_flags(&mut session, last_load_failed);
    let initial_url = session.current_url().clone();

    let label = pin_webview_label(&pin_id, &window_label);
    if let Some(webview) = app.get_webview(&label) {
        // Never close() + add_child on the show path. A tiny first layout only
        // gets set_size here; creation waits for a usable host below.
        let nav = reuse_existing_webview(
            &app,
            &manager,
            &pin_id,
            &window_label,
            &start_url,
            previous_start.as_ref(),
            &mut session,
            &webview,
            &bounds,
        )?;
        persist_session(&app, &pin_id, &session);
        store_session(&manager, pin_id, session);
        return Ok(nav);
    }

    if !bounds_are_usable(bounds.width, bounds.height) {
        let nav = session.nav_state(&pin_id);
        store_session(&manager, pin_id, session);
        return Ok(nav);
    }

    let window = app
        .get_window(&window_label)
        .ok_or_else(|| format!("{window_label} window is not available"))?;
    let nav_app = app.clone();
    let nav_pin = pin_id.clone();
    let load_app = app.clone();
    let load_pin = pin_id.clone();
    let builder = WebviewBuilder::new(label, WebviewUrl::External(initial_url))
        .data_directory(profile_dir.clone())
        .data_store_identifier(pin_data_store_identifier(&pin_id))
        .on_navigation(move |url| {
            if let Some(manager) = nav_app.try_state::<PinWebviewManager>() {
                record_navigation(&nav_app, &manager, &nav_pin, url);
            }
            true
        })
        .on_page_load(move |_webview, payload| match payload.event() {
            PageLoadEvent::Started => {
                if !is_unusable_document_url(Some(payload.url())) {
                    if let Some(manager) = load_app.try_state::<PinWebviewManager>() {
                        mark_load_ok(&load_app, &manager, &load_pin, payload.url());
                    }
                }
            }
            PageLoadEvent::Finished => {
                if let Some(manager) = load_app.try_state::<PinWebviewManager>() {
                    apply_load_verdict(
                        &load_app,
                        &manager,
                        &load_pin,
                        payload.url(),
                        classify_from_url(Some(payload.url())),
                    );
                }
            }
        });

    let origin = window_content_origin(&window);
    window
        .add_child(
            builder,
            pin_webview_position(&bounds, origin),
            LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0)),
        )
        .map_err(|error| error.to_string())?;

    session.last_load_failed = false;
    if session.pre_inspect_bounds.is_none() {
        session.last_bounds = Some(bounds.clone());
    }
    let nav = session.nav_state(&pin_id);
    persist_session(&app, &pin_id, &session);
    store_session(&manager, pin_id, session);
    Ok(nav)
}

#[tauri::command]
pub async fn pin_webview_hide(
    app: AppHandle,
    pin_id: String,
    window_label: Option<String>,
) -> Result<(), String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    let window_label = normalize_window_label(window_label.as_deref());
    if let Some(webview) = app.get_webview(&pin_webview_label(&pin_id, &window_label)) {
        webview.hide().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn pin_webview_hide_all(
    app: AppHandle,
    window_label: Option<String>,
) -> Result<(), String> {
    let window_label = normalize_window_label(window_label.as_deref());
    for webview in app.webviews().into_values() {
        if !webview.label().starts_with(PIN_LABEL_PREFIX) {
            continue;
        }
        if pin_parent_window_label(&webview) != window_label {
            continue;
        }
        let _ = webview.hide();
    }
    Ok(())
}

#[tauri::command]
pub async fn pin_webview_set_bounds(
    app: AppHandle,
    manager: State<'_, PinWebviewManager>,
    pin_id: String,
    bounds: PinBounds,
    window_label: Option<String>,
) -> Result<(), String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    let window_label = normalize_window_label(window_label.as_deref());
    apply_bounds(&app, &pin_id, &window_label, &bounds)?;
    remember_bounds(&manager, &pin_id, &bounds);
    Ok(())
}

#[tauri::command]
pub async fn pin_webview_go_back(
    app: AppHandle,
    manager: State<'_, PinWebviewManager>,
    pin_id: String,
    window_label: Option<String>,
) -> Result<PinNavState, String> {
    navigate_history(&app, &manager, &pin_id, window_label.as_deref(), true)
}

#[tauri::command]
pub async fn pin_webview_go_forward(
    app: AppHandle,
    manager: State<'_, PinWebviewManager>,
    pin_id: String,
    window_label: Option<String>,
) -> Result<PinNavState, String> {
    navigate_history(&app, &manager, &pin_id, window_label.as_deref(), false)
}

fn navigate_history(
    app: &AppHandle,
    manager: &PinWebviewManager,
    pin_id: &str,
    window_label: Option<&str>,
    back: bool,
) -> Result<PinNavState, String> {
    let pin_id = sanitize_pin_id(pin_id)?;
    let window_label = normalize_window_label(window_label);
    let target = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "pinned site session lock poisoned".to_string())?;
        let session = sessions
            .get_mut(&pin_id)
            .ok_or_else(|| "pinned site is not open".to_string())?;
        let url = if back {
            session.back()
        } else {
            session.forward()
        };
        if url.is_some() {
            session.programmatic = true;
        }
        let nav = session.nav_state(&pin_id);
        persist_session(app, &pin_id, session);
        (url, nav)
    };
    if let Some(url) = target.0 {
        if let Some(webview) = app.get_webview(&pin_webview_label(&pin_id, &window_label)) {
            navigate_pin_webview(app, manager, &pin_id, &webview, url)?;
        }
    }
    emit_nav(app, target.1.clone());
    Ok(target.1)
}

#[tauri::command]
pub async fn pin_webview_reload(
    app: AppHandle,
    manager: State<'_, PinWebviewManager>,
    pin_id: String,
    window_label: Option<String>,
) -> Result<(), String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    let window_label = normalize_window_label(window_label.as_deref());
    let label = pin_webview_label(&pin_id, &window_label);
    let start_url = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "pinned site session lock poisoned".to_string())?;
        let Some(session) = sessions.get_mut(&pin_id) else {
            if let Some(webview) = app.get_webview(&label) {
                webview.reload().map_err(|error| error.to_string())?;
            }
            return Ok(());
        };
        session.programmatic = true;
        session.last_load_failed = false;
        persist_session(&app, &pin_id, session);
        session.start_url.clone()
    };
    if let Some(webview) = app.get_webview(&label) {
        webview.show().map_err(|error| error.to_string())?;
        navigate_pin_webview(&app, &manager, &pin_id, &webview, start_url)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn pin_webview_nav_state(
    manager: State<'_, PinWebviewManager>,
    pin_id: String,
) -> Result<PinNavState, String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    let sessions = manager
        .sessions
        .lock()
        .map_err(|_| "pinned site session lock poisoned".to_string())?;
    Ok(sessions
        .get(&pin_id)
        .map(|session| session.nav_state(&pin_id))
        .unwrap_or(PinNavState {
            pin_id,
            can_go_back: false,
            can_go_forward: false,
            current_url: String::new(),
        }))
}

#[tauri::command]
pub async fn pin_webview_close(
    app: AppHandle,
    manager: State<'_, PinWebviewManager>,
    pin_id: String,
    window_label: Option<String>,
) -> Result<(), String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    let window_label = normalize_window_label(window_label.as_deref());
    if let Some(webview) = app.get_webview(&pin_webview_label(&pin_id, &window_label)) {
        webview.close().map_err(|error| error.to_string())?;
    }
    if !pin_still_open_anywhere(&app, &pin_id) {
        if let Ok(mut sessions) = manager.sessions.lock() {
            sessions.remove(&pin_id);
        }
        if let Ok(dir) = pin_profile_dir(&app, &pin_id) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn pin_webview_poll(
    app: AppHandle,
    manager: State<'_, PinWebviewManager>,
    pin_id: String,
    start_url: String,
    window_label: Option<String>,
) -> Result<PinPollResult, String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    let start_url = parse_https_url(&start_url)?;
    let window_label = normalize_window_label(window_label.as_deref());
    let Some(webview) = app.get_webview(&pin_webview_label(&pin_id, &window_label)) else {
        return Ok(PinPollResult { changed: false });
    };
    let cookie_url = start_url.clone();
    let cookies = tokio::task::spawn_blocking(move || webview.cookies_for_url(cookie_url))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let cookie_header = cookies
        .iter()
        .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
        .collect::<Vec<_>>()
        .join("; ");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(start_url.clone());
    if !cookie_header.is_empty() {
        request = request.header("Cookie", cookie_header);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let last_modified = response
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let body = response.bytes().await.map_err(|error| error.to_string())?;
    let body_hash = hash_response_body(&body);

    let mut sessions = manager
        .sessions
        .lock()
        .map_err(|_| "pinned site session lock poisoned".to_string())?;
    let session = sessions
        .entry(pin_id.clone())
        .or_insert_with(|| PinSession::from_start(start_url, PersistedSession::default()));
    let had_snapshot =
        session.etag.is_some() || session.last_modified.is_some() || session.body_hash.is_some();
    let differs = if let Some(etag) = etag.as_ref() {
        session.etag.as_ref() != Some(etag)
    } else if let Some(last_modified) = last_modified.as_ref() {
        session.last_modified.as_ref() != Some(last_modified)
    } else {
        session.body_hash.as_ref() != Some(&body_hash)
    };
    let changed = had_snapshot && differs;
    session.etag = etag;
    session.last_modified = last_modified;
    session.body_hash = Some(body_hash);
    persist_session(&app, &pin_id, session);
    Ok(PinPollResult { changed })
}


#[tauri::command]
pub async fn pin_webview_inspect(
    app: AppHandle,
    manager: State<'_, PinWebviewManager>,
    pin_id: String,
    window_label: Option<String>,
) -> Result<PinInspectResult, String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    let window_label = normalize_window_label(window_label.as_deref());
    let webview_id = pin_webview_label(&pin_id, &window_label);
    if !pin_inspect_target_is_safe(&webview_id) {
        return Err("inspect must target a pin webview".into());
    }
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "pin webview is not open".to_string())?;
    let window_size = app.get_window(&window_label).and_then(|window| {
        window
            .inner_size()
            .ok()
            .map(|size| (size.width, size.height))
    });
    let bounds = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "pinned site session lock poisoned".to_string())?;
        let session = sessions.get_mut(&pin_id);
        let bounds = session.as_ref().and_then(|s| s.last_bounds.clone());
        if let Some(session) = session {
            if session.pre_inspect_bounds.is_none() {
                session.pre_inspect_bounds = bounds.clone();
            }
        }
        bounds
    };
    // Detached WebKit inspector only. Docked open_devtools reparents the pin
    // WKWebView into a split and breaks mobile/device-bezel framing in link
    // pop-outs (and still crowds slide-out chrome). Keep bounds locked as a
    // belt-and-suspenders clamp if WebKit briefly attaches.
    crate::playground_webview::inspect::lock_main_window_size(&app, &window_label, window_size);
    if let Err(error) = crate::playground_webview::inspect::open_detached_inspector(&webview) {
        crate::playground_webview::inspect::unlock_main_window_size(&app, &window_label);
        return Err(error);
    }
    crate::playground_webview::inspect::redetach_inspector_for_webview(&webview);
    if let Some(bounds) = bounds.as_ref() {
        let _ = apply_bounds(&app, &pin_id, &window_label, bounds);
    }
    schedule_pin_inspect_bounds_restore(app.clone(), pin_id.clone(), window_label.clone());
    Ok(PinInspectResult { webview_id })
}

#[tauri::command]
pub async fn pin_webview_close_inspect(
    app: AppHandle,
    manager: State<'_, PinWebviewManager>,
    pin_id: String,
    window_label: Option<String>,
) -> Result<(), String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    let window_label = normalize_window_label(window_label.as_deref());
    let webview_id = pin_webview_label(&pin_id, &window_label);
    if !pin_inspect_target_is_safe(&webview_id) {
        return Err("close inspect must target a pin webview".into());
    }
    let Some(webview) = app.get_webview(&webview_id) else {
        // Already gone — still clear pre-inspect + unlock for parity.
        clear_pin_pre_inspect_bounds(&app, &pin_id);
        crate::playground_webview::inspect::unlock_main_window_size(&app, &window_label);
        let _ = manager;
        return Ok(());
    };
    crate::playground_webview::inspect::close_playground_inspector(&webview)?;
    // Restore frozen bounds + unlock (same path as natural inspect-close).
    for delay_ms in [0_u64, 16, 50, 200] {
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        reapply_pin_last_bounds(&app, &pin_id, &window_label);
    }
    clear_pin_pre_inspect_bounds(&app, &pin_id);
    crate::playground_webview::inspect::unlock_main_window_size(&app, &window_label);
    let _ = manager;
    Ok(())
}

fn schedule_pin_inspect_bounds_restore(app: AppHandle, pin_id: String, window_label: String) {
    tauri::async_runtime::spawn(async move {
        // Keep the pin webview clamped to the React page-host while Inspect is
        // open so a docked inspector cannot grow over URL / mode chrome.
        let mut saw_visible = false;
        let mut waiting_ticks = 0_u32;
        for delay_ms in [0_u64, 16, 50, 100, 200] {
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            reapply_pin_last_bounds(&app, &pin_id, &window_label);
        }
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if pin_inspector_is_visible(&app, &pin_id, &window_label) {
                saw_visible = true;
                if let Some(wv) = app.get_webview(&pin_webview_label(&pin_id, &window_label)) {
                    crate::playground_webview::inspect::redetach_inspector_for_webview(&wv);
                }
                reapply_pin_last_bounds(&app, &pin_id, &window_label);
                continue;
            }
            if !saw_visible {
                waiting_ticks += 1;
                reapply_pin_last_bounds(&app, &pin_id, &window_label);
                if waiting_ticks < 8 {
                    continue;
                }
                clear_pin_pre_inspect_bounds(&app, &pin_id);
                crate::playground_webview::inspect::unlock_main_window_size(&app, &window_label);
                break;
            }
            for delay_ms in [0_u64, 16, 50, 200, 500, 1000] {
                if delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
                reapply_pin_last_bounds(&app, &pin_id, &window_label);
            }
            clear_pin_pre_inspect_bounds(&app, &pin_id);
            crate::playground_webview::inspect::unlock_main_window_size(&app, &window_label);
            break;
        }
    });
}

fn pin_inspector_is_visible(app: &AppHandle, pin_id: &str, window_label: &str) -> bool {
    let Some(webview) = app.get_webview(&pin_webview_label(pin_id, window_label)) else {
        return false;
    };
    crate::playground_webview::inspect::inspector_is_visible_for_webview(&webview)
}

fn clear_pin_pre_inspect_bounds(app: &AppHandle, pin_id: &str) {
    let Some(manager) = app.try_state::<PinWebviewManager>() else {
        return;
    };
    if let Ok(mut sessions) = manager.sessions.lock() {
        if let Some(session) = sessions.get_mut(pin_id) {
            session.pre_inspect_bounds = None;
        }
    };
}

fn reapply_pin_last_bounds(app: &AppHandle, pin_id: &str, window_label: &str) {
    let Some(manager) = app.try_state::<PinWebviewManager>() else {
        return;
    };
    let bounds = manager.sessions.lock().ok().and_then(|sessions| {
        sessions.get(pin_id).and_then(|session| {
            session
                .pre_inspect_bounds
                .clone()
                .or_else(|| session.last_bounds.clone())
        })
    });
    if let Some(bounds) = bounds.as_ref() {
        let _ = apply_bounds(app, pin_id, window_label, bounds);
    }
}

#[tauri::command]
pub async fn pin_webview_screenshot(
    app: AppHandle,
    pin_id: String,
    window_label: Option<String>,
) -> Result<PinScreenshotResult, String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    let window_label = normalize_window_label(window_label.as_deref());
    let webview_id = pin_webview_label(&pin_id, &window_label);
    if !pin_inspect_target_is_safe(&webview_id) {
        return Err("screenshot must target a pin webview".into());
    }
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "pin webview is not open".to_string())?;
    let bytes = crate::playground_webview::capture::snapshot_child_webview_png(&webview)?;
    let safe_name = pin_id.replace(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-', "_");
    Ok(PinScreenshotResult {
        bytes,
        mime: "image/png".into(),
        filename: format!("pin-{safe_name}.png"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_clamps_back_to_home() {
        let start = Url::parse("https://example.com/").expect("url");
        let mut session = PinSession::from_start(start.clone(), PersistedSession::default());
        session.push(Url::parse("https://example.com/a").expect("url"));
        session.push(Url::parse("https://example.com/b").expect("url"));
        assert!(session.can_go_back());
        assert_eq!(
            session.back().map(|url| url.to_string()).as_deref(),
            Some("https://example.com/a")
        );
        session.back();
        assert!(!session.can_go_back());
        assert!(session.back().is_none());
        assert_eq!(session.current_url().as_str(), "https://example.com/");
        assert!(session.can_go_forward());
    }

    #[test]
    fn sanitize_rejects_path_traversal() {
        assert!(sanitize_pin_id("../etc").is_err());
        assert!(sanitize_pin_id("wayfinder").is_ok());
    }

    #[test]
    fn sanitize_accepts_playground_pin_hyphen_ids() {
        // Conversation playground pins use playground-pin-{sid}. A colon
        // separator fails sanitize and surfaces as Failed to open link.
        assert!(sanitize_pin_id("playground-pin-demo-1").is_ok());
        assert!(sanitize_pin_id("playground-pin:demo-1").is_err());
        assert!(sanitize_pin_id("hula-link-side-panel").is_ok());
    }

    #[test]
    fn window_scoped_labels_stay_pin_id_on_main_and_suffix_elsewhere() {
        assert_eq!(
            pin_webview_label("hula-link-side-panel", "main"),
            "pin-hula-link-side-panel"
        );
        assert_eq!(
            pin_webview_label("playground-pin-demo-1", "main"),
            "pin-playground-pin-demo-1"
        );
        assert_eq!(
            pin_webview_label("hula-link-side-panel", "popout-thread-abc"),
            "pin-hula-link-side-panel--popout-thread-abc"
        );
        assert_eq!(
            normalize_window_label(None),
            "main"
        );
        assert_eq!(
            normalize_window_label(Some("  popout-split-1  ")),
            "popout-split-1"
        );
    }

    #[test]
    fn content_origin_offsets_decorated_titlebar_and_is_zero_for_overlay() {
        let decorated = content_origin_from_inner_outer(0, 28, 0, 0, 1.0);
        assert_eq!(decorated.x, 0.0);
        assert_eq!(decorated.y, 28.0);
        let retina = content_origin_from_inner_outer(0, 56, 0, 0, 2.0);
        assert_eq!(retina.x, 0.0);
        assert_eq!(retina.y, 28.0);
        let overlay = content_origin_from_inner_outer(100, 200, 100, 200, 2.0);
        assert_eq!(overlay.x, 0.0);
        assert_eq!(overlay.y, 0.0);
        let bounds = PinBounds {
            x: 12.0,
            y: 40.0,
            width: 800.0,
            height: 600.0,
        };
        let positioned = pin_webview_position(&bounds, decorated);
        assert_eq!(positioned.x, 12.0);
        assert_eq!(positioned.y, 68.0);
    }

    #[test]
    fn body_hash_is_lowercase_hex_sha256() {
        assert_eq!(
            hash_response_body(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash_response_body(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_ne!(hash_response_body(b"a"), hash_response_body(b"b"));
    }

    #[test]
    fn changing_start_origin_resets_history() {
        let persisted = PersistedSession {
            last_url: Some("https://old.example/page".into()),
            history: vec![
                "https://old.example/".into(),
                "https://old.example/page".into(),
            ],
            index: 1,
            ..PersistedSession::default()
        };
        let session =
            PinSession::from_start(Url::parse("https://new.example/").expect("url"), persisted);
        assert_eq!(session.history.len(), 1);
        assert_eq!(session.current_url().as_str(), "https://new.example/");
        assert!(!session.can_go_back());
    }

    #[test]
    fn pin_inspect_rejects_app_webview_and_accepts_pin_labels() {
        assert!(pin_inspect_target_is_safe("pin-hula-link-side-panel"));
        assert!(pin_inspect_target_is_safe("pin-demo--popout-thread-1"));
        assert!(!pin_inspect_target_is_safe("main"));
        assert!(!pin_inspect_target_is_safe("playground-abc"));
    }
}
