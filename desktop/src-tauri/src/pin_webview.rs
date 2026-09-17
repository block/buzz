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
    if let Some(webview) = app.get_webview(&pin_label(pin_id)) {
        let _ = webview.hide();
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

fn apply_bounds(app: &AppHandle, pin_id: &str, bounds: &PinBounds) -> Result<(), String> {
    let Some(webview) = app.get_webview(&pin_label(pin_id)) else {
        return Ok(());
    };
    webview
        .set_position(LogicalPosition::new(bounds.x, bounds.y))
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
    start_url: &Url,
    previous_start: Option<&Url>,
    session: &mut PinSession,
    webview: &Webview,
    bounds: &PinBounds,
) -> Result<PinNavState, String> {
    apply_bounds(app, pin_id, bounds)?;
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
) -> Result<PinNavState, String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    let start_url = parse_https_url(&start_url)?;
    let profile_dir = pin_profile_dir(&app, &pin_id)?;
    std::fs::create_dir_all(&profile_dir).map_err(|error| error.to_string())?;
    let persisted = read_persisted(&profile_dir.join(SESSION_FILE));
    let mut session = PinSession::from_start(start_url.clone(), persisted);
    let (previous_start, last_load_failed) = session_surface(&manager, &pin_id);
    remember_session_flags(&mut session, last_load_failed);
    let initial_url = session.current_url().clone();

    let label = pin_label(&pin_id);
    if let Some(webview) = app.get_webview(&label) {
        // Never close() + add_child on the show path. A tiny first layout only
        // gets set_size here; creation waits for a usable host below.
        let nav = reuse_existing_webview(
            &app,
            &manager,
            &pin_id,
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
        .get_window("main")
        .ok_or_else(|| "main window is not available".to_string())?;
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

    window
        .add_child(
            builder,
            LogicalPosition::new(bounds.x, bounds.y),
            LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0)),
        )
        .map_err(|error| error.to_string())?;

    session.last_load_failed = false;
    let nav = session.nav_state(&pin_id);
    persist_session(&app, &pin_id, &session);
    store_session(&manager, pin_id, session);
    Ok(nav)
}

#[tauri::command]
pub async fn pin_webview_hide(app: AppHandle, pin_id: String) -> Result<(), String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    if let Some(webview) = app.get_webview(&pin_label(&pin_id)) {
        webview.hide().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn pin_webview_hide_all(app: AppHandle) -> Result<(), String> {
    for webview in app.webviews().into_values() {
        if webview.label().starts_with(PIN_LABEL_PREFIX) {
            let _ = webview.hide();
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn pin_webview_set_bounds(
    app: AppHandle,
    pin_id: String,
    bounds: PinBounds,
) -> Result<(), String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    apply_bounds(&app, &pin_id, &bounds)
}

#[tauri::command]
pub async fn pin_webview_go_back(
    app: AppHandle,
    manager: State<'_, PinWebviewManager>,
    pin_id: String,
) -> Result<PinNavState, String> {
    navigate_history(&app, &manager, &pin_id, true)
}

#[tauri::command]
pub async fn pin_webview_go_forward(
    app: AppHandle,
    manager: State<'_, PinWebviewManager>,
    pin_id: String,
) -> Result<PinNavState, String> {
    navigate_history(&app, &manager, &pin_id, false)
}

fn navigate_history(
    app: &AppHandle,
    manager: &PinWebviewManager,
    pin_id: &str,
    back: bool,
) -> Result<PinNavState, String> {
    let pin_id = sanitize_pin_id(pin_id)?;
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
        if let Some(webview) = app.get_webview(&pin_label(&pin_id)) {
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
) -> Result<(), String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    let start_url = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "pinned site session lock poisoned".to_string())?;
        let Some(session) = sessions.get_mut(&pin_id) else {
            if let Some(webview) = app.get_webview(&pin_label(&pin_id)) {
                webview.reload().map_err(|error| error.to_string())?;
            }
            return Ok(());
        };
        session.programmatic = true;
        session.last_load_failed = false;
        persist_session(&app, &pin_id, session);
        session.start_url.clone()
    };
    if let Some(webview) = app.get_webview(&pin_label(&pin_id)) {
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
) -> Result<(), String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    if let Some(webview) = app.get_webview(&pin_label(&pin_id)) {
        webview.close().map_err(|error| error.to_string())?;
    }
    if let Ok(mut sessions) = manager.sessions.lock() {
        sessions.remove(&pin_id);
    }
    if let Ok(dir) = pin_profile_dir(&app, &pin_id) {
        let _ = std::fs::remove_dir_all(dir);
    }
    Ok(())
}

#[tauri::command]
pub async fn pin_webview_poll(
    app: AppHandle,
    manager: State<'_, PinWebviewManager>,
    pin_id: String,
    start_url: String,
) -> Result<PinPollResult, String> {
    let pin_id = sanitize_pin_id(&pin_id)?;
    let start_url = parse_https_url(&start_url)?;
    let Some(webview) = app.get_webview(&pin_label(&pin_id)) else {
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
}
