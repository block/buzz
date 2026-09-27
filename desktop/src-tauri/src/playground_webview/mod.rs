//! Playground overlay webviews. Inspect targets `playground-{sid}` only.

pub(crate) mod capture;
pub(crate) mod inspect;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, Url, Webview, WebviewUrl,
};
use uuid::Uuid;

const PLAYGROUND_LABEL_PREFIX: &str = "playground-";
const APP_WEBVIEW_LABEL: &str = "main";
const MIN_EDGE: f64 = 32.0;
const OPENCLAW_GATEWAY_PORT: u16 = 18789;
const BROWSER_DEBUG_PORTS: [u16; 5] = [9222, 9223, 9229, 9230, 5858];
const DOM_HASH_COOKIE: &str = "__buzz_pg_dom";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaygroundBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaygroundProbeResult {
    up: bool,
    status: Option<u16>,
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaygroundInspectResult {
    webview_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaygroundNewTabRequest {
    pub opener_sid: String,
    pub opener_label: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaygroundNavState {
    sid: String,
    can_go_back: bool,
    can_go_forward: bool,
    current_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaygroundPollResult {
    changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaygroundScreenshotResult {
    pub(crate) bytes: Vec<u8>,
    pub(crate) mime: String,
    pub(crate) filename: String,
}

struct PlaygroundSession {
    start_url: Url,
    history: Vec<Url>,
    index: usize,
    programmatic: bool,
    etag: Option<String>,
    last_modified: Option<String>,
    body_hash: Option<String>,
    last_bounds: Option<PlaygroundBounds>,
    /// Frozen at Inspect open; preferred by restore until Inspect closes.
    pre_inspect_bounds: Option<PlaygroundBounds>,
    last_user_agent: Option<String>,
}

impl PlaygroundSession {
    fn new(start_url: Url) -> Self {
        Self {
            start_url: start_url.clone(),
            history: vec![start_url],
            index: 0,
            programmatic: false,
            etag: None,
            last_modified: None,
            body_hash: None,
            last_bounds: None,
            pre_inspect_bounds: None,
            last_user_agent: None,
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

    fn nav_state(&self, sid: &str) -> PlaygroundNavState {
        PlaygroundNavState {
            sid: sid.to_string(),
            can_go_back: self.can_go_back(),
            can_go_forward: self.can_go_forward(),
            current_url: self.current_url().to_string(),
        }
    }
}

#[derive(Default)]
pub struct PlaygroundWebviewManager {
    sessions: Mutex<HashMap<String, PlaygroundSession>>,
}

pub fn is_blocked_playground_port(port: u16) -> bool {
    port == OPENCLAW_GATEWAY_PORT || BROWSER_DEBUG_PORTS.contains(&port)
}

pub fn parse_playground_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|error| format!("invalid url: {error}"))?;
    if url.scheme() != "https" {
        return Err("playground URL must use https".into());
    }
    if url.host_str().is_none() {
        return Err("playground URL must include a host".into());
    }
    if let Some(port) = url.port() {
        if is_blocked_playground_port(port) {
            return Err("playground URL port is not allowed".into());
        }
    }
    Ok(url)
}

pub fn classify_probe_status(status: u16, content_type: Option<&str>, cf_access: bool) -> bool {
    if status == 502 || status == 530 {
        return false;
    }
    if (200..400).contains(&status) {
        return true;
    }
    if status == 401 || status == 403 {
        return cf_access || is_html_content_type(content_type);
    }
    false
}

fn is_html_content_type(content_type: Option<&str>) -> bool {
    content_type
        .unwrap_or("")
        .to_ascii_lowercase()
        .contains("text/html")
}

fn looks_like_cf_access(headers: &reqwest::header::HeaderMap, body: &str) -> bool {
    if headers
        .get("cf-mitigated")
        .and_then(|value| value.to_str().ok())
        .is_some()
    {
        return true;
    }
    if headers
        .get("location")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|location| location.contains("cloudflareaccess.com"))
    {
        return true;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("cloudflare access") || lower.contains("access pin")
}

fn sanitize_sid(sid: &str) -> Result<String, String> {
    if sid.is_empty() || sid.len() > 80 {
        return Err("invalid playground sid".into());
    }
    if !sid
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("invalid playground sid".into());
    }
    Ok(sid.to_string())
}

pub fn playground_label(sid: &str) -> String {
    format!("{PLAYGROUND_LABEL_PREFIX}{sid}")
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

pub fn normalize_window_label(window_label: Option<&str>) -> String {
    let raw = window_label
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(APP_WEBVIEW_LABEL);
    sanitize_window_label(raw)
}

/// One live WKWebView per playground sid. Label is always `playground-{sid}`
/// regardless of which window currently parents it. Stages re-show by
/// reparenting that single child — never spawn `playground-{sid}--{window}`
/// siblings (those forked DOM/Drive phase across pin Open vs detach).
pub fn playground_webview_label(sid: &str, _window_label: &str) -> String {
    playground_label(sid)
}

/// Locate the live playground webview for `sid` (canonical label, or a legacy
/// `playground-{sid}--*` sibling from older builds).
pub(crate) fn find_playground_webview_for_sid(
    app: &AppHandle,
    sid: &str,
) -> Option<(String, Webview)> {
    let canonical = playground_label(sid);
    if let Some(webview) = app.get_webview(&canonical) {
        return Some((canonical, webview));
    }
    let prefix = format!("{}{}--", PLAYGROUND_LABEL_PREFIX, sid);
    for webview in app.webviews().into_values() {
        let label = webview.label().to_string();
        if label.starts_with(&prefix) {
            return Some((label, webview));
        }
    }
    None
}

fn playground_parent_is(
    webview: &Webview,
    window_label: &str,
) -> bool {
    playground_parent_window_label(webview) == window_label
}

/// Move an existing playground child onto `window_label` when needed.
fn reparent_playground_webview(
    app: &AppHandle,
    webview: &Webview,
    window_label: &str,
) -> Result<(), String> {
    if playground_parent_is(webview, window_label) {
        return Ok(());
    }
    let window = app
        .get_window(window_label)
        .ok_or_else(|| format!("{window_label} window is not available"))?;
    webview
        .reparent(&window)
        .map_err(|error| error.to_string())
}

fn playground_parent_window_label(webview: &Webview) -> String {
    webview.window().label().to_string()
}

pub fn inspect_target_is_safe(webview_id: &str) -> bool {
    webview_id.starts_with(PLAYGROUND_LABEL_PREFIX) && webview_id != APP_WEBVIEW_LABEL
}

fn apply_user_agent(webview: &Webview, user_agent: &str) -> Result<(), String> {
    let ua = user_agent.to_string();
    webview
        .with_webview(move |platform| {
            #[cfg(target_os = "macos")]
            {
                use objc2_foundation::NSString;
                use objc2_web_kit::WKWebView;
                // SAFETY: inner() is the child WKWebView for this playground
                // label. setCustomUserAgent is an objc2 unsafe setter; wry
                // uses the same wrapper. Must run on the AppKit main thread,
                // which with_webview already is.
                let view: &WKWebView = unsafe { &*platform.inner().cast::<WKWebView>() };
                unsafe {
                    view.setCustomUserAgent(Some(&NSString::from_str(&ua)));
                }
            }
            #[cfg(target_os = "linux")]
            {
                use webkit2gtk::{SettingsExt, WebViewExt};
                let view = platform.inner();
                if let Some(settings) = WebViewExt::settings(&view) {
                    settings.set_user_agent(Some(ua.as_str()));
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                let _ = platform;
                let _ = ua;
            }
        })
        .map_err(|error| error.to_string())
}

fn sync_user_agent(
    app: &AppHandle,
    manager: &PlaygroundWebviewManager,
    sid: &str,
    window_label: &str,
    user_agent: Option<&str>,
    reload: bool,
) -> Result<(), String> {
    let Some(user_agent) = user_agent.filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let changed = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "playground session lock poisoned".to_string())?;
        let Some(session) = sessions.get_mut(sid) else {
            return Ok(());
        };
        if session.last_user_agent.as_deref() == Some(user_agent) {
            false
        } else {
            session.last_user_agent = Some(user_agent.to_string());
            true
        }
    };
    let Some((_label, webview)) = find_playground_webview_for_sid(app, sid) else {
        return Ok(());
    };
    let _ = window_label;
    if changed {
        apply_user_agent(&webview, user_agent)?;
        if reload {
            webview.reload().map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn bounds_are_usable(bounds: &PlaygroundBounds) -> bool {
    bounds.width >= MIN_EDGE && bounds.height >= MIN_EDGE
}

/// Off-screen keeper park from the TS runtime (`KEEPER_BOUNDS` = -64,-64,64×64).
/// Applying these to an existing WKWebView collapses `innerWidth`/`innerHeight`
/// and breaks Drive/Observe hit-testing ("no element") for the whole grant.
fn is_keeper_park_bounds(bounds: &PlaygroundBounds) -> bool {
    bounds.width <= 64.0 && bounds.height <= 64.0 && bounds.x <= 0.0 && bounds.y <= 0.0
}

/// Remember stage/page-host bounds. Never let a keeper park stomp a prior
/// full-size `last_bounds` — Drive may keep acting on a parked webview.
fn remember_show_bounds(session: &mut PlaygroundSession, bounds: &PlaygroundBounds, show: bool) {
    if session.pre_inspect_bounds.is_some() {
        return;
    }
    if show || !is_keeper_park_bounds(bounds) {
        session.last_bounds = Some(bounds.clone());
        return;
    }
    // Keeper park with no prior size: still record so create/hide bookkeeping
    // has something, but prefer keeping a larger remembered size.
    if session
        .last_bounds
        .as_ref()
        .is_none_or(|prev| is_keeper_park_bounds(prev))
    {
        session.last_bounds = Some(bounds.clone());
    }
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

fn playground_webview_position(
    bounds: &PlaygroundBounds,
    origin: LogicalPosition<f64>,
) -> LogicalPosition<f64> {
    LogicalPosition::new(bounds.x + origin.x, bounds.y + origin.y)
}

fn apply_bounds(
    app: &AppHandle,
    sid: &str,
    window_label: &str,
    bounds: &PlaygroundBounds,
) -> Result<(), String> {
    let Some((_label, webview)) = find_playground_webview_for_sid(app, sid) else {
        return Ok(());
    };
    let origin = app
        .get_window(window_label)
        .map(|window| window_content_origin(&window))
        .unwrap_or(LogicalPosition::new(0.0, 0.0));
    webview
        .set_position(playground_webview_position(bounds, origin))
        .map_err(|error| error.to_string())?;
    webview
        .set_size(LogicalSize::new(
            bounds.width.max(1.0),
            bounds.height.max(1.0),
        ))
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn hide_other_playgrounds(app: &AppHandle, keep_label: &str, window_label: &str) {
    for webview in app.webviews().into_values() {
        let label = webview.label();
        if !label.starts_with(PLAYGROUND_LABEL_PREFIX) || label == keep_label {
            continue;
        }
        if playground_parent_window_label(&webview) != window_label {
            continue;
        }
        let _ = webview.hide();
    }
}

pub(crate) fn playground_sid_from_webview_label(label: &str) -> Option<String> {
    let rest = label.strip_prefix(PLAYGROUND_LABEL_PREFIX)?;
    let sid = rest.split("--").next()?.trim();
    if sid.is_empty() {
        None
    } else {
        Some(sid.to_string())
    }
}

/// Pop-out teardown for playground children parented to `window_label`.
/// Prefer reparenting the single live WKWebView onto main (hidden) so Drive /
/// Observe and DOM phase survive detach → pin Open. Only destroy when main is
/// gone or reparent fails.
pub fn close_playgrounds_for_window(app: &AppHandle, window_label: &str) {
    let window_label = normalize_window_label(Some(window_label));
    if window_label == APP_WEBVIEW_LABEL {
        return;
    }
    let mut destroyed_labels: Vec<String> = Vec::new();
    let mut destroyed_sids: Vec<String> = Vec::new();
    let webviews: Vec<_> = app.webviews().into_values().collect();
    for webview in webviews {
        let label = webview.label().to_string();
        if !label.starts_with(PLAYGROUND_LABEL_PREFIX) {
            continue;
        }
        if playground_parent_window_label(&webview) != window_label {
            continue;
        }
        let Some(sid) = playground_sid_from_webview_label(&label) else {
            continue;
        };
        let parked = if app.get_window(APP_WEBVIEW_LABEL).is_some() {
            match reparent_playground_webview(app, &webview, APP_WEBVIEW_LABEL) {
                Ok(()) => {
                    let _ = webview.hide();
                    true
                }
                Err(error) => {
                    eprintln!(
                        "buzz-desktop: playground reparent to main on pop-out close failed: {error}"
                    );
                    false
                }
            }
        } else {
            false
        };
        if parked {
            crate::browser_agent::ensure_instrumentation_for_label(app, &label);
            continue;
        }
        destroyed_sids.push(sid);
        destroyed_labels.push(label);
        let _ = webview.close();
    }
    for label in &destroyed_labels {
        crate::browser_agent::clear_grant_for_label(app, label);
    }
    for sid in destroyed_sids {
        let still_open = find_playground_webview_for_sid(app, &sid).is_some();
        if !still_open {
            if let Some(manager) = app.try_state::<PlaygroundWebviewManager>() {
                if let Ok(mut sessions) = manager.sessions.lock() {
                    sessions.remove(&sid);
                }
            }
            crate::browser_agent::clear_grants_for_surface(app, &sid);
        }
    }
}

fn data_store_identifier(sid: &str) -> [u8; 16] {
    *Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("buzz-playground:{sid}").as_bytes(),
    )
    .as_bytes()
}

fn emit_nav(app: &AppHandle, state: PlaygroundNavState, webview_label: &str) {
    crate::browser_agent::record_nav_event(app, webview_label, &state.current_url, None);
    if let Err(error) = app.emit("playground-webview-nav", &state) {
        eprintln!("buzz-desktop: playground-webview-nav emit failed: {error}");
    }
}

fn record_navigation(
    app: &AppHandle,
    manager: &PlaygroundWebviewManager,
    sid: &str,
    window_label: &str,
    url: &Url,
) {
    let mut sessions = match manager.sessions.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    let Some(session) = sessions.get_mut(sid) else {
        return;
    };
    let webview_label = playground_webview_label(sid, window_label);
    if session.programmatic {
        session.programmatic = false;
        emit_nav(app, session.nav_state(sid), &webview_label);
        return;
    }
    session.push(url.clone());
    emit_nav(app, session.nav_state(sid), &webview_label);
}

fn remember_bounds(manager: &PlaygroundWebviewManager, sid: &str, bounds: &PlaygroundBounds) {
    if let Ok(mut sessions) = manager.sessions.lock() {
        if let Some(session) = sessions.get_mut(sid) {
            // React stage/page-host is authoritative. While Inspect is open, refresh
            // the clamp target so dock cannot permanently stretch last_bounds and
            // fullscreen settle stays under header chrome.
            if session.pre_inspect_bounds.is_some() {
                session.pre_inspect_bounds = Some(bounds.clone());
            }
            session.last_bounds = Some(bounds.clone());
        }
    }
}

fn hash_response_body(body: &[u8]) -> String {
    hex::encode(Sha256::digest(body))
}

fn navigate_playground(webview: &Webview, url: Url) -> Result<(), String> {
    webview.navigate(url).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn playground_probe(url: String) -> Result<PlaygroundProbeResult, String> {
    let parsed = match parse_playground_url(&url) {
        Ok(value) => value,
        Err(message) => {
            return Ok(PlaygroundProbeResult {
                up: false,
                status: None,
                message: Some(message),
            });
        }
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|error| error.to_string())?;
    let response = match client.get(parsed).send().await {
        Ok(response) => response,
        Err(error) => {
            return Ok(PlaygroundProbeResult {
                up: false,
                status: None,
                message: Some(format!("connect failed: {error}")),
            });
        }
    };
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let headers = response.headers().clone();
    let body = response.text().await.unwrap_or_default();
    let prefix: String = body.chars().take(4000).collect();
    let cf_access = looks_like_cf_access(&headers, &prefix);
    let up = classify_probe_status(status, content_type.as_deref(), cf_access);
    Ok(PlaygroundProbeResult {
        up,
        status: Some(status),
        message: if up {
            None
        } else {
            Some(format!("playground returned {status}"))
        },
    })
}

#[tauri::command]
pub async fn playground_webview_show(
    app: AppHandle,
    manager: State<'_, PlaygroundWebviewManager>,
    sid: String,
    url: String,
    bounds: PlaygroundBounds,
    visible: Option<bool>,
    user_agent: Option<String>,
    window_label: Option<String>,
) -> Result<PlaygroundNavState, String> {
    let sid = sanitize_sid(&sid)?;
    let url = parse_playground_url(&url)?;
    let window_label = normalize_window_label(window_label.as_deref());
    let show = visible.unwrap_or(true);
    let label = playground_webview_label(&sid, &window_label);
    if show {
        hide_other_playgrounds(&app, &label, &window_label);
    }
    let (nav, create_bounds) = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "playground session lock poisoned".to_string())?;
        let session = sessions
            .entry(sid.clone())
            .or_insert_with(|| PlaygroundSession::new(url.clone()));
        if session.start_url.origin() != url.origin() {
            *session = PlaygroundSession::new(url.clone());
        }
        remember_show_bounds(session, &bounds, show);
        // For cold create under keeper park, prefer a remembered full viewport
        // so Drive grants that outlive the React stage stay clickable.
        let create_bounds = if !show && is_keeper_park_bounds(&bounds) {
            session
                .last_bounds
                .clone()
                .filter(|b| !is_keeper_park_bounds(b))
                .unwrap_or_else(|| bounds.clone())
        } else {
            bounds.clone()
        };
        let nav = session.nav_state(&sid);
        (nav, create_bounds)
    };

    // Re-show the one live WKWebView for this sid (reparent across windows).
    // Never create a sibling label — that forked Drive/DOM phase.
    if let Some((live_label, webview)) = find_playground_webview_for_sid(&app, &sid) {
        reparent_playground_webview(&app, &webview, &window_label)?;
        if show || !is_keeper_park_bounds(&bounds) {
            apply_bounds(&app, &sid, &window_label, &bounds)?;
        } else {
            // Hide only — do not collapse to 64×64. If a prior race already
            // shrunk the child, heal back to the last full stage bounds.
            let heal = {
                let sessions = manager
                    .sessions
                    .lock()
                    .map_err(|_| "playground session lock poisoned".to_string())?;
                sessions
                    .get(&sid)
                    .and_then(|s| s.last_bounds.clone())
                    .filter(|b| !is_keeper_park_bounds(b))
            };
            if let Some(full) = heal {
                apply_bounds(&app, &sid, &window_label, &full)?;
            }
        }
        sync_user_agent(
            &app,
            &manager,
            &sid,
            &window_label,
            user_agent.as_deref(),
            true,
        )?;
        if webview.url().ok().as_ref() != Some(&url) && webview.url().ok().is_none() {
            navigate_playground(&webview, url)?;
        }
        if show {
            webview.show().map_err(|error| error.to_string())?;
        } else {
            webview.hide().map_err(|error| error.to_string())?;
        }
        if let Some(state) = app.try_state::<crate::browser_agent::BrowserAgentState>() {
            crate::browser_agent::set_webview_hidden(&app, &state, &live_label, !show);
        }
        crate::browser_agent::ensure_instrumentation_for_label(&app, &live_label);
        emit_nav(&app, nav.clone(), &live_label);
        return Ok(nav);
    }

    let bounds = create_bounds;
    if !bounds_are_usable(&bounds) {
        return Ok(nav);
    }

    let window = app
        .get_window(&window_label)
        .ok_or_else(|| format!("{window_label} window is not available"))?;
    let profile_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("playground-profiles")
        .join(&sid);
    std::fs::create_dir_all(&profile_dir).map_err(|error| error.to_string())?;
    let nav_app = app.clone();
    let nav_sid = sid.clone();
    let nav_window = window_label.clone();
    let load_app = app.clone();
    let load_label = label.clone();
    let mut builder = WebviewBuilder::new(label.clone(), WebviewUrl::External(url))
        .data_directory(profile_dir)
        .data_store_identifier(data_store_identifier(&sid))
        .devtools(true);
    if let Some(user_agent) = user_agent.as_deref().filter(|value| !value.is_empty()) {
        builder = builder.user_agent(user_agent);
    }
    let new_tab_app = app.clone();
    let new_tab_sid = sid.clone();
    let new_tab_label = label.clone();
    let builder = builder
        .on_navigation(move |next| {
            if let Some(manager) = nav_app.try_state::<PlaygroundWebviewManager>() {
                record_navigation(&nav_app, &manager, &nav_sid, &nav_window, next);
            }
            true
        })
        .on_page_load(move |webview, payload| {
            if payload.event() == PageLoadEvent::Finished {
                crate::browser_agent::ensure_instrumentation_for_label(&load_app, &load_label);
                // Main-frame nav + title (deduped). Prefer over raw on_navigation spam.
                crate::browser_agent::record_nav_from_page(&load_app, &load_label);
                // Force a Drive screen after full load so reloads / same-URL
                // transitions still post once ready+quiet settle completes.
                let url = webview
                    .url()
                    .ok()
                    .map(|u| u.to_string())
                    .unwrap_or_default();
                crate::browser_agent::drive_screen::schedule_drive_screen_post(
                    &load_app,
                    &load_label,
                    &url,
                    None,
                    true,
                );
            }
        })
        // MVP B: window.open / target=_blank → frontend sibling tab; deny native popup.
        // Opener may break (window.opener / return value) — documented intentionally.
        .on_new_window(move |url, _features| {
            let payload = PlaygroundNewTabRequest {
                opener_sid: new_tab_sid.clone(),
                opener_label: new_tab_label.clone(),
                url: url.to_string(),
            };
            if let Err(error) = new_tab_app.emit("playground-webview-new-tab", &payload) {
                eprintln!(
                    "buzz-desktop: playground-webview-new-tab emit failed: {error}"
                );
            }
            NewWindowResponse::Deny
        });

    let origin = window_content_origin(&window);
    let webview = window
        .add_child(
            builder,
            playground_webview_position(&bounds, origin),
            LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0)),
        )
        .map_err(|error| error.to_string())?;
    if !show {
        webview.hide().map_err(|error| error.to_string())?;
    }
    if let Some(state) = app.try_state::<crate::browser_agent::BrowserAgentState>() {
        crate::browser_agent::set_webview_hidden(&app, &state, &label, !show);
    }
    sync_user_agent(
        &app,
        &manager,
        &sid,
        &window_label,
        user_agent.as_deref(),
        false,
    )?;
    crate::browser_agent::ensure_instrumentation_for_label(&app, &label);
    emit_nav(&app, nav.clone(), &label);
    Ok(nav)
}

#[tauri::command]
pub async fn playground_webview_hide(
    app: AppHandle,
    sid: String,
    window_label: Option<String>,
) -> Result<(), String> {
    let sid = sanitize_sid(&sid)?;
    let window_label = normalize_window_label(window_label.as_deref());
    // Only hide when this window still parents the live child. After reparent
    // to another host, the abandoning stage must not blank the active view.
    if let Some((label, webview)) = find_playground_webview_for_sid(&app, &sid) {
        if playground_parent_is(&webview, &window_label) {
            webview.hide().map_err(|error| error.to_string())?;
            if let Some(state) = app.try_state::<crate::browser_agent::BrowserAgentState>() {
                crate::browser_agent::set_webview_hidden(&app, &state, &label, true);
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn playground_webview_hide_all(
    app: AppHandle,
    window_label: Option<String>,
) -> Result<(), String> {
    let window_label = normalize_window_label(window_label.as_deref());
    for webview in app.webviews().into_values() {
        if !webview.label().starts_with(PLAYGROUND_LABEL_PREFIX) {
            continue;
        }
        if playground_parent_window_label(&webview) != window_label {
            continue;
        }
        let _ = webview.hide();
        if let Some(state) = app.try_state::<crate::browser_agent::BrowserAgentState>() {
            crate::browser_agent::set_webview_hidden(&app, &state, webview.label(), true);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn playground_webview_set_bounds(
    app: AppHandle,
    manager: State<'_, PlaygroundWebviewManager>,
    sid: String,
    bounds: PlaygroundBounds,
    user_agent: Option<String>,
    window_label: Option<String>,
) -> Result<(), String> {
    let sid = sanitize_sid(&sid)?;
    let window_label = normalize_window_label(window_label.as_deref());
    remember_bounds(&manager, &sid, &bounds);
    apply_bounds(&app, &sid, &window_label, &bounds)?;
    sync_user_agent(
        &app,
        &manager,
        &sid,
        &window_label,
        user_agent.as_deref(),
        true,
    )
}

#[tauri::command]
pub async fn playground_webview_close(
    app: AppHandle,
    manager: State<'_, PlaygroundWebviewManager>,
    sid: String,
    window_label: Option<String>,
) -> Result<(), String> {
    let sid = sanitize_sid(&sid)?;
    let window_label = normalize_window_label(window_label.as_deref());
    // Dispose closes the one live webview for this sid from any host. Hide
    // remains window-scoped; close is session teardown.
    let _ = window_label;
    let closed_label = if let Some((label, webview)) = find_playground_webview_for_sid(&app, &sid)
    {
        webview.close().map_err(|error| error.to_string())?;
        Some(label)
    } else {
        None
    };
    let still_open = find_playground_webview_for_sid(&app, &sid).is_some();
    if !still_open {
        if let Ok(mut sessions) = manager.sessions.lock() {
            sessions.remove(&sid);
        }
        crate::browser_agent::clear_grants_for_surface(&app, &sid);
        if let Some(label) = closed_label {
            crate::browser_agent::clear_grant_for_label(&app, &label);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn playground_webview_close_all(
    app: AppHandle,
    manager: State<'_, PlaygroundWebviewManager>,
    window_label: Option<String>,
) -> Result<(), String> {
    let window_label = normalize_window_label(window_label.as_deref());
    for webview in app.webviews().into_values() {
        if !webview.label().starts_with(PLAYGROUND_LABEL_PREFIX) {
            continue;
        }
        if playground_parent_window_label(&webview) != window_label {
            continue;
        }
        let _ = webview.close();
    }
    let any_playground_left = app
        .webviews()
        .into_values()
        .any(|webview| webview.label().starts_with(PLAYGROUND_LABEL_PREFIX));
    if !any_playground_left {
        if let Ok(mut sessions) = manager.sessions.lock() {
            let sids: Vec<String> = sessions.keys().cloned().collect();
            sessions.clear();
            for sid in sids {
                crate::browser_agent::clear_grants_for_surface(&app, &sid);
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn playground_webview_inspect(
    app: AppHandle,
    manager: State<'_, PlaygroundWebviewManager>,
    sid: String,
    window_label: Option<String>,
) -> Result<PlaygroundInspectResult, String> {
    let sid = sanitize_sid(&sid)?;
    let window_label = normalize_window_label(window_label.as_deref());
    let (webview_id, webview) = find_playground_webview_for_sid(&app, &sid)
        .ok_or_else(|| "playground webview is not open".to_string())?;
    if !inspect_target_is_safe(&webview_id) {
        return Err("inspect must target the playground webview".into());
    }
    if webview_id == APP_WEBVIEW_LABEL {
        return Err("inspect must not target the app webview".into());
    }
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
            .map_err(|_| "playground session lock poisoned".to_string())?;
        let session = sessions.get_mut(&sid);
        let bounds = session.as_ref().and_then(|s| s.last_bounds.clone());
        if let Some(session) = session {
            // Freeze pre-inspect layout so close can hard-restore even if
            // WebKit temporarily stretches the child to the full client.
            if session.pre_inspect_bounds.is_none() {
                session.pre_inspect_bounds = bounds.clone();
            }
        }
        bounds
    };
    // Lock the inspected window *before* show so WebKit cannot bounce the
    // Buzz/pop-out frame. Natural attached inspect (open_devtools). Callers
    // should already be app-wide fullscreen so the dock stays in that surface.
    inspect::lock_main_window_size(&app, &window_label, window_size);
    if let Err(error) = inspect::open_playground_inspector(&webview) {
        inspect::unlock_main_window_size(&app, &window_label);
        return Err(error);
    }
    // Clamp to the stage/page-host immediately so the first dock split stays
    // under header chrome; continuous re-clamp is in schedule_inspect_stage_restore.
    if let Some(bounds) = bounds.as_ref() {
        let keep = inspect::clamp_webview_bounds_to_stage(bounds, bounds);
        let _ = apply_bounds(&app, &sid, &window_label, &keep);
    }
    inspect::schedule_inspect_stage_restore(app.clone(), sid.clone(), window_label.clone());
    Ok(PlaygroundInspectResult { webview_id })
}

#[tauri::command]
pub async fn playground_webview_close_inspect(
    app: AppHandle,
    manager: State<'_, PlaygroundWebviewManager>,
    sid: String,
    window_label: Option<String>,
) -> Result<(), String> {
    let sid = sanitize_sid(&sid)?;
    let window_label = normalize_window_label(window_label.as_deref());
    let Some((webview_id, webview)) = find_playground_webview_for_sid(&app, &sid) else {
        return Ok(());
    };
    if !inspect_target_is_safe(&webview_id) {
        return Err("close inspect must target the playground webview".into());
    }
    inspect::close_playground_inspector(&webview)?;
    inspect::finish_inspect_close(&app, &sid, &window_label);
    let _ = manager;
    Ok(())
}

#[tauri::command]
pub async fn playground_webview_go_back(
    app: AppHandle,
    manager: State<'_, PlaygroundWebviewManager>,
    sid: String,
    window_label: Option<String>,
) -> Result<PlaygroundNavState, String> {
    navigate_history(&app, &manager, &sid, window_label.as_deref(), true)
}

#[tauri::command]
pub async fn playground_webview_go_forward(
    app: AppHandle,
    manager: State<'_, PlaygroundWebviewManager>,
    sid: String,
    window_label: Option<String>,
) -> Result<PlaygroundNavState, String> {
    navigate_history(&app, &manager, &sid, window_label.as_deref(), false)
}

fn navigate_history(
    app: &AppHandle,
    manager: &PlaygroundWebviewManager,
    sid: &str,
    window_label: Option<&str>,
    back: bool,
) -> Result<PlaygroundNavState, String> {
    let sid = sanitize_sid(sid)?;
    let window_label = normalize_window_label(window_label);
    let target = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "playground session lock poisoned".to_string())?;
        let session = sessions
            .get_mut(&sid)
            .ok_or_else(|| "playground is not open".to_string())?;
        let url = if back {
            session.back()
        } else {
            session.forward()
        };
        if url.is_some() {
            session.programmatic = true;
        }
        (url, session.nav_state(&sid))
    };
    if let Some(url) = target.0 {
        if let Some((_label, webview)) = find_playground_webview_for_sid(app, &sid) {
            navigate_playground(&webview, url)?;
        }
    }
    let label = find_playground_webview_for_sid(app, &sid)
        .map(|(label, _)| label)
        .unwrap_or_else(|| playground_webview_label(&sid, &window_label));
    emit_nav(app, target.1.clone(), &label);
    Ok(target.1)
}

#[tauri::command]
pub async fn playground_webview_reload(
    app: AppHandle,
    sid: String,
    window_label: Option<String>,
) -> Result<(), String> {
    let sid = sanitize_sid(&sid)?;
    let _window_label = normalize_window_label(window_label.as_deref());
    if let Some(webview) = find_playground_webview_for_sid(&app, &sid).map(|(_, webview)| webview) {
        webview.reload().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn playground_webview_navigate(
    app: AppHandle,
    manager: State<'_, PlaygroundWebviewManager>,
    sid: String,
    url: String,
    window_label: Option<String>,
) -> Result<PlaygroundNavState, String> {
    let sid = sanitize_sid(&sid)?;
    let window_label = normalize_window_label(window_label.as_deref());
    let url = parse_playground_url(&url)?;
    let nav = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "playground session lock poisoned".to_string())?;
        let session = sessions
            .entry(sid.clone())
            .or_insert_with(|| PlaygroundSession::new(url.clone()));
        session.programmatic = true;
        session.push(url.clone());
        session.nav_state(&sid)
    };
    let label = find_playground_webview_for_sid(&app, &sid)
        .map(|(label, _)| label)
        .unwrap_or_else(|| playground_webview_label(&sid, &window_label));
    if let Some((_label, webview)) = find_playground_webview_for_sid(&app, &sid) {
        navigate_playground(&webview, url)?;
    }
    emit_nav(&app, nav.clone(), &label);
    Ok(nav)
}

#[tauri::command]
pub async fn playground_webview_nav_state(
    manager: State<'_, PlaygroundWebviewManager>,
    sid: String,
    window_label: Option<String>,
) -> Result<PlaygroundNavState, String> {
    let sid = sanitize_sid(&sid)?;
    let _ = window_label;
    let sessions = manager
        .sessions
        .lock()
        .map_err(|_| "playground session lock poisoned".to_string())?;
    Ok(sessions
        .get(&sid)
        .map(|session| session.nav_state(&sid))
        .unwrap_or(PlaygroundNavState {
            sid,
            can_go_back: false,
            can_go_forward: false,
            current_url: String::new(),
        }))
}

#[tauri::command]
pub async fn playground_webview_eval(
    app: AppHandle,
    sid: String,
    js: String,
    window_label: Option<String>,
) -> Result<String, String> {
    let sid = sanitize_sid(&sid)?;
    let window_label = normalize_window_label(window_label.as_deref());
    let _ = window_label;
    let (webview_id, webview) = find_playground_webview_for_sid(&app, &sid)
        .ok_or_else(|| "playground webview is not open".to_string())?;
    if !inspect_target_is_safe(&webview_id) {
        return Err("eval must target the playground webview".into());
    }
    webview.eval(&js).map_err(|error| error.to_string())?;
    Ok(String::new())
}

#[tauri::command]
pub async fn playground_webview_dom_hash(
    app: AppHandle,
    sid: String,
    start_url: String,
    window_label: Option<String>,
) -> Result<String, String> {
    let sid = sanitize_sid(&sid)?;
    let _window_label = normalize_window_label(window_label.as_deref());
    let start_url = parse_playground_url(&start_url)?;
    let Some(webview) = find_playground_webview_for_sid(&app, &sid).map(|(_, webview)| webview) else {
        return Ok(String::new());
    };
    let cookie_url = start_url.clone();
    let cookies = tokio::task::spawn_blocking(move || webview.cookies_for_url(cookie_url))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    Ok(cookies
        .iter()
        .find(|cookie| cookie.name() == DOM_HASH_COOKIE)
        .map(|cookie| cookie.value().to_string())
        .unwrap_or_default())
}

#[tauri::command]
pub async fn playground_webview_poll(
    app: AppHandle,
    manager: State<'_, PlaygroundWebviewManager>,
    sid: String,
    start_url: String,
    window_label: Option<String>,
) -> Result<PlaygroundPollResult, String> {
    let sid = sanitize_sid(&sid)?;
    let _window_label = normalize_window_label(window_label.as_deref());
    let start_url = parse_playground_url(&start_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(start_url.clone());
    if let Some(webview) = find_playground_webview_for_sid(&app, &sid).map(|(_, webview)| webview) {
        let cookie_url = start_url.clone();
        if let Ok(Ok(cookies)) =
            tokio::task::spawn_blocking(move || webview.cookies_for_url(cookie_url)).await
        {
            let cookie_header = cookies
                .iter()
                .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
                .collect::<Vec<_>>()
                .join("; ");
            if !cookie_header.is_empty() {
                request = request.header("Cookie", cookie_header);
            }
        }
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
        .map_err(|_| "playground session lock poisoned".to_string())?;
    let session = sessions
        .entry(sid.clone())
        .or_insert_with(|| PlaygroundSession::new(start_url));
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
    let _ = app;
    Ok(PlaygroundPollResult { changed })
}

#[tauri::command]
pub async fn playground_webview_is_open(
    app: AppHandle,
    sid: String,
    window_label: Option<String>,
) -> Result<bool, String> {
    let sid = sanitize_sid(&sid)?;
    let _ = normalize_window_label(window_label.as_deref());
    // Cold chip is about the logical browser client, not a window sibling.
    Ok(find_playground_webview_for_sid(&app, &sid).is_some())
}

/// `full_page: true` captures the capped full scrollable document (optional API).
/// Default / false = visible WKWebView bounds at native pixel size (Browsers
/// list thumbs and agent/chrome PNG). Do not force snapshotWidth for thumbs.
#[tauri::command]
pub async fn playground_webview_screenshot(
    app: AppHandle,
    manager: State<'_, PlaygroundWebviewManager>,
    sid: String,
    window_label: Option<String>,
    full_page: Option<bool>,
) -> Result<PlaygroundScreenshotResult, String> {
    let sid = sanitize_sid(&sid)?;
    let window_label = normalize_window_label(window_label.as_deref());
    let (webview_id, webview) = find_playground_webview_for_sid(&app, &sid)
        .ok_or_else(|| "playground webview is not open".to_string())?;
    if !inspect_target_is_safe(&webview_id) {
        return Err("screenshot must target the playground webview".into());
    }
    let _ = window_label;
    let _ = manager;
    capture::capture_playground_png(&webview, &webview_id, full_page.unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_http_and_debug_ports() {
        assert!(parse_playground_url("http://app.example.com").is_err());
        assert!(parse_playground_url("https://app.example.com:18789").is_err());
        assert!(parse_playground_url("https://app.example.com:9222").is_err());
        assert!(parse_playground_url("https://app.example.com:9229").is_err());
        assert!(parse_playground_url("https://app.example.com").is_ok());
    }

    #[test]
    fn probe_classifies_access_pin_up_and_bad_gateway_down() {
        assert!(classify_probe_status(200, Some("text/html"), false));
        assert!(classify_probe_status(302, Some("text/html"), false));
        assert!(classify_probe_status(403, Some("text/html"), true));
        assert!(classify_probe_status(403, Some("text/html"), false));
        assert!(!classify_probe_status(502, Some("text/html"), false));
        assert!(!classify_probe_status(530, Some("text/html"), false));
    }

    #[test]
    fn inspect_never_targets_the_app_webview() {
        assert!(inspect_target_is_safe("playground-abc"));
        assert!(inspect_target_is_safe("playground-abc--popout-thread-x"));
        assert!(!inspect_target_is_safe("main"));
        assert_ne!(playground_label("demo"), APP_WEBVIEW_LABEL);
        assert_eq!(playground_webview_label("demo", "main"), "playground-demo");
        assert_eq!(
            playground_webview_label("demo", "popout-thread-x"),
            "playground-demo"
        );
        assert_eq!(
            playground_webview_label("demo", "main"),
            playground_webview_label("demo", "popout-split-abc")
        );
        assert_eq!(normalize_window_label(None), APP_WEBVIEW_LABEL);
        assert_eq!(normalize_window_label(Some("")), APP_WEBVIEW_LABEL);
    }

    #[test]
    fn inspect_does_not_change_main_window_or_stage_size() {
        assert_eq!(
            inspect::resolved_window_size_after_inspect((1280, 800), (1800, 800)),
            (1280, 800)
        );
        assert_eq!(
            inspect::inspect_set_size_after_open((1280, 800), (1800, 800)),
            None
        );
        let before = PlaygroundBounds {
            x: 40.0,
            y: 90.0,
            width: 800.0,
            height: 600.0,
        };
        assert_eq!(
            inspect::resolved_stage_bounds_after_inspect(&before, (1280, 800), (1800, 800)),
            before
        );
        assert_eq!(
            inspect::playground_inspect_presentation(),
            inspect::PlaygroundInspectPresentation::AttachedDocked
        );
    }

    #[test]
    fn screenshot_targets_the_playground_webview_not_screencapture() {
        assert_eq!(capture::PLAYGROUND_CAPTURE_BACKEND, "webview-snapshot");
        assert_ne!(capture::PLAYGROUND_CAPTURE_BACKEND, "screencapture");
        assert_eq!(
            capture::playground_screenshot_target("demo").expect("label"),
            "playground-demo"
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
        let bounds = PlaygroundBounds {
            x: 12.0,
            y: 40.0,
            width: 800.0,
            height: 600.0,
        };
        let positioned = playground_webview_position(&bounds, decorated);
        assert_eq!(positioned.x, 12.0);
        assert_eq!(positioned.y, 68.0);
    }

    #[test]
    fn history_back_and_forward_stay_on_the_playground_webview() {
        let start = Url::parse("https://app.example.com/foo/").expect("url");
        let mut session = PlaygroundSession::new(start);
        session.push(Url::parse("https://app.example.com/foo/bar").expect("url"));
        assert!(session.can_go_back());
        assert_eq!(
            session.back().map(|url| url.to_string()).as_deref(),
            Some("https://app.example.com/foo/")
        );
        assert!(session.can_go_forward());
        assert!(!inspect_target_is_safe("main"));
    }

    #[test]
    fn keeper_park_bounds_match_runtime_64_square() {
        let keeper = PlaygroundBounds {
            x: -64.0,
            y: -64.0,
            width: 64.0,
            height: 64.0,
        };
        assert!(is_keeper_park_bounds(&keeper));
        let stage = PlaygroundBounds {
            x: 80.0,
            y: 120.0,
            width: 1141.0,
            height: 756.0,
        };
        assert!(!is_keeper_park_bounds(&stage));
        assert!(bounds_are_usable(&keeper));
        assert!(bounds_are_usable(&stage));
    }

    #[test]
    fn remember_show_bounds_keeps_full_size_across_keeper_park() {
        let start = Url::parse("https://app.example.com/").expect("url");
        let mut session = PlaygroundSession::new(start);
        let full = PlaygroundBounds {
            x: 40.0,
            y: 90.0,
            width: 1141.0,
            height: 756.0,
        };
        let keeper = PlaygroundBounds {
            x: -64.0,
            y: -64.0,
            width: 64.0,
            height: 64.0,
        };
        remember_show_bounds(&mut session, &full, true);
        assert_eq!(session.last_bounds.as_ref(), Some(&full));
        remember_show_bounds(&mut session, &keeper, false);
        assert_eq!(
            session.last_bounds.as_ref(),
            Some(&full),
            "keeper park must not stomp Drive-usable last_bounds"
        );
    }
}
