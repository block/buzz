//! Buzz in-app browser Observe/Drive grants (WKWebView playground).
//! Pins are out of scope. Not OpenClaw Chromium / CDP.

pub mod drive;
pub mod drive_screen;
pub mod grant;
pub mod observe;

use std::fs::{create_dir_all, File};
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State, Url};

use crate::playground_webview;
use drive::{
    action_js, ensure_action_id, error_result, navigate_result, parse_page_result, validate_action,
    DriveAction, DriveActionResult, DRIVE_RESULT_COOKIE,
};
use grant::{
    now_ms, pin_label, playground_label, BrowserAgentGrant, BrowserAgentGrantStore,
    BrowserAgentMode, BrowserAgentSurface,
};
use observe::{
    drain_page_queue_js, instrumentation_js, snapshot_to_cookie_js, BrowserObserveBuffer,
    ObserveEvent, PageDrainEvent, SNAPSHOT_COOKIE,
};

#[derive(Default)]
pub struct BrowserAgentState {
    pub grants: BrowserAgentGrantStore,
    pub observe: BrowserObserveBuffer,
    pub drive_screens: drive_screen::DriveScreenTracker,
}

fn ensure_data_root(app: &AppHandle, state: &BrowserAgentState) -> Result<PathBuf, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("browser-agent");
    create_dir_all(&root).map_err(|e| e.to_string())?;
    state.observe.set_root(root.clone());
    Ok(root)
}

fn mirror_grant(root: &PathBuf, grant: Option<&BrowserAgentGrant>, label: &str) {
    let dir = root.join(label);
    let _ = create_dir_all(&dir);
    let path = dir.join("grant.json");
    match grant {
        Some(g) => {
            if let Ok(bytes) = serde_json::to_vec_pretty(g) {
                if let Ok(mut f) = File::create(&path) {
                    let _ = f.write_all(&bytes);
                }
            }
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn emit_grant(app: &AppHandle, grant: Option<&BrowserAgentGrant>, webview_label: &str) {
    let _ = app.emit(
        "browser-agent-grant",
        json!({
            "webviewLabel": webview_label,
            "grant": grant,
        }),
    );
}

fn resolve_label(
    surface: BrowserAgentSurface,
    surface_id: &str,
    window_label: Option<&str>,
) -> Result<String, String> {
    let window = window_label.unwrap_or("main").trim();
    let window = if window.is_empty() { "main" } else { window };
    match surface {
        BrowserAgentSurface::Playground => {
            let sid = sanitize_id(surface_id, "sid")?;
            Ok(playground_label(&sid, window))
        }
        BrowserAgentSurface::Pin => {
            let pin_id = sanitize_id(surface_id, "pinId")?;
            Ok(pin_label(&pin_id, window))
        }
    }
}

fn sanitize_id(raw: &str, kind: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{kind} is required"));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("{kind} has invalid characters"));
    }
    Ok(trimmed.to_string())
}

fn eval_on_label(app: &AppHandle, label: &str, js: &str) -> Result<(), String> {
    let webview = app
        .get_webview(label)
        .ok_or_else(|| format!("webview {label} is not open"))?;
    webview.eval(js).map_err(|e| e.to_string())
}

fn install_instrumentation(app: &AppHandle, label: &str, drive: bool) -> Result<(), String> {
    eval_on_label(app, label, &instrumentation_js(label, drive))
}

fn drive_lock_enabled(grant: &BrowserAgentGrant) -> bool {
    matches!(grant.mode, BrowserAgentMode::Drive) && !grant.user_has_control
}

fn playground_surface_id_from_label(label: &str) -> Option<String> {
    let rest = label.strip_prefix("playground-")?;
    if let Some((sid, _)) = rest.split_once("--") {
        Some(sid.to_string())
    } else if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

/// Rebind a surface grant onto this webview label if needed, then (re)install
/// Observe/Drive instrumentation. Call from playground show + page load so
/// detach / pin-open / navigation keep Drive lock alive.
pub fn ensure_instrumentation_for_label(app: &AppHandle, webview_label: &str) {
    let Some(state) = app.try_state::<BrowserAgentState>() else {
        return;
    };
    let label = webview_label.trim();
    if label.is_empty() {
        return;
    }
    // Only rebind onto a label that is actually open — avoids parking the grant
    // on a not-yet-created detach target.
    if app.get_webview(label).is_none() {
        return;
    }
    let grant = if let Some(existing) = state.grants.get(label) {
        existing
    } else {
        let Some(surface_id) = playground_surface_id_from_label(label) else {
            return;
        };
        // Capture prior label before rebind so UI on the old host clears.
        let prior_label = state
            .grants
            .get_for_surface(&surface_id)
            .map(|g| g.webview_label);
        let Some(rebound) = state.grants.rebind_surface_to_label(&surface_id, label) else {
            return;
        };
        if let Ok(root) = ensure_data_root(app, &state) {
            if let Some(ref prior) = prior_label {
                if prior != label {
                    mirror_grant(&root, None, prior);
                    state.observe.clear(prior);
                }
            }
            mirror_grant(&root, Some(&rebound), label);
        }
        if let Some(prior) = prior_label {
            if prior != label {
                emit_grant(app, None, &prior);
            }
        }
        emit_grant(app, Some(&rebound), label);
        rebound
    };
    let drive = drive_lock_enabled(&grant);
    let _ = install_instrumentation(app, label, drive);
}

/// Clear grants when a playground/pin surface is disposed.
pub fn clear_grants_for_surface(app: &AppHandle, surface_id: &str) {
    let Some(state) = app.try_state::<BrowserAgentState>() else {
        return;
    };
    let removed = state.grants.clear_surface(surface_id);
    for grant in removed {
        state.observe.clear(&grant.webview_label);
        if let Ok(root) = ensure_data_root(app, &state) {
            mirror_grant(&root, None, &grant.webview_label);
        }
        emit_grant(app, None, &grant.webview_label);
    }
}

pub fn clear_grant_for_label(app: &AppHandle, webview_label: &str) {
    let Some(state) = app.try_state::<BrowserAgentState>() else {
        return;
    };
    if state.grants.clear(webview_label).is_some() {
        state.observe.clear(webview_label);
        state.drive_screens.clear(webview_label);
        if let Ok(root) = ensure_data_root(app, &state) {
            mirror_grant(&root, None, webview_label);
        }
        emit_grant(app, None, webview_label);
    }
}

pub fn record_nav_event(app: &AppHandle, webview_label: &str, url: &str, title: Option<&str>) {
    let Some(state) = app.try_state::<BrowserAgentState>() else {
        return;
    };
    if state.grants.get(webview_label).is_none() {
        return;
    }
    if !state.observe.push_nav(webview_label, url, title, now_ms()) {
        return;
    }
    let _ = app.emit(
        "browser-agent-observe",
        json!({ "webviewLabel": webview_label, "kind": "nav" }),
    );
    // Host-side Drive screen shot + path caption into grant channel/thread.
    drive_screen::schedule_drive_screen_post(app, webview_label, url);
}

/// Ask the page to emit a main-frame nav with document.title (deduped in-buffer).
pub fn record_nav_from_page(app: &AppHandle, webview_label: &str) {
    let label = webview_label.trim();
    if label.is_empty() {
        return;
    }
    let _ = eval_on_label(
        app,
        label,
        "(function(){var a=window.__buzzBrowserAgent;if(a&&a.pushNavIfChanged)a.pushNavIfChanged(location.href,document.title);})();",
    );
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantSetInput {
    pub surface: BrowserAgentSurface,
    pub surface_id: String,
    pub agent_id: String,
    pub agent_pubkey: String,
    pub channel_id: String,
    pub thread_root: Option<String>,
    pub mode: BrowserAgentMode,
    pub allow_replace: Option<bool>,
    pub window_label: Option<String>,
}

#[tauri::command]
pub async fn browser_agent_grant_set(
    app: AppHandle,
    state: State<'_, BrowserAgentState>,
    input: GrantSetInput,
) -> Result<BrowserAgentGrant, String> {
    if matches!(input.surface, BrowserAgentSurface::Pin) {
        return Err("Observe/Drive is not available on pinned sites".into());
    }
    let label = resolve_label(
        input.surface,
        &input.surface_id,
        input.window_label.as_deref(),
    )?;
    if app.get_webview(&label).is_none() {
        return Err(format!("webview {label} is not open"));
    }
    let agent_pubkey = input.agent_pubkey.trim().to_string();
    if agent_pubkey.is_empty() {
        return Err("agentPubkey is required".into());
    }
    if input.channel_id.trim().is_empty() {
        return Err("channelId is required".into());
    }
    let grant = BrowserAgentGrant {
        webview_label: label.clone(),
        surface: input.surface,
        surface_id: sanitize_id(&input.surface_id, "surfaceId")?,
        agent_id: input.agent_id.trim().to_string(),
        agent_pubkey,
        channel_id: input.channel_id.trim().to_string(),
        thread_root: input
            .thread_root
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        mode: input.mode,
        user_has_control: false,
        created_at_ms: now_ms(),
    };
    state
        .grants
        .set(grant.clone(), input.allow_replace.unwrap_or(false))?;
    let root = ensure_data_root(&app, &state)?;
    mirror_grant(&root, Some(&grant), &label);
    let drive = drive_lock_enabled(&grant);
    install_instrumentation(&app, &label, drive)?;
    state.observe.push(
        &label,
        "grant",
        Some(json!({ "mode": grant.mode.as_str(), "agentPubkey": grant.agent_pubkey })),
        now_ms(),
    );
    emit_grant(&app, Some(&grant), &label);
    Ok(grant)
}

#[tauri::command]
pub async fn browser_agent_grant_clear(
    app: AppHandle,
    state: State<'_, BrowserAgentState>,
    webview_label: String,
) -> Result<(), String> {
    let label = webview_label.trim().to_string();
    let _ = state.grants.clear(&label);
    state.observe.clear(&label);
    // Best-effort unlock overlay if page still up.
    let _ = eval_on_label(
        &app,
        &label,
        "(function(){var a=window.__buzzBrowserAgent;if(a)a.setDrive(false);})();",
    );
    if let Ok(root) = ensure_data_root(&app, &state) {
        mirror_grant(&root, None, &label);
    }
    emit_grant(&app, None, &label);
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RebindSurfaceInput {
    pub from_surface_id: String,
    pub to_surface_id: String,
    pub to_webview_label: String,
    pub from_webview_label: Option<String>,
}

/// Move Observe/Drive grant across playground tabs (tab switch / new-tab focus).
#[tauri::command]
pub async fn browser_agent_rebind_surface(
    app: AppHandle,
    state: State<'_, BrowserAgentState>,
    input: RebindSurfaceInput,
) -> Result<Option<BrowserAgentGrant>, String> {
    let from = sanitize_id(&input.from_surface_id, "fromSurfaceId")?;
    let to = sanitize_id(&input.to_surface_id, "toSurfaceId")?;
    let new_label = input.to_webview_label.trim().to_string();
    if new_label.is_empty() {
        return Err("toWebviewLabel is required".into());
    }
    let prior_label = state
        .grants
        .get_for_surface(&from)
        .map(|g| g.webview_label.clone())
        .or_else(|| {
            input
                .from_webview_label
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    let Some(rebound) = state.grants.rebind_across_surfaces(&from, &to, &new_label) else {
        return Ok(None);
    };
    if let Ok(root) = ensure_data_root(&app, &state) {
        if let Some(ref prior) = prior_label {
            if prior != &new_label {
                mirror_grant(&root, None, prior);
                state.observe.clear(prior);
                // Unlock prior tab if Drive lock was installed.
                let _ = eval_on_label(
                    &app,
                    prior,
                    "(function(){var a=window.__buzzBrowserAgent;if(a)a.setDrive(false);})();",
                );
                emit_grant(&app, None, prior);
            }
        }
        mirror_grant(&root, Some(&rebound), &new_label);
    }
    emit_grant(&app, Some(&rebound), &new_label);
    if app.get_webview(&new_label).is_some() {
        let drive = drive_lock_enabled(&rebound);
        let _ = install_instrumentation(&app, &new_label, drive);
    }
    Ok(Some(rebound))
}

#[tauri::command]
pub async fn browser_agent_grant_get(
    state: State<'_, BrowserAgentState>,
    webview_label: String,
) -> Result<Option<BrowserAgentGrant>, String> {
    Ok(state.grants.get(webview_label.trim()))
}

#[tauri::command]
pub async fn browser_agent_grants_for_agent(
    state: State<'_, BrowserAgentState>,
    agent_pubkey: String,
) -> Result<Vec<BrowserAgentGrant>, String> {
    Ok(state.grants.list_for_agent(agent_pubkey.trim()))
}

#[tauri::command]
pub async fn browser_agent_grants_list(
    app: AppHandle,
    state: State<'_, BrowserAgentState>,
) -> Result<Vec<BrowserAgentGrant>, String> {
    // Drop leftover pin grants (Observe/Drive no longer applies to pins).
    let pin_labels: Vec<String> = state
        .grants
        .list_all()
        .into_iter()
        .filter(|g| matches!(g.surface, BrowserAgentSurface::Pin))
        .map(|g| g.webview_label)
        .collect();
    for label in pin_labels {
        let _ = state.grants.clear(&label);
        state.observe.clear(&label);
        if let Ok(root) = ensure_data_root(&app, &state) {
            mirror_grant(&root, None, &label);
        }
        emit_grant(&app, None, &label);
    }
    Ok(state.grants.list_all())
}

#[tauri::command]
pub async fn browser_agent_take_control(
    app: AppHandle,
    state: State<'_, BrowserAgentState>,
    webview_label: String,
) -> Result<BrowserAgentGrant, String> {
    let label = webview_label.trim().to_string();
    let grant = state
        .grants
        .get(&label)
        .ok_or_else(|| "no browser agent grant for this webview".to_string())?;
    if !matches!(grant.mode, BrowserAgentMode::Drive) {
        return Err("Take control is only available in Drive mode".into());
    }
    let Some(next) = state.grants.set_user_has_control(&label, true) else {
        return Err("no browser agent grant for this webview".into());
    };
    let _ = eval_on_label(
        &app,
        &label,
        "(function(){var a=window.__buzzBrowserAgent;if(a)a.setDrive(false);})();",
    );
    if let Ok(root) = ensure_data_root(&app, &state) {
        mirror_grant(&root, Some(&next), &label);
    }
    emit_grant(&app, Some(&next), &label);
    Ok(next)
}

#[tauri::command]
pub async fn browser_agent_release_control(
    app: AppHandle,
    state: State<'_, BrowserAgentState>,
    webview_label: String,
) -> Result<BrowserAgentGrant, String> {
    let label = webview_label.trim().to_string();
    let grant = state
        .grants
        .get(&label)
        .ok_or_else(|| "no browser agent grant for this webview".to_string())?;
    if !matches!(grant.mode, BrowserAgentMode::Drive) {
        return Err("Release control is only available in Drive mode".into());
    }
    let Some(next) = state.grants.set_user_has_control(&label, false) else {
        return Err("no browser agent grant for this webview".into());
    };
    install_instrumentation(&app, &label, true)?;
    if let Ok(root) = ensure_data_root(&app, &state) {
        mirror_grant(&root, Some(&next), &label);
    }
    emit_grant(&app, Some(&next), &label);
    Ok(next)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservePollInput {
    pub webview_label: String,
    pub agent_pubkey: String,
    pub after_id: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservePollResult {
    pub events: Vec<ObserveEvent>,
    pub grant: BrowserAgentGrant,
}

#[tauri::command]
pub async fn browser_observe_poll(
    app: AppHandle,
    state: State<'_, BrowserAgentState>,
    input: ObservePollInput,
) -> Result<ObservePollResult, String> {
    let label = input.webview_label.trim().to_string();
    let grant =
        state
            .grants
            .require_mode(&label, input.agent_pubkey.trim(), BrowserAgentMode::Observe)?;
    // Flush page console/network into the host ring (page cursor ≠ host after_id).
    let _ = flush_page_observe_queue(&app, &state, &label).await;
    let after = input.after_id.unwrap_or(0);
    let limit = input.limit.unwrap_or(50);
    let events = state.observe.poll(&label, after, limit);
    Ok(ObservePollResult { events, grant })
}

fn urlencoding_decode(raw: &str) -> Result<String, ()> {
    // Minimal decode for cookie value; percent-decoding.
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let h = |c: u8| -> Option<u8> {
                    match c {
                        b'0'..=b'9' => Some(c - b'0'),
                        b'a'..=b'f' => Some(c - b'a' + 10),
                        b'A'..=b'F' => Some(c - b'A' + 10),
                        _ => None,
                    }
                };
                if let (Some(a), Some(b)) = (h(bytes[i + 1]), h(bytes[i + 2])) {
                    out.push((a << 4) | b);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| ())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveInput {
    pub webview_label: String,
    pub agent_pubkey: String,
    pub action: DriveAction,
    pub window_label: Option<String>,
}

async fn read_cookie_value(app: &AppHandle, label: &str, name: &str) -> Option<String> {
    let webview = app.get_webview(label)?;
    let url = webview.url().ok()?;
    let cookie_url = url.clone();
    let cookies = tokio::task::spawn_blocking(move || webview.cookies_for_url(cookie_url))
        .await
        .ok()?
        .ok()?;
    let cookie = cookies.iter().find(|c| c.name() == name)?;
    urlencoding_decode(cookie.value()).ok()
}

fn enrich_drive_payload(
    result: &DriveActionResult,
    action: Option<&DriveAction>,
) -> serde_json::Value {
    let mut payload = serde_json::to_value(result).unwrap_or_else(|_| {
        json!({
            "id": result.id,
            "ok": result.ok,
            "kind": result.kind,
            "error": result.error,
        })
    });
    if let (Some(action), Some(obj)) = (action, payload.as_object_mut()) {
        if let Some(key) = action
            .key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            obj.entry("key").or_insert_with(|| json!(key));
        }
        if let Some(text) = action.text.as_deref() {
            let truncated: String = text.chars().take(24).collect();
            obj.entry("text").or_insert_with(|| json!(truncated));
        }
        if let Some(x) = action.x {
            obj.entry("x").or_insert_with(|| json!(x));
        }
        if let Some(y) = action.y {
            obj.entry("y").or_insert_with(|| json!(y));
        }
        if let Some(url) = action
            .url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            obj.entry("url").or_insert_with(|| json!(url));
        }
    }
    payload
}

fn emit_observe(app: &AppHandle, label: &str, kind: &str, payload: Option<serde_json::Value>) {
    let mut body = json!({
        "webviewLabel": label,
        "kind": kind,
    });
    if let Some(payload) = payload {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("payload".into(), payload);
        }
    }
    let _ = app.emit("browser-agent-observe", body);
}

fn record_drive_result(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
    result: &DriveActionResult,
    action: Option<&DriveAction>,
) {
    let kind = if result.ok { "drive" } else { "drive_error" };
    let payload = enrich_drive_payload(result, action);
    state
        .observe
        .push(label, kind, Some(payload.clone()), now_ms());
    emit_observe(app, label, kind, Some(payload));
}

async fn eval_drive_action(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
    action: &DriveAction,
) -> DriveActionResult {
    let mut action = action.clone();
    let id = ensure_action_id(&mut action);
    let kind = match validate_action(&action) {
        Ok(k) => k,
        Err(e) => {
            let r = error_result(&id, action.kind.trim(), e);
            record_drive_result(app, state, label, &r, Some(&action));
            return r;
        }
    };

    if kind == "navigate" {
        let url = action.url.as_deref().unwrap_or("").trim();
        let nav = navigate_host(app, label, &state.grants.get(label), url).await;
        let r = match nav {
            Ok(()) => navigate_result(&id, url),
            Err(e) => error_result(&id, "navigate", e),
        };
        record_drive_result(app, state, label, &r, Some(&action));
        return r;
    }

    if kind == "waitfor" {
        let r = wait_for_host(app, label, &action, &id).await;
        record_drive_result(app, state, label, &r, Some(&action));
        return r;
    }

    let js = match action_js(&action) {
        Ok(js) => js,
        Err(e) => {
            let r = error_result(&id, &kind, e);
            record_drive_result(app, state, label, &r, Some(&action));
            return r;
        }
    };
    if let Err(e) = eval_on_label(app, label, &js) {
        let r = error_result(&id, &kind, e);
        record_drive_result(app, state, label, &r, Some(&action));
        return r;
    }

    // Page actions are async (cursor/type animation). Poll cookie until id matches.
    let timeout_ms = drive_result_timeout_ms(&kind, &action);
    let r = wait_page_drive_result(app, label, &id, timeout_ms)
        .await
        .unwrap_or_else(|| DriveActionResult {
            id: id.clone(),
            ok: true,
            kind: kind.clone(),
            hit: None,
            url: None,
            error: None,
            message: Some("applied (no page result)".into()),
        });
    record_drive_result(app, state, label, &r, Some(&action));
    r
}

fn drive_result_timeout_ms(kind: &str, action: &DriveAction) -> u64 {
    match kind {
        "type" => {
            let n = action
                .text
                .as_deref()
                .map(|s| s.chars().count())
                .unwrap_or(0) as u64;
            (1_200 + n * 70).min(30_000)
        }
        "click" | "hover" => 2_500,
        _ => 1_500,
    }
}

async fn wait_page_drive_result(
    app: &AppHandle,
    label: &str,
    id: &str,
    timeout_ms: u64,
) -> Option<DriveActionResult> {
    let started = std::time::Instant::now();
    loop {
        if let Some(raw) = read_cookie_value(app, label, DRIVE_RESULT_COOKIE).await {
            if let Some(parsed) = parse_page_result(&raw) {
                if parsed.id == id {
                    return Some(parsed);
                }
            }
        }
        if started.elapsed().as_millis() as u64 >= timeout_ms {
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    }
}

async fn navigate_host(
    app: &AppHandle,
    label: &str,
    grant: &Option<BrowserAgentGrant>,
    url: &str,
) -> Result<(), String> {
    let surface = grant
        .as_ref()
        .map(|g| g.surface)
        .unwrap_or(BrowserAgentSurface::Playground);
    match surface {
        BrowserAgentSurface::Playground => {
            let parsed = playground_webview::parse_playground_url(url)?;
            let webview = app
                .get_webview(label)
                .ok_or_else(|| "playground webview is not open".to_string())?;
            webview.navigate(parsed).map_err(|e| e.to_string())?;
        }
        BrowserAgentSurface::Pin => {
            let parsed = Url::parse(url).map_err(|e| e.to_string())?;
            if parsed.scheme() != "https" && parsed.scheme() != "http" {
                return Err("pin navigate must use http or https".into());
            }
            let webview = app
                .get_webview(label)
                .ok_or_else(|| "pin webview is not open".to_string())?;
            webview.navigate(parsed).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

async fn wait_for_host(
    app: &AppHandle,
    label: &str,
    action: &DriveAction,
    id: &str,
) -> DriveActionResult {
    let timeout_ms = action.timeout_ms.unwrap_or(10_000).min(60_000);
    let started = std::time::Instant::now();
    loop {
        let js = match action_js(action) {
            Ok(js) => js,
            Err(e) => return error_result(id, "waitFor", e),
        };
        if let Err(e) = eval_on_label(app, label, &js) {
            return error_result(id, "waitFor", e);
        }
        if let Some(raw) = read_cookie_value(app, label, DRIVE_RESULT_COOKIE).await {
            if let Some(parsed) = parse_page_result(&raw) {
                if parsed.ok {
                    return parsed;
                }
                if started.elapsed().as_millis() as u64 >= timeout_ms {
                    return error_result(
                        id,
                        "waitFor",
                        parsed.error.unwrap_or_else(|| "timeout".into()),
                    );
                }
            }
        } else if started.elapsed().as_millis() as u64 >= timeout_ms {
            return error_result(id, "waitFor", "timeout");
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
}

#[tauri::command]
pub async fn browser_drive(
    app: AppHandle,
    state: State<'_, BrowserAgentState>,
    input: DriveInput,
) -> Result<DriveActionResult, String> {
    let label = input.webview_label.trim().to_string();
    let grant =
        state
            .grants
            .require_mode(&label, input.agent_pubkey.trim(), BrowserAgentMode::Drive)?;
    let lock = drive_lock_enabled(&grant);
    install_instrumentation(&app, &label, lock)?;
    Ok(eval_drive_action(&app, &state, &label, &input.action).await)
}

/// Atomically take the inbox file (rename → read → delete temp).
/// Lines appended during processing land in a fresh `drive-inbox.jsonl`.
fn take_drive_inbox(path: &std::path::Path) -> Result<String, String> {
    if !path.exists() {
        return Ok(String::new());
    }
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let tmp = parent.join(format!(
        "drive-inbox-{}.taking",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    match std::fs::rename(path, &tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(e.to_string()),
    }
    let raw = std::fs::read_to_string(&tmp).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    Ok(raw)
}

async fn process_drive_inbox_for_label(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
) -> Result<u32, String> {
    let Some(grant) = state.grants.get(label) else {
        return Ok(0);
    };
    // Skip while human has Taken control (Drive lock paused).
    if matches!(grant.mode, BrowserAgentMode::Drive) && grant.user_has_control {
        return Ok(0);
    }
    let root = ensure_data_root(app, state)?;
    let mut applied = 0u32;
    // Snapshot requests apply in Observe or Drive.
    applied += process_snapshot_request(app, state, label, &root).await;
    if !matches!(grant.mode, BrowserAgentMode::Drive) {
        return Ok(applied);
    }
    let path = root.join(label).join("drive-inbox.jsonl");
    let raw = take_drive_inbox(&path)?;
    if raw.trim().is_empty() {
        return Ok(applied);
    }
    emit_observe(app, label, "drive_started", None);
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                let r = error_result("inbox", "drive_error", format!("invalid inbox line: {e}"));
                record_drive_result(app, state, label, &r, None);
                continue;
            }
        };
        let Some(action_val) = value.get("action").cloned() else {
            let r = error_result("inbox", "drive_error", "inbox line missing action");
            record_drive_result(app, state, label, &r, None);
            continue;
        };
        let mut action: DriveAction = match serde_json::from_value(action_val) {
            Ok(a) => a,
            Err(e) => {
                let r = error_result("inbox", "drive_error", format!("invalid action: {e}"));
                record_drive_result(app, state, label, &r, None);
                continue;
            }
        };
        if let Some(id) = value.get("id").and_then(|v| v.as_str()) {
            if action.id.as_deref().unwrap_or("").is_empty() {
                action.id = Some(id.to_string());
            }
        }
        let agent_pubkey = value
            .get("agentPubkey")
            .and_then(|v| v.as_str())
            .unwrap_or(grant.agent_pubkey.as_str())
            .to_string();
        if agent_pubkey != grant.agent_pubkey {
            let r = error_result(
                action.id.as_deref().unwrap_or("inbox"),
                "drive_error",
                "inbox agentPubkey mismatch",
            );
            record_drive_result(app, state, label, &r, None);
            continue;
        }
        let live = match state
            .grants
            .require_mode(label, &agent_pubkey, BrowserAgentMode::Drive)
        {
            Ok(g) => g,
            Err(e) => {
                let r = error_result(action.id.as_deref().unwrap_or("inbox"), "drive_error", e);
                record_drive_result(app, state, label, &r, None);
                continue;
            }
        };
        install_instrumentation(app, label, drive_lock_enabled(&live))?;
        let result = eval_drive_action(app, state, label, &action).await;
        if result.ok {
            applied += 1;
        }
    }
    Ok(applied)
}

#[tauri::command]
pub async fn browser_agent_process_drive_inbox(
    app: AppHandle,
    state: State<'_, BrowserAgentState>,
    webview_label: String,
) -> Result<u32, String> {
    process_drive_inbox_for_label(&app, &state, webview_label.trim()).await
}

/// Flush in-page console/network queue into the observe buffer / events.jsonl
/// via `eval_with_callback` (cookie drain cannot carry BODY_CAP bodies).
/// Uses a page-seq cursor independent of the host ring `after_id`.
async fn flush_page_observe_queue(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
) -> usize {
    if !state.observe.try_begin_page_flush() {
        return 0;
    }
    let result = flush_page_observe_queue_inner(app, state, label).await;
    state.observe.end_page_flush();
    result
}

async fn flush_page_observe_queue_inner(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
) -> usize {
    let Some(webview) = app.get_webview(label) else {
        return 0;
    };
    let after = state.observe.page_after_id(label);
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    // eval_with_callback takes Fn (not FnOnce); Option+Mutex lets us send once.
    let tx = std::sync::Mutex::new(Some(tx));
    let js = drain_page_queue_js(after, 25);
    if webview
        .eval_with_callback(js, move |result| {
            if let Ok(mut slot) = tx.lock() {
                if let Some(sender) = slot.take() {
                    let _ = sender.send(result);
                }
            }
        })
        .is_err()
    {
        return 0;
    }
    let raw = match tokio::time::timeout(std::time::Duration::from_millis(120), rx).await {
        Ok(Ok(s)) => s,
        _ => return 0,
    };
    if raw.is_empty() || raw == "null" || raw == "[]" {
        return 0;
    }
    let page_events: Vec<PageDrainEvent> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return 0,
    };
    state
        .observe
        .ingest_page_events(label, after, &page_events, now_ms())
}

/// Poll every live Observe/Drive grant (~200ms): flush page console/network
/// into events.jsonl for MCP, process Drive inbox, and tab-switch requests.
pub fn spawn_grant_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let Some(state) = app.try_state::<BrowserAgentState>() else {
                continue;
            };
            let labels: Vec<String> = state
                .grants
                .list_all()
                .into_iter()
                .filter(|g| matches!(g.mode, BrowserAgentMode::Drive | BrowserAgentMode::Observe))
                .map(|g| g.webview_label)
                .collect();
            for label in labels {
                let _ = flush_page_observe_queue(&app, &state, &label).await;
                if let Err(e) = process_drive_inbox_for_label(&app, &state, &label).await {
                    eprintln!("buzz-desktop: grant watcher {label}: {e}");
                }
                if let Ok(root) = ensure_data_root(&app, &state) {
                    process_tab_switch_request(&app, &state, &label, &root);
                }
            }
        }
    });
}

async fn process_snapshot_request(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
    root: &PathBuf,
) -> u32 {
    let req_path = root.join(label).join("snapshot-request.json");
    let Ok(raw) = std::fs::read_to_string(&req_path) else {
        return 0;
    };
    let _ = std::fs::remove_file(&req_path);
    let want_shot = serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|v| v.get("screenshot").and_then(|s| s.as_bool()))
        .unwrap_or(false);

    let Some(grant) = state.grants.get(label) else {
        return 0;
    };
    install_instrumentation(app, label, drive_lock_enabled(&grant)).ok();
    let _ = eval_on_label(app, label, &snapshot_to_cookie_js());
    let mut payload = serde_json::json!({
        "ok": false,
        "error": "no snapshot cookie",
    });
    if let Some(decoded) = read_cookie_value(app, label, SNAPSHOT_COOKIE).await {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&decoded) {
            payload = value;
        }
    }
    if want_shot {
        if let Some(webview) = app.get_webview(label) {
            match playground_webview::capture::capture_playground_png(&webview, label, false) {
                Ok(shot) => {
                    let b64 = base64_encode(&shot.bytes);
                    if let Some(m) = payload.as_object_mut() {
                        m.insert("screenshotPngBase64".into(), json!(b64));
                    }
                }
                Err(e) => {
                    if let Some(m) = payload.as_object_mut() {
                        m.insert("screenshotError".into(), json!(e));
                    }
                }
            }
        }
    }
    state
        .observe
        .push(label, "snapshot", Some(payload.clone()), now_ms());
    emit_observe(app, label, "snapshot", Some(payload));
    1
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTabInfoInput {
    pub surface_id: String,
    pub url: String,
    pub title: String,
    pub is_main: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTabsEventInput {
    pub kind: String,
    pub surface_id: String,
    pub opener_surface_id: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTabsSyncInput {
    pub browser_id: String,
    pub main_tab_sid: String,
    pub active_tab_sid: String,
    pub tabs: Vec<BrowserTabInfoInput>,
    pub event: Option<BrowserTabsEventInput>,
}

/// Mirror browser-group tabs under the active grant dir for MCP `browser_tabs`.
/// Optionally push an Observe event (`tab_opened` / `tab_switched`).
#[tauri::command]
pub async fn browser_agent_sync_tabs(
    app: AppHandle,
    state: State<'_, BrowserAgentState>,
    input: BrowserTabsSyncInput,
) -> Result<(), String> {
    let browser_id = input.browser_id.trim();
    if browser_id.is_empty() {
        return Err("browserId is required".into());
    }
    let root = ensure_data_root(&app, &state)?;
    let body = serde_json::json!({
        "browserId": browser_id,
        "mainTabSid": input.main_tab_sid,
        "activeTabSid": input.active_tab_sid,
        "tabs": input.tabs.iter().map(|t| serde_json::json!({
            "surfaceId": t.surface_id,
            "url": t.url,
            "title": t.title,
            "isMain": t.is_main,
        })).collect::<Vec<_>>(),
    });
    // Write tabs.json next to every live grant whose surface is in this group.
    let surface_set: std::collections::HashSet<String> = input
        .tabs
        .iter()
        .map(|t| t.surface_id.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let grants = state.grants.list_all();
    let mut wrote = false;
    for grant in &grants {
        if !matches!(grant.surface, BrowserAgentSurface::Playground) {
            continue;
        }
        if !surface_set.contains(&grant.surface_id) {
            continue;
        }
        let dir = root.join(&grant.webview_label);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("tabs.json");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&body).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        wrote = true;
        if let Some(ref ev) = input.event {
            let kind = ev.kind.trim();
            if kind == "tab_opened" || kind == "tab_switched" {
                let payload = serde_json::json!({
                    "browserId": browser_id,
                    "surfaceId": ev.surface_id,
                    "openerSurfaceId": ev.opener_surface_id,
                    "url": ev.url,
                    "mainTabSid": input.main_tab_sid,
                    "activeTabSid": input.active_tab_sid,
                    "tabs": body.get("tabs").cloned().unwrap_or(serde_json::json!([])),
                });
                state
                    .observe
                    .push(&grant.webview_label, kind, Some(payload.clone()), now_ms());
                emit_observe(&app, &grant.webview_label, kind, Some(payload));
            }
        }
    }
    // Always keep a browser-id keyed copy for agents that list before grant rebind.
    if !wrote {
        let dir = root.join("browsers").join(browser_id);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("tabs.json");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&body).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    } else {
        let dir = root.join("browsers").join(browser_id);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("tabs.json");
        let _ = std::fs::write(&path, serde_json::to_vec_pretty(&body).unwrap_or_default());
    }
    Ok(())
}

fn process_tab_switch_request(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
    root: &PathBuf,
) {
    let req_path = root.join(label).join("tab-switch-request.json");
    let Ok(raw) = std::fs::read_to_string(&req_path) else {
        return;
    };
    let _ = std::fs::remove_file(&req_path);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    let Some(surface_id) = value
        .get("surfaceId")
        .or_else(|| value.get("surface_id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    // Ensure the requesting agent still holds a grant on this label.
    if state.grants.get(label).is_none() {
        return;
    }
    let browser_id = value
        .get("browserId")
        .or_else(|| value.get("browser_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let payload = serde_json::json!({
        "surfaceId": surface_id,
        "browserId": browser_id,
    });
    let _ = app.emit("browser-agent-switch-tab", payload);
}


#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookInjectProcedure {
    pub id: String,
    pub title: String,
    pub summary: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookInjectPayload {
    pub agent_brief: String,
    pub procedures: Vec<RunbookInjectProcedure>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookFullProcedure {
    pub id: String,
    pub title: String,
    pub steps: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_channel: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookFullPayload {
    pub agent_brief: String,
    pub procedures: Vec<RunbookFullProcedure>,
    pub updated_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MirrorRunbookInput {
    pub webview_label: String,
    pub inject: RunbookInjectPayload,
    pub full: RunbookFullPayload,
}

fn write_json_file(path: &PathBuf, value: &impl Serialize) {
    if let Ok(bytes) = serde_json::to_vec_pretty(value) {
        if let Ok(mut f) = File::create(path) {
            let _ = f.write_all(&bytes);
        }
    }
}

/// Write grant-adjacent runbook files for MCP inject + on-demand get.
#[tauri::command]
pub async fn browser_agent_mirror_runbook(
    app: AppHandle,
    state: State<'_, BrowserAgentState>,
    input: MirrorRunbookInput,
) -> Result<(), String> {
    let label = input.webview_label.trim().to_string();
    if label.is_empty() {
        return Err("webviewLabel is required".into());
    }
    let root = ensure_data_root(&app, &state)?;
    let dir = root.join(&label);
    create_dir_all(&dir).map_err(|e| e.to_string())?;
    write_json_file(&dir.join("runbook.json"), &input.inject);
    write_json_file(&dir.join("runbook-full.json"), &input.full);
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookProposeLine {
    pub title: String,
    pub steps: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
}

fn take_runbook_propose_inbox(path: &std::path::Path) -> Result<String, String> {
    if !path.exists() {
        return Ok(String::new());
    }
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let tmp = parent.join(format!(
        "runbook-propose-{}.taking",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    match std::fs::rename(path, &tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(e.to_string()),
    }
    let raw = std::fs::read_to_string(&tmp).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    Ok(raw)
}

/// Drain MCP `browser_runbook_propose` lines for Desktop to apply as pending.
#[tauri::command]
pub async fn browser_agent_take_runbook_proposes(
    app: AppHandle,
    state: State<'_, BrowserAgentState>,
    webview_label: String,
) -> Result<Vec<RunbookProposeLine>, String> {
    let label = webview_label.trim().to_string();
    if label.is_empty() {
        return Err("webviewLabel is required".into());
    }
    let root = ensure_data_root(&app, &state)?;
    let path = root.join(&label).join("runbook-propose.jsonl");
    let raw = take_runbook_propose_inbox(&path)?;
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(parsed) = serde_json::from_str::<RunbookProposeLine>(line) {
            if !parsed.title.trim().is_empty() {
                out.push(parsed);
            }
        }
    }
    Ok(out)
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod inbox_tests {
    use super::take_drive_inbox;
    use std::io::Write;

    #[test]
    fn take_inbox_is_atomic_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drive-inbox.jsonl");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, r#"{{"action":{{"kind":"scroll","dx":0,"dy":1}}}}"#).unwrap();
        }
        let raw = take_drive_inbox(&path).unwrap();
        assert!(raw.contains("scroll"));
        assert!(!path.exists(), "inbox should be renamed away");
        // New appends can recreate the file without racing the taken contents.
        std::fs::write(&path, "new\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
    }
}
