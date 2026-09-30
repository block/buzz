//! File-backed Buzz browser Observe/Drive tools (grant-gated).
//! Desktop mirrors grants/events under `{BUZZ_BROWSER_AGENT_DIR|/app-data/browser-agent}`.

use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
thread_local! {
    static TEST_AGENT_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
    static TEST_PUBKEY: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn agent_dir() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(dir) = TEST_AGENT_DIR.with(|c| c.borrow().clone()) {
            return dir;
        }
    }
    if let Ok(dir) = std::env::var("BUZZ_BROWSER_AGENT_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    // macOS Hula default app support path when Desktop is running as this user.
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join("Library/Application Support/com.huladesk.buzz/browser-agent")
}

fn caller_pubkey() -> Option<String> {
    #[cfg(test)]
    {
        if let Some(pk) = TEST_PUBKEY.with(|c| c.borrow().clone()) {
            return Some(pk);
        }
    }
    // buzz-acp injects the agent secret; derive is heavy — prefer explicit pubkey env.
    for key in ["BUZZ_AGENT_PUBKEY", "BUZZ_ACP_PUBKEY"] {
        if let Ok(v) = std::env::var(key) {
            let t = v.trim().to_lowercase();
            if t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(t);
            }
        }
    }
    None
}

fn read_json(path: &Path) -> Option<Value> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn grant_matches(grant: &Value, pubkey: &str) -> bool {
    grant
        .get("agentPubkey")
        .and_then(|v| v.as_str())
        .map(|p| p.eq_ignore_ascii_case(pubkey))
        .unwrap_or(false)
}


fn grant_surface_id(grant: &Value) -> Option<&str> {
    grant.get("surfaceId").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
}

fn grant_label(grant: &Value) -> Option<&str> {
    grant
        .get("webviewLabel")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Resolve a stable agent target to the live grant mirror dir + grant JSON.
/// Prefer `surface_id` (survives popout/detach). Fall back to `webview_label`,
/// then the sole current grant for this agent when neither is set.
fn resolve_grant_target(
    pubkey: &str,
    webview_label: &str,
    surface_id: Option<&str>,
) -> Result<(PathBuf, Value, String), ErrorData> {
    let root = agent_dir();
    let surface = surface_id.map(str::trim).filter(|s| !s.is_empty());
    let label = webview_label.trim();

    if let Some(sid) = surface {
        let mut matches = Vec::new();
        if let Ok(entries) = fs::read_dir(&root) {
            for entry in entries.flatten() {
                let grant_path = entry.path().join("grant.json");
                if let Some(grant) = read_json(&grant_path) {
                    if grant_matches(&grant, pubkey) && grant_surface_id(&grant) == Some(sid) {
                        let entry_name = entry.file_name().to_string_lossy().into_owned();
                        let live = grant_label(&grant)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| entry_name.clone());
                        let dir = {
                            let preferred = root.join(&live);
                            if preferred.join("grant.json").exists() {
                                preferred
                            } else {
                                entry.path()
                            }
                        };
                        let canonical = grant_label(&grant) == Some(entry_name.as_str());
                        matches.push((canonical, dir, grant, live));
                    }
                }
            }
        }
        // Prefer mirror dir whose name matches grant.webviewLabel (post-rebind).
        matches.sort_by_key(|(canonical, _, _, _)| std::cmp::Reverse(*canonical));
        if let Some((_, dir, grant, live)) = matches.into_iter().next() {
            return Ok((dir, grant, live));
        }
        return Err(ErrorData::invalid_params(
            format!("no grant for surfaceId={sid}"),
            None,
        ));
    }

    if !label.is_empty() {
        let dir = root.join(label);
        let grant = read_json(&dir.join("grant.json")).ok_or_else(|| {
            ErrorData::invalid_params(format!("no grant for {label}"), None)
        })?;
        if !grant_matches(&grant, pubkey) {
            return Err(ErrorData::invalid_params(
                "caller is not the granted agent for this webview",
                None,
            ));
        }
        // If mirror shows a newer label for same surface, prefer that.
        if let Some(sid) = grant_surface_id(&grant) {
            if let Ok(entries) = fs::read_dir(&root) {
                for entry in entries.flatten() {
                    let gpath = entry.path().join("grant.json");
                    if let Some(other) = read_json(&gpath) {
                        if grant_matches(&other, pubkey)
                            && grant_surface_id(&other) == Some(sid)
                        {
                            if let Some(live) = grant_label(&other).map(|s| s.to_string()) {
                                if live != label {
                                    let live_dir = root.join(&live);
                                    if live_dir.join("grant.json").exists() {
                                        return Ok((live_dir, other, live));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        return Ok((dir, grant, label.to_string()));
    }

    // No target: sole grant for this agent.
    let mut matches = Vec::new();
    if let Ok(entries) = fs::read_dir(&root) {
        for entry in entries.flatten() {
            let grant_path = entry.path().join("grant.json");
            if let Some(grant) = read_json(&grant_path) {
                if grant_matches(&grant, pubkey) {
                    let live = grant_label(&grant)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            entry
                                .file_name()
                                .to_string_lossy()
                                .into_owned()
                        });
                    matches.push((root.join(&live), grant, live));
                }
            }
        }
    }
    match matches.len() {
        0 => Err(ErrorData::invalid_params(
            "no browser grant for this agent; pass surface_id or webview_label",
            None,
        )),
        1 => {
            let (dir, grant, live) = matches.remove(0);
            Ok((dir, grant, live))
        }
        _ => Err(ErrorData::invalid_params(
            "multiple browser grants; pass surface_id (preferred) or webview_label",
            None,
        )),
    }
}

fn require_drive_grant_resolved(
    pubkey: &str,
    webview_label: &str,
    surface_id: Option<&str>,
) -> Result<(PathBuf, Value, String), ErrorData> {
    let (dir, grant, label) = resolve_grant_target(pubkey, webview_label, surface_id)?;
    let mode = grant
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("observe");
    if mode != "drive" {
        return Err(ErrorData::invalid_params(
            "drive requires Drive mode grant",
            None,
        ));
    }
    Ok((dir, grant, label))
}

fn event_action_id(ev: &Value) -> Option<&str> {
    ev.get("payload")
        .and_then(|p| p.get("id"))
        .and_then(|v| v.as_str())
}

fn collect_drive_results_from_events(
    events_path: &Path,
    wanted: &[String],
) -> (Vec<Value>, Option<String>) {
    let mut found: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    let mut last_url: Option<String> = None;
    if let Ok(file) = fs::File::open(events_path) {
        for line in BufReader::new(file).lines().flatten() {
            let Ok(ev) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let kind = ev.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            if kind == "drive" || kind == "drive_error" {
                if let Some(id) = event_action_id(&ev) {
                    if wanted.iter().any(|w| w == id) {
                        found.insert(id.to_string(), ev.clone());
                    }
                }
                if let Some(url) = ev
                    .get("payload")
                    .and_then(|p| p.get("url"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    last_url = Some(url.to_string());
                }
            } else if kind == "nav" {
                if let Some(url) = ev
                    .get("payload")
                    .and_then(|p| p.get("url"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    last_url = Some(url.to_string());
                }
            }
        }
    }
    let results: Vec<Value> = wanted
        .iter()
        .filter_map(|id| found.get(id).cloned())
        .collect();
    (results, last_url)
}

/// Poll events.jsonl until all action ids have drive/drive_error results or timeout.
fn wait_for_drive_results(
    dir: &Path,
    ids: &[String],
    timeout_ms: u64,
    poll_ms: u64,
) -> (Vec<Value>, Option<String>, bool) {
    let path = dir.join("events.jsonl");
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        let (results, last_url) = collect_drive_results_from_events(&path, ids);
        if results.len() >= ids.len() {
            return (results, last_url, true);
        }
        if std::time::Instant::now() >= deadline {
            return (results, last_url, false);
        }
        std::thread::sleep(std::time::Duration::from_millis(poll_ms.max(5)));
    }
}


fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn new_action_id() -> String {
    format!(
        "d{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObservePollParams {
    /// Live webview label (fragile across popout). Prefer `surface_id`.
    #[serde(default)]
    pub webview_label: String,
    /// Stable playground surface id — preferred target across detach/popout.
    #[serde(default)]
    pub surface_id: Option<String>,
    #[serde(default)]
    pub after_id: Option<u64>,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub fn observe_poll(p: ObservePollParams) -> Result<CallToolResult, ErrorData> {
    let Some(pubkey) = caller_pubkey() else {
        return Err(ErrorData::invalid_params(
            "BUZZ_AGENT_PUBKEY (64-hex) required for browser_observe_poll",
            None,
        ));
    };
    let (dir, grant, label) = resolve_grant_target(
        &pubkey,
        &p.webview_label,
        p.surface_id.as_deref(),
    )?;
    let after = p.after_id.unwrap_or(0);
    let limit = p.limit.unwrap_or(50).clamp(1, 200);
    let path = dir.join("events.jsonl");
    let mut events = Vec::new();
    if let Ok(file) = fs::File::open(path) {
        for line in BufReader::new(file).lines().flatten() {
            if let Ok(ev) = serde_json::from_str::<Value>(&line) {
                let id = ev.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                if id > after {
                    events.push(ev);
                    if events.len() >= limit {
                        break;
                    }
                }
            }
        }
    }
    let runbook = read_json(&dir.join("runbook.json"));
    let webview_hidden = grant
        .get("webviewHidden")
        .and_then(|v| v.as_bool())
        .or_else(|| grant.get("parked").and_then(|v| v.as_bool()))
        .unwrap_or(false);
    let body = json!({
        "grant": grant,
        "webviewLabel": label,
        "surfaceId": grant_surface_id(&grant),
        "webviewHidden": webview_hidden,
        "parked": webview_hidden,
        "runbook": runbook,
        "events": events,
        "driveContext": {
            "surfaceId": grant_surface_id(&grant),
            "webviewLabel": label,
            "webviewHidden": webview_hidden,
            "parked": webview_hidden,
            "mode": grant.get("mode"),
            "preferSurfaceId": true,
            "tools": [
                "browser_observe_poll",
                "browser_agent_grants",
                "browser_tabs",
                "browser_switch_tab",
                "browser_get_viewport",
                "browser_set_viewport",
                "browser_snapshot",
                "browser_record_start",
                "browser_record_stop_and_post",
                "browser_drive",
                "browser_fill_field",
                "browser_runbook_get",
                "browser_runbook_propose"
            ]
        },
        "runbookNote": "runbook.agentBrief + active procedures + driveProtocol. Prefer surfaceId. browser_snapshot waits inline. browser_record_start/stop_and_post are OPT-IN section clips only — call when the runbook/user asks for that section; never auto-start on Drive grant. browser_fill_field for forms. browser_runbook_propose auto-activates unless the title is human-persisted. Agent brief is human-owned."
    });
    Ok(CallToolResult::success(vec![Content::text(
        body.to_string(),
    )]))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GrantsParams {}

pub fn grants(_p: GrantsParams) -> Result<CallToolResult, ErrorData> {
    let Some(pubkey) = caller_pubkey() else {
        return Err(ErrorData::invalid_params(
            "BUZZ_AGENT_PUBKEY required for browser_agent_grants",
            None,
        ));
    };
    let root = agent_dir();
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let grant_path = entry.path().join("grant.json");
            if let Some(grant) = read_json(&grant_path) {
                if grant_matches(&grant, &pubkey) {
                    out.push(grant);
                }
            }
        }
    }
    Ok(CallToolResult::success(vec![Content::text(
        json!({ "grants": out }).to_string(),
    )]))
}



#[derive(Debug, Deserialize, JsonSchema)]
pub struct TabsParams {
    /// Live webview label (fragile across popout). Prefer `surface_id`.
    #[serde(default)]
    pub webview_label: Option<String>,
    /// Preferred stable playground session / tab id.
    #[serde(default)]
    pub surface_id: Option<String>,
}

/// List tabs in the browser group for a grant. Main tab is primary focus
/// (`isMain` / `mainTabSid`); drive/observe target the active tab.
pub fn tabs(p: TabsParams) -> Result<CallToolResult, ErrorData> {
    let Some(pubkey) = caller_pubkey() else {
        return Err(ErrorData::invalid_params(
            "BUZZ_AGENT_PUBKEY required for browser_tabs",
            None,
        ));
    };
    let (_dir, grant, label) = resolve_grant_target(
        &pubkey,
        p.webview_label.as_deref().unwrap_or(""),
        p.surface_id.as_deref(),
    )?;
    let root = agent_dir();
    let mut tabs_val = read_json(&root.join(&label).join("tabs.json"));
    if tabs_val.is_none() {
        if let Some(sid) = grant_surface_id(&grant) {
            // Fall back to browsers/{browserId} or any sibling grant tabs.json.
            if let Ok(entries) = fs::read_dir(root.join("browsers")) {
                for entry in entries.flatten() {
                    if let Some(v) = read_json(&entry.path().join("tabs.json")) {
                        let active = v.get("activeTabSid").and_then(|x| x.as_str());
                        let main = v.get("mainTabSid").and_then(|x| x.as_str());
                        let list = v.get("tabs").and_then(|x| x.as_array());
                        let mentions = list
                            .map(|arr| {
                                arr.iter().any(|tab| {
                                    tab.get("surfaceId").and_then(|s| s.as_str()) == Some(sid)
                                })
                            })
                            .unwrap_or(false);
                        if mentions || active == Some(sid) || main == Some(sid) {
                            tabs_val = Some(v);
                            break;
                        }
                    }
                }
            }
        }
    }
    let body = json!({
        "ok": true,
        "grant": grant,
        "webviewLabel": label,
        "surfaceId": grant_surface_id(&grant),
        "tabs": tabs_val.clone().unwrap_or(json!({
            "browserId": grant_surface_id(&grant),
            "mainTabSid": grant_surface_id(&grant),
            "activeTabSid": grant_surface_id(&grant),
            "tabs": [{
                "surfaceId": grant_surface_id(&grant),
                "url": "",
                "title": "",
                "isMain": true,
            }],
        })),
        "note": "Main tab (isMain/mainTabSid) is primary focus. Extra tabs come from in-page window.open / target=_blank. Use browser_switch_tab to focus a tab (rebinds Observe/Drive). Poll browser_observe_poll for kind=tab_opened / tab_switched."
    });
    Ok(CallToolResult::success(vec![Content::text(body.to_string())]))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SwitchTabParams {
    /// Target tab surfaceId (playground session id).
    pub surface_id: String,
    /// Optional: current grant label or any tab in the same group.
    #[serde(default)]
    pub webview_label: Option<String>,
    /// Optional: any surface in the group to resolve the grant (defaults to surface_id).
    #[serde(default)]
    pub from_surface_id: Option<String>,
}

/// Ask Desktop to focus a tab in the granted browser group (main or extra).
/// Writes tab-switch-request.json; Desktop emits browser-agent-switch-tab and rebinds the grant.
pub fn switch_tab(p: SwitchTabParams) -> Result<CallToolResult, ErrorData> {
    let Some(pubkey) = caller_pubkey() else {
        return Err(ErrorData::invalid_params(
            "BUZZ_AGENT_PUBKEY required for browser_switch_tab",
            None,
        ));
    };
    let target = p.surface_id.trim();
    if target.is_empty() {
        return Err(ErrorData::invalid_params("surface_id is required", None));
    }
    let resolve_sid = p
        .from_surface_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(target);
    let (dir, grant, label) = resolve_grant_target(
        &pubkey,
        p.webview_label.as_deref().unwrap_or(""),
        Some(resolve_sid),
    )?;
    let req = json!({
        "surfaceId": target,
        "browserId": read_json(&dir.join("tabs.json"))
            .and_then(|v| v.get("browserId").and_then(|b| b.as_str()).map(|s| s.to_string())),
        "requestedAtMs": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    });
    let path = dir.join("tab-switch-request.json");
    let mut f = fs::File::create(&path).map_err(|e| {
        ErrorData::internal_error(format!("write tab-switch-request: {e}"), None)
    })?;
    f.write_all(req.to_string().as_bytes()).map_err(|e| {
        ErrorData::internal_error(format!("write tab-switch-request: {e}"), None)
    })?;
    Ok(CallToolResult::success(vec![Content::text(
        json!({
            "ok": true,
            "queued": true,
            "surfaceId": target,
            "webviewLabel": label,
            "grantSurfaceId": grant_surface_id(&grant),
            "note": "Desktop will focus the tab and rebind Observe/Drive onto it. Poll browser_observe_poll for kind=tab_switched, or call browser_tabs."
        })
        .to_string(),
    )]))
}

const VIEWPORT_DEVICE_IDS: &[&str] = &[
    "iphone-se",
    "iphone-16",
    "iphone-16-pro-max",
    "pixel-8",
    "ipad-mini",
    "ipad-pro-11",
];

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetViewportParams {
    #[serde(default)]
    pub webview_label: Option<String>,
    #[serde(default)]
    pub surface_id: Option<String>,
}

/// Read the mirrored Stage viewport for a grant (Observe or Drive).
pub fn get_viewport(p: GetViewportParams) -> Result<CallToolResult, ErrorData> {
    let Some(pubkey) = caller_pubkey() else {
        return Err(ErrorData::invalid_params(
            "BUZZ_AGENT_PUBKEY required for browser_get_viewport",
            None,
        ));
    };
    let (dir, grant, label) = resolve_grant_target(
        &pubkey,
        p.webview_label.as_deref().unwrap_or(""),
        p.surface_id.as_deref(),
    )?;
    let mut viewport = read_json(&dir.join("viewport.json"));
    if viewport.is_none() {
        if let Some(sid) = grant_surface_id(&grant) {
            viewport = read_json(&agent_dir().join("viewports").join(sid).join("viewport.json"));
        }
    }
    let body = json!({
        "ok": true,
        "grant": grant,
        "webviewLabel": label,
        "surfaceId": grant_surface_id(&grant),
        "viewport": viewport.clone().unwrap_or(json!({
            "surfaceId": grant_surface_id(&grant),
            "mode": "desktop",
            "width": 0,
            "height": 0,
        })),
        "note": "Viewport mirrors Desktop Stage (Desktop | Responsive | Mobile). Use browser_set_viewport while Driving to change it."
    });
    Ok(CallToolResult::success(vec![Content::text(body.to_string())]))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetViewportParams {
    /// desktop | responsive | mobile
    pub mode: String,
    #[serde(default)]
    pub webview_label: Option<String>,
    #[serde(default)]
    pub surface_id: Option<String>,
    /// Responsive CSS width (min 320).
    #[serde(default)]
    pub width: Option<f64>,
    /// Responsive CSS height (min 320).
    #[serde(default)]
    pub height: Option<f64>,
    /// Mobile museum device id (iphone-16, pixel-8, …).
    #[serde(default)]
    pub device_id: Option<String>,
    /// portrait | landscape (mobile).
    #[serde(default)]
    pub orientation: Option<String>,
    /// Mobile museum scale percent (50–200, steps of 25).
    #[serde(default)]
    pub scale_percent: Option<f64>,
}

/// Validate MCP viewport payload (pure; used by set_viewport + tests).
pub fn validate_set_viewport_params(p: &SetViewportParams) -> Result<Value, String> {
    let mode = p.mode.trim().to_ascii_lowercase();
    match mode.as_str() {
        "desktop" => Ok(json!({ "mode": "desktop" })),
        "responsive" => {
            let width = p.width.unwrap_or(390.0);
            let height = p.height.unwrap_or(844.0);
            if !width.is_finite() || !height.is_finite() {
                return Err("width/height must be finite numbers".into());
            }
            let width = width.round().max(320.0) as i64;
            let height = height.round().max(320.0) as i64;
            Ok(json!({
                "mode": "responsive",
                "width": width,
                "height": height,
            }))
        }
        "mobile" => {
            let device_id = p
                .device_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("iphone-16");
            if !VIEWPORT_DEVICE_IDS.iter().any(|d| *d == device_id) {
                return Err(format!(
                    "deviceId must be one of: {}",
                    VIEWPORT_DEVICE_IDS.join(", ")
                ));
            }
            let orientation = p
                .orientation
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("portrait")
                .to_ascii_lowercase();
            if orientation != "portrait" && orientation != "landscape" {
                return Err("orientation must be portrait|landscape".into());
            }
            let scale = p.scale_percent.unwrap_or(100.0);
            if !scale.is_finite() {
                return Err("scalePercent must be a finite number".into());
            }
            let stepped = ((scale / 25.0).round() * 25.0).clamp(50.0, 200.0) as i64;
            Ok(json!({
                "mode": "mobile",
                "deviceId": device_id,
                "orientation": orientation,
                "scalePercent": stepped,
            }))
        }
        _ => Err("mode must be desktop|responsive|mobile".into()),
    }
}

/// Ask Desktop to set Stage viewport (Drive mode only). Writes viewport-request.json.
pub fn set_viewport(p: SetViewportParams) -> Result<CallToolResult, ErrorData> {
    let Some(pubkey) = caller_pubkey() else {
        return Err(ErrorData::invalid_params(
            "BUZZ_AGENT_PUBKEY required for browser_set_viewport",
            None,
        ));
    };
    let patch = validate_set_viewport_params(&p).map_err(|e| {
        ErrorData::invalid_params(e, None)
    })?;
    let (dir, grant, label) = require_drive_grant_resolved(
        &pubkey,
        p.webview_label.as_deref().unwrap_or(""),
        p.surface_id.as_deref(),
    )?;
    let surface = grant_surface_id(&grant)
        .map(|s| s.to_string())
        .unwrap_or_default();
    let mut req = patch;
    if let Some(obj) = req.as_object_mut() {
        obj.insert("surfaceId".into(), json!(surface));
        obj.insert(
            "requestedAtMs".into(),
            json!(SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)),
        );
    }
    let path = dir.join("viewport-request.json");
    let mut f = fs::File::create(&path).map_err(|e| {
        ErrorData::internal_error(format!("write viewport-request: {e}"), None)
    })?;
    f.write_all(req.to_string().as_bytes()).map_err(|e| {
        ErrorData::internal_error(format!("write viewport-request: {e}"), None)
    })?;
    Ok(CallToolResult::success(vec![Content::text(
        json!({
            "ok": true,
            "queued": true,
            "surfaceId": surface,
            "webviewLabel": label,
            "viewport": req,
            "note": "Desktop will apply Stage viewport (mode/device/size/scale). Call browser_get_viewport or re-snapshot after apply."
        })
        .to_string(),
    )]))
}

/// Drive action shape. Field is `kind` (not `type`).
/// Kinds: navigate | click | type | fill | scroll | hover | key | waitFor.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DriveActionParam {
    /// Action kind: navigate | click | type | fill | scroll | hover | key | waitFor
    pub kind: String,
    #[serde(default)]
    #[schemars(description = "Optional id echoed in drive/drive_error events")]
    pub id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub selector: Option<String>,
    /// Snapshot interactive ref (e.g. "e0") from browser_snapshot.
    #[serde(default, rename = "ref")]
    #[schemars(description = "Snapshot interactive ref from browser_snapshot (e.g. e0)")]
    pub ref_id: Option<String>,
    #[serde(default)]
    pub dx: Option<f64>,
    #[serde(default)]
    pub dy: Option<f64>,
    /// For kind=key: Enter, Tab, Escape, Backspace, ArrowLeft/Right/Up/Down
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub url_contains: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// For kind=fill: clear existing value before typing (default true).
    #[serde(default)]
    pub clear: Option<bool>,
}

const SUPPORTED_KEYS: &[&str] = &[
    "Enter",
    "Tab",
    "Escape",
    "Backspace",
    "ArrowLeft",
    "ArrowRight",
    "ArrowUp",
    "ArrowDown",
];

fn validate_drive_action(action: &DriveActionParam) -> Result<String, String> {
    let kind = action.kind.trim().to_ascii_lowercase();
    match kind.as_str() {
        "click" | "hover" => {
            let has_xy = action.x.is_some() && action.y.is_some();
            let has_sel = action
                .selector
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some();
            let has_ref = action
                .ref_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some();
            if !has_xy && !has_sel && !has_ref {
                return Err(format!("{kind} requires x,y or selector or ref"));
            }
        }
        "type" => {
            if action.text.is_none() {
                return Err("type requires text".into());
            }
        }
        "fill" => {
            if action.text.is_none() {
                return Err("fill requires text".into());
            }
            let has_xy = action.x.is_some() && action.y.is_some();
            let has_sel = action
                .selector
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some();
            let has_ref = action
                .ref_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some();
            if !has_xy && !has_sel && !has_ref {
                return Err("fill requires x,y or selector or ref".into());
            }
        }
        "scroll" => {}
        "navigate" => {
            if action.url.as_deref().map(str::trim).unwrap_or("").is_empty() {
                return Err("navigate requires url".into());
            }
        }
        "key" => {
            let key = action.key.as_deref().map(str::trim).unwrap_or("");
            if key.is_empty() {
                return Err("key requires key".into());
            }
            if !SUPPORTED_KEYS
                .iter()
                .any(|k| k.eq_ignore_ascii_case(key))
            {
                return Err(format!(
                    "unsupported key {key:?}; supported: {}",
                    SUPPORTED_KEYS.join(", ")
                ));
            }
        }
        "waitfor" => {
            let has = action
                .url_contains
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some()
                || action
                    .selector
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .is_some()
                || action
                    .text
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .is_some();
            if !has {
                return Err("waitFor requires urlContains, selector, or text".into());
            }
        }
        "" => return Err("action.kind is required (use kind, not type)".into()),
        other => {
            return Err(format!(
                "unknown drive action kind: {other} (use kind, not type)"
            ))
        }
    }
    Ok(kind)
}

/// Parse `action` from object or JSON string. Rejects unknown kinds / missing fields.
pub fn parse_drive_action(raw: &Value) -> Result<DriveActionParam, String> {
    let value = match raw {
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err("action string is empty".into());
            }
            serde_json::from_str::<Value>(trimmed)
                .map_err(|e| format!("action string is not valid JSON: {e}"))?
        }
        other => other.clone(),
    };
    if let Some(obj) = value.as_object() {
        if obj.contains_key("type") && !obj.contains_key("kind") {
            return Err("action uses `type`; Drive actions require `kind`".into());
        }
    }
    let action: DriveActionParam = serde_json::from_value(value)
        .map_err(|e| format!("invalid Drive action shape: {e}"))?;
    validate_drive_action(&action)?;
    Ok(action)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DriveParams {
    /// Live webview label (fragile across popout). Prefer `surface_id`.
    #[serde(default)]
    pub webview_label: String,
    /// Stable playground surface id — preferred target across detach/popout.
    #[serde(default)]
    pub surface_id: Option<String>,
    /// Single Drive action object, or a JSON string of that object.
    /// Shape: { kind, id?, url?, x?, y?, text?, selector?, ref?, dx?, dy?, key?, urlContains?, timeoutMs? }.
    /// Use `kind` (not `type`): navigate | click | type | fill | scroll | hover | key | waitFor.
    #[schemars(description = "DriveAction object or JSON string. Field is kind (not type).")]
    pub action: Value,
    /// Optional batch of actions. Validated then queued; by default waits for per-step results.
    #[serde(default)]
    pub actions: Option<Vec<Value>>,
    /// When true, return after queueing without waiting for Desktop results.
    #[serde(default)]
    pub queue_only: Option<bool>,
    /// Max ms to wait for drive/drive_error results (default 10000). Ignored if queue_only.
    #[serde(default)]
    pub wait_timeout_ms: Option<u64>,
    /// When true, after Drive results also wait for an inline DOM snapshot.
    #[serde(default)]
    pub include_snapshot: Option<bool>,
}

fn queue_action(dir: &Path, pubkey: &str, mut action: DriveActionParam) -> Result<String, ErrorData> {
    let id = action
        .id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(new_action_id);
    action.id = Some(id.clone());
    fs::create_dir_all(dir).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    let path = dir.join("drive-inbox.jsonl");
    let line = format!(
        "{}\n",
        json!({
            "id": id,
            "agentPubkey": pubkey,
            "action": action,
            "atMs": now_ms(),
        })
    );
    // Lock + single write_all so Desktop rename cannot observe a mid-line truncate
    // (`EOF while parsing a string at column …`).
    let _guard = inbox_lock_acquire(dir).map_err(|e| ErrorData::internal_error(e, None))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    file.write_all(line.as_bytes())
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    file.flush()
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    let _ = fs::write(dir.join("drive-wake"), format!("{}\n", now_ms()));
    Ok(id)
}

struct InboxLockGuard {
    path: PathBuf,
}

impl Drop for InboxLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn inbox_lock_acquire(dir: &Path) -> Result<InboxLockGuard, String> {
    let path = dir.join("drive-inbox.lock");
    let started = std::time::Instant::now();
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(InboxLockGuard { path }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if started.elapsed().as_secs() >= 5 {
                    let _ = fs::remove_file(&path);
                    continue;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn drive_wait_response(
    dir: &Path,
    label: &str,
    grant: &Value,
    ids: &[String],
    batch: bool,
    queue_only: bool,
    wait_timeout_ms: u64,
    poll_ms: u64,
    include_snapshot: bool,
    pubkey: &str,
) -> Result<CallToolResult, ErrorData> {
    let started = std::time::Instant::now();
    if queue_only {
        let body = if batch {
            json!({
                "ok": true,
                "queued": true,
                "batch": true,
                "ids": ids,
                "count": ids.len(),
                "webviewLabel": label,
                "surfaceId": grant_surface_id(grant),
                "elapsedMs": started.elapsed().as_millis() as u64,
            })
        } else {
            json!({
                "ok": true,
                "queued": true,
                "id": ids.first(),
                "webviewLabel": label,
                "surfaceId": grant_surface_id(grant),
                "elapsedMs": started.elapsed().as_millis() as u64,
            })
        };
        return Ok(CallToolResult::success(vec![Content::text(body.to_string())]));
    }

    let (results, last_url, complete) =
        wait_for_drive_results(dir, ids, wait_timeout_ms, poll_ms);
    let steps: Vec<Value> = ids
        .iter()
        .map(|id| {
            results
                .iter()
                .find(|ev| event_action_id(ev) == Some(id.as_str()))
                .cloned()
                .unwrap_or_else(|| {
                    json!({
                        "kind": "pending",
                        "payload": { "id": id, "ok": false, "error": "no result yet" }
                    })
                })
        })
        .collect();
    let all_ok = complete
        && steps.iter().all(|ev| {
            ev.get("kind").and_then(|v| v.as_str()) == Some("drive")
                && ev
                    .get("payload")
                    .and_then(|p| p.get("ok"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
        });
    let mut snapshot = Value::Null;
    if include_snapshot {
        let after = request_snapshot(dir, pubkey, false);
        if let Some(ev) = wait_for_snapshot(dir, after, 15_000, 40) {
            snapshot = ev.get("payload").cloned().unwrap_or(Value::Null);
        }
    }
    let body = json!({
        "ok": all_ok,
        "queued": true,
        "batch": batch,
        "ids": ids,
        "count": ids.len(),
        "complete": complete,
        "results": steps,
        "url": last_url,
        "webviewLabel": label,
        "surfaceId": grant_surface_id(grant),
        "elapsedMs": started.elapsed().as_millis() as u64,
        "snapshot": snapshot,
    });
    Ok(CallToolResult::success(vec![Content::text(body.to_string())]))
}

pub fn drive(p: DriveParams) -> Result<CallToolResult, ErrorData> {
    drive_with_poll(p, 50)
}

fn drive_with_poll(p: DriveParams, poll_ms: u64) -> Result<CallToolResult, ErrorData> {
    let _ = poll_ms;
    let Some(pubkey) = caller_pubkey() else {
        return Err(ErrorData::invalid_params(
            "BUZZ_AGENT_PUBKEY required for browser_drive",
            None,
        ));
    };
    let (dir, grant, label) = require_drive_grant_resolved(
        &pubkey,
        &p.webview_label,
        p.surface_id.as_deref(),
    )?;
    let queue_only = p.queue_only.unwrap_or(false);
    let wait_timeout_ms = p.wait_timeout_ms.unwrap_or(10_000).min(60_000);
    let include_snapshot = p.include_snapshot.unwrap_or(false);

    if let Some(batch) = p.actions.as_ref() {
        if batch.is_empty() {
            return Err(ErrorData::invalid_params(
                "actions array is empty",
                None,
            ));
        }
        let mut ids = Vec::new();
        let mut parsed = Vec::new();
        for (i, raw) in batch.iter().enumerate() {
            match parse_drive_action(raw) {
                Ok(a) => parsed.push(a),
                Err(e) => {
                    return Err(ErrorData::invalid_params(
                        format!("actions[{i}]: {e}"),
                        None,
                    ));
                }
            }
        }
        for action in parsed {
            ids.push(queue_action(&dir, &pubkey, action)?);
        }
        return drive_wait_response(
            &dir,
            &label,
            &grant,
            &ids,
            true,
            queue_only,
            wait_timeout_ms,
            poll_ms,
            include_snapshot,
            &pubkey,
        );
    }

    let action = parse_drive_action(&p.action).map_err(|e| {
        ErrorData::invalid_params(e, None)
    })?;
    let id = queue_action(&dir, &pubkey, action)?;
    drive_wait_response(
        &dir,
        &label,
        &grant,
        &[id],
        false,
        queue_only,
        wait_timeout_ms,
        poll_ms,
        include_snapshot,
        &pubkey,
    )
}


fn event_kind(ev: &Value) -> Option<&str> {
    ev.get("kind").and_then(|v| v.as_str())
}

/// Byte-offset watermark into events.jsonl. Survives Desktop id resets after
/// restart (append of low ids after stale high ids) — id-only after_id misses.
#[derive(Debug, Clone, Copy)]
struct EventsWatermark {
    file_len: u64,
}

fn events_file_len(dir: &Path) -> u64 {
    fs::metadata(dir.join("events.jsonl"))
        .map(|m| m.len())
        .unwrap_or(0)
}

fn events_watermark(dir: &Path) -> EventsWatermark {
    EventsWatermark {
        file_len: events_file_len(dir),
    }
}

/// Read events.jsonl lines appended after `after.file_len`.
fn read_events_after(dir: &Path, after: EventsWatermark) -> Vec<Value> {
    use std::io::{Read, Seek, SeekFrom};
    let path = dir.join("events.jsonl");
    let Ok(mut file) = fs::File::open(&path) else {
        return Vec::new();
    };
    if after.file_len > 0 {
        if file.seek(SeekFrom::Start(after.file_len)).is_err() {
            return Vec::new();
        }
    }
    let mut buf = String::new();
    if file.read_to_string(&mut buf).is_err() {
        return Vec::new();
    }
    // If we seek mid-line (rare truncation), drop the partial first line.
    let body = if after.file_len > 0 && !buf.is_empty() && !buf.starts_with('{') {
        match buf.find('\n') {
            Some(i) => &buf[i + 1..],
            None => return Vec::new(),
        }
    } else {
        buf.as_str()
    };
    body.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn wait_for_snapshot(
    dir: &Path,
    after: EventsWatermark,
    timeout_ms: u64,
    poll_ms: u64,
) -> Option<Value> {
    let started = std::time::Instant::now();
    loop {
        for ev in read_events_after(dir, after) {
            if event_kind(&ev) == Some("snapshot") {
                return Some(ev);
            }
        }
        if started.elapsed().as_millis() as u64 >= timeout_ms {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(poll_ms.max(10)));
    }
}

fn request_snapshot(dir: &Path, pubkey: &str, screenshot: bool) -> EventsWatermark {
    let after = events_watermark(dir);
    let _ = fs::write(
        dir.join("snapshot-request.json"),
        json!({
            "agentPubkey": pubkey,
            "screenshot": screenshot,
            "atMs": now_ms(),
        })
        .to_string(),
    );
    let _ = fs::write(dir.join("drive-wake"), format!("{}\n", now_ms()));
    after
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SnapshotParams {
    /// Live webview label (fragile across popout). Prefer `surface_id`.
    #[serde(default)]
    pub webview_label: String,
    /// Stable playground surface id — preferred target across detach/popout.
    #[serde(default)]
    pub surface_id: Option<String>,
    /// When true, ask Desktop to attach a PNG via existing capture (best-effort; may be omitted).
    #[serde(default)]
    pub screenshot: Option<bool>,
}

/// Request a DOM snapshot and wait for Desktop `kind=snapshot` (inline payload).
pub fn snapshot(p: SnapshotParams) -> Result<CallToolResult, ErrorData> {
    let Some(pubkey) = caller_pubkey() else {
        return Err(ErrorData::invalid_params(
            "BUZZ_AGENT_PUBKEY required for browser_snapshot",
            None,
        ));
    };
    let (dir, grant, label) = resolve_grant_target(
        &pubkey,
        &p.webview_label,
        p.surface_id.as_deref(),
    )?;

    let want_shot = p.screenshot.unwrap_or(false);
    let started = std::time::Instant::now();
    let after = request_snapshot(&dir, &pubkey, want_shot);
    // Desktop usually ACKs in ≤2s; keep headroom for parked background paint.
    let timeout_ms = if want_shot { 20_000 } else { 15_000 };
    let snap_ev = wait_for_snapshot(&dir, after, timeout_ms, 40);

    let mut last_url = Value::Null;
    let mut last_title = Value::Null;
    if let Ok(file) = fs::File::open(dir.join("events.jsonl")) {
        for line in BufReader::new(file).lines().flatten() {
            if let Ok(ev) = serde_json::from_str::<Value>(&line) {
                if event_kind(&ev) == Some("nav") {
                    if let Some(payload) = ev.get("payload") {
                        if let Some(u) = payload.get("url") {
                            last_url = u.clone();
                        }
                        if let Some(t) = payload.get("title") {
                            last_title = t.clone();
                        }
                    }
                }
            }
        }
    }

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let (ok, snapshot_payload) = match snap_ev {
        Some(ev) => (true, ev.get("payload").cloned().unwrap_or(Value::Null)),
        None => (
            false,
            json!({"ok": false, "error": "snapshot timeout — Desktop may be closed or grant webview missing"}),
        ),
    };

    Ok(CallToolResult::success(vec![Content::text(
        json!({
            "ok": ok,
            "webviewLabel": label,
            "surfaceId": grant_surface_id(&grant),
            "grant": grant,
            "url": last_url,
            "title": last_title,
            "screenshotRequested": want_shot,
            "snapshot": snapshot_payload,
            "elapsedMs": elapsed_ms,
            "note": "Inline snapshot payload. Prefer surfaceId on next drive/fill."
        })
        .to_string(),
    )]))
}


// ── Drive viewport recording ────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecordStartParams {
    #[serde(default)]
    pub webview_label: Option<String>,
    #[serde(default)]
    pub surface_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecordStopParams {
    #[serde(default)]
    pub webview_label: Option<String>,
    #[serde(default)]
    pub surface_id: Option<String>,
    /// Optional short note included in the posted caption (max ~280 chars on Desktop).
    #[serde(default)]
    pub caption: Option<String>,
    /// Max ms to wait for encode+upload+post (default 90000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

fn new_record_id() -> String {
    format!(
        "r{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    )
}

fn write_record_request(dir: &Path, body: &Value) -> Result<(), ErrorData> {
    fs::create_dir_all(dir).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    let path = dir.join("record-request.json");
    fs::write(&path, body.to_string())
        .map_err(|e| ErrorData::internal_error(format!("write record-request: {e}"), None))?;
    let _ = fs::write(dir.join("drive-wake"), format!("{}\n", now_ms()));
    Ok(())
}

fn wait_for_record_event(
    dir: &Path,
    after: EventsWatermark,
    request_id: &str,
    kinds: &[&str],
    timeout_ms: u64,
    poll_ms: u64,
) -> Option<Value> {
    let started = std::time::Instant::now();
    loop {
        for ev in read_events_after(dir, after) {
            let kind = event_kind(&ev).unwrap_or("");
            if !kinds.iter().any(|k| *k == kind) {
                continue;
            }
            let rid = ev
                .get("payload")
                .and_then(|p| p.get("requestId"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if rid == request_id {
                return Some(ev);
            }
        }
        if started.elapsed().as_millis() as u64 >= timeout_ms {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(poll_ms.max(20)));
    }
}

/// Start Drive WKWebView viewport recording for a grant (MCP → Desktop).
pub fn record_start(p: RecordStartParams) -> Result<CallToolResult, ErrorData> {
    let Some(pubkey) = caller_pubkey() else {
        return Err(ErrorData::invalid_params(
            "BUZZ_AGENT_PUBKEY required for browser_record_start",
            None,
        ));
    };
    let (dir, grant, label) = require_drive_grant_resolved(
        &pubkey,
        p.webview_label.as_deref().unwrap_or(""),
        p.surface_id.as_deref(),
    )?;
    let request_id = new_record_id();
    let after = events_watermark(&dir);
    write_record_request(
        &dir,
        &json!({
            "action": "start",
            "requestId": request_id,
            "agentPubkey": pubkey,
            "atMs": now_ms(),
        }),
    )?;
    let ev = wait_for_record_event(
        &dir,
        after,
        &request_id,
        &["record_started", "record_error"],
        15_000,
        40,
    );
    let (ok, payload) = match ev {
        Some(ev) => {
            let kind = event_kind(&ev).unwrap_or("");
            let payload = ev.get("payload").cloned().unwrap_or(Value::Null);
            (kind == "record_started", payload)
        }
        None => (
            false,
            json!({
                "ok": false,
                "error": "record_start timeout — Desktop may be closed or grant webview missing",
            }),
        ),
    };
    Ok(CallToolResult::success(vec![Content::text(
        json!({
            "ok": ok,
            "requestId": request_id,
            "webviewLabel": label,
            "surfaceId": grant_surface_id(&grant),
            "grant": grant,
            "result": payload,
            "note": "Opt-in section recording (~4 fps; safety max ~180s). Call browser_record_stop_and_post when the section is done (ends early) to encode MP4 and post to the grant channel/thread. Do not start unless the runbook/user asked for this clip."
        })
        .to_string(),
    )]))
}

/// Stop Drive recording, encode MP4, upload, and post to the grant channel/thread.
pub fn record_stop_and_post(p: RecordStopParams) -> Result<CallToolResult, ErrorData> {
    let Some(pubkey) = caller_pubkey() else {
        return Err(ErrorData::invalid_params(
            "BUZZ_AGENT_PUBKEY required for browser_record_stop_and_post",
            None,
        ));
    };
    let (dir, grant, label) = require_drive_grant_resolved(
        &pubkey,
        p.webview_label.as_deref().unwrap_or(""),
        p.surface_id.as_deref(),
    )?;
    let request_id = new_record_id();
    let after = events_watermark(&dir);
    let mut body = json!({
        "action": "stop",
        "requestId": request_id,
        "agentPubkey": pubkey,
        "atMs": now_ms(),
    });
    if let Some(caption) = p
        .caption
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("caption".into(), json!(caption));
        }
    }
    write_record_request(&dir, &body)?;
    let timeout_ms = p.timeout_ms.unwrap_or(90_000).clamp(5_000, 180_000);
    let ev = wait_for_record_event(
        &dir,
        after,
        &request_id,
        &["record_posted", "record_error"],
        timeout_ms,
        80,
    );
    let (ok, payload) = match ev {
        Some(ev) => {
            let kind = event_kind(&ev).unwrap_or("");
            let payload = ev.get("payload").cloned().unwrap_or(Value::Null);
            (kind == "record_posted", payload)
        }
        None => (
            false,
            json!({
                "ok": false,
                "error": "record_stop timeout — encode/upload may still be running; poll browser_observe_poll for record_posted|record_error",
            }),
        ),
    };
    Ok(CallToolResult::success(vec![Content::text(
        json!({
            "ok": ok,
            "requestId": request_id,
            "webviewLabel": label,
            "surfaceId": grant_surface_id(&grant),
            "channelId": grant.get("channelId"),
            "threadRoot": grant.get("threadRoot"),
            "result": payload,
            "note": "On success Desktop posted ![video](url) as the grant agent into the bound channel/thread."
        })
        .to_string(),
    )]))
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn with_env<T>(dir: &Path, pubkey: &str, f: impl FnOnce() -> T) -> T {
        TEST_AGENT_DIR.with(|c| *c.borrow_mut() = Some(dir.to_path_buf()));
        TEST_PUBKEY.with(|c| *c.borrow_mut() = Some(pubkey.to_string()));
        let out = f();
        TEST_AGENT_DIR.with(|c| *c.borrow_mut() = None);
        TEST_PUBKEY.with(|c| *c.borrow_mut() = None);
        out
    }

    #[test]
    fn observe_poll_respects_grant() {
        let dir = tempdir().unwrap();
        let pubkey = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let label = "playground-demo";
        let gdir = dir.path().join(label);
        fs::create_dir_all(&gdir).unwrap();
        fs::write(
            gdir.join("grant.json"),
            format!(
                r#"{{"agentPubkey":"{pubkey}","mode":"observe","webviewLabel":"{label}"}}"#
            ),
        )
        .unwrap();
        let mut events = fs::File::create(gdir.join("events.jsonl")).unwrap();
        writeln!(
            events,
            r#"{{"id":1,"webviewLabel":"{label}","kind":"console","atMs":1}}"#
        )
        .unwrap();
        let result = with_env(dir.path(), pubkey, || {
            observe_poll(ObservePollParams {
                webview_label: label.into(),
                surface_id: None,
                after_id: Some(0),
                limit: Some(10),
            })
            .unwrap()
        });
        let text = format!("{result:?}");
        assert!(text.contains("console"), "{text}");
    }


    #[test]
    fn wait_for_snapshot_sees_events_after_id_reset_via_byte_watermark() {
        let dir = tempdir().unwrap();
        let gdir = dir.path().join("playground-demo");
        fs::create_dir_all(&gdir).unwrap();
        // Stale high ids from a prior Desktop session.
        let mut events = fs::File::create(gdir.join("events.jsonl")).unwrap();
        writeln!(
            events,
            r#"{{"id":100,"webviewLabel":"playground-demo","kind":"nav","atMs":1}}"#
        )
        .unwrap();
        writeln!(
            events,
            r#"{{"id":101,"webviewLabel":"playground-demo","kind":"console","atMs":2}}"#
        )
        .unwrap();
        events.flush().unwrap();
        let after = events_watermark(&gdir);
        // Desktop restart: new low ids appended (the old id-only waiter would miss).
        writeln!(
            events,
            r#"{{"id":1,"webviewLabel":"playground-demo","kind":"grant","atMs":3}}"#
        )
        .unwrap();
        writeln!(
            events,
            r#"{{"id":2,"webviewLabel":"playground-demo","kind":"snapshot","atMs":4,"payload":{{"ok":true}}}}"#
        )
        .unwrap();
        events.flush().unwrap();
        let ev = wait_for_snapshot(&gdir, after, 200, 10).expect("snapshot");
        assert_eq!(event_kind(&ev), Some("snapshot"));
        assert_eq!(ev.get("id").and_then(|v| v.as_u64()), Some(2));
    }

    #[test]
    fn wait_for_record_event_matches_request_id_after_id_reset() {
        let dir = tempdir().unwrap();
        let gdir = dir.path().join("playground-demo");
        fs::create_dir_all(&gdir).unwrap();
        let mut events = fs::File::create(gdir.join("events.jsonl")).unwrap();
        writeln!(
            events,
            r#"{{"id":50,"webviewLabel":"playground-demo","kind":"nav","atMs":1}}"#
        )
        .unwrap();
        events.flush().unwrap();
        let after = events_watermark(&gdir);
        writeln!(
            events,
            r#"{{"id":1,"webviewLabel":"playground-demo","kind":"record_started","atMs":2,"payload":{{"ok":true,"requestId":"rABC"}}}}"#
        )
        .unwrap();
        events.flush().unwrap();
        let ev = wait_for_record_event(
            &gdir,
            after,
            "rABC",
            &["record_started", "record_error"],
            200,
            10,
        )
        .expect("record_started");
        assert_eq!(event_kind(&ev), Some("record_started"));
    }

    #[test]
    fn validate_set_viewport_desktop_responsive_mobile() {
        let desktop = validate_set_viewport_params(&SetViewportParams {
            mode: "desktop".into(),
            webview_label: None,
            surface_id: None,
            width: None,
            height: None,
            device_id: None,
            orientation: None,
            scale_percent: None,
        })
        .unwrap();
        assert_eq!(desktop.get("mode").and_then(|v| v.as_str()), Some("desktop"));

        let responsive = validate_set_viewport_params(&SetViewportParams {
            mode: "responsive".into(),
            webview_label: None,
            surface_id: None,
            width: Some(100.0),
            height: Some(200.0),
            device_id: None,
            orientation: None,
            scale_percent: None,
        })
        .unwrap();
        assert_eq!(responsive.get("width").and_then(|v| v.as_i64()), Some(320));
        assert_eq!(responsive.get("height").and_then(|v| v.as_i64()), Some(320));

        let mobile = validate_set_viewport_params(&SetViewportParams {
            mode: "mobile".into(),
            webview_label: None,
            surface_id: None,
            width: None,
            height: None,
            device_id: Some("iphone-16".into()),
            orientation: Some("landscape".into()),
            scale_percent: Some(75.0),
        })
        .unwrap();
        assert_eq!(mobile.get("deviceId").and_then(|v| v.as_str()), Some("iphone-16"));
        assert_eq!(
            mobile.get("orientation").and_then(|v| v.as_str()),
            Some("landscape")
        );
        assert_eq!(mobile.get("scalePercent").and_then(|v| v.as_i64()), Some(75));
    }

    #[test]
    fn validate_set_viewport_rejects_bad_mode_and_device() {
        assert!(validate_set_viewport_params(&SetViewportParams {
            mode: "tablet".into(),
            webview_label: None,
            surface_id: None,
            width: None,
            height: None,
            device_id: None,
            orientation: None,
            scale_percent: None,
        })
        .is_err());
        assert!(validate_set_viewport_params(&SetViewportParams {
            mode: "mobile".into(),
            webview_label: None,
            surface_id: None,
            width: None,
            height: None,
            device_id: Some("nokia".into()),
            orientation: None,
            scale_percent: None,
        })
        .is_err());
    }

    #[test]
    fn set_viewport_requires_drive_and_queues_request() {
        let dir = tempdir().unwrap();
        let pubkey = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let label = "playground-vp";
        let gdir = dir.path().join(label);
        fs::create_dir_all(&gdir).unwrap();
        fs::write(
            gdir.join("grant.json"),
            format!(
                r#"{{"agentPubkey":"{pubkey}","mode":"observe","webviewLabel":"{label}","surfaceId":"sid-vp"}}"#
            ),
        )
        .unwrap();
        let err = with_env(dir.path(), pubkey, || {
            set_viewport(SetViewportParams {
                mode: "responsive".into(),
                webview_label: Some(label.into()),
                surface_id: None,
                width: Some(414.0),
                height: Some(896.0),
                device_id: None,
                orientation: None,
                scale_percent: None,
            })
            .unwrap_err()
        });
        let msg = format!("{err:?}");
        assert!(msg.contains("Drive"), "{msg}");

        fs::write(
            gdir.join("grant.json"),
            format!(
                r#"{{"agentPubkey":"{pubkey}","mode":"drive","webviewLabel":"{label}","surfaceId":"sid-vp"}}"#
            ),
        )
        .unwrap();
        let result = with_env(dir.path(), pubkey, || {
            set_viewport(SetViewportParams {
                mode: "mobile".into(),
                webview_label: Some(label.into()),
                surface_id: None,
                width: None,
                height: None,
                device_id: Some("pixel-8".into()),
                orientation: Some("portrait".into()),
                scale_percent: Some(100.0),
            })
            .unwrap()
        });
        let text = format!("{result:?}");
        assert!(text.contains("queued"), "{text}");
        let req = fs::read_to_string(gdir.join("viewport-request.json")).unwrap();
        assert!(req.contains("pixel-8"), "{req}");
        assert!(req.contains("mobile"), "{req}");
    }

    #[test]
    fn parse_action_object_and_string() {
        let obj = json!({"kind":"key","key":"Enter"});
        let a = parse_drive_action(&obj).unwrap();
        assert_eq!(a.kind, "key");

        let s = Value::String(r#"{"kind":"click","x":1,"y":2}"#.into());
        let b = parse_drive_action(&s).unwrap();
        assert_eq!(b.kind, "click");
    }

    #[test]
    fn parse_rejects_type_field_and_unknown_kind() {
        let bad = json!({"type":"click","x":1,"y":2});
        let err = parse_drive_action(&bad).unwrap_err();
        assert!(err.contains("kind"), "{err}");

        let unk = json!({"kind":"teleport"});
        assert!(parse_drive_action(&unk).unwrap_err().contains("unknown"));
    }

    #[test]
    fn drive_rejects_invalid_action_without_queueing() {
        let dir = tempdir().unwrap();
        let pubkey = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let label = "playground-reject";
        let gdir = dir.path().join(label);
        fs::create_dir_all(&gdir).unwrap();
        fs::write(
            gdir.join("grant.json"),
            format!(
                r#"{{"agentPubkey":"{pubkey}","mode":"drive","webviewLabel":"{label}"}}"#
            ),
        )
        .unwrap();
        let err = with_env(dir.path(), pubkey, || {
            drive(DriveParams {
                webview_label: label.into(),
                surface_id: None,
                action: json!({"kind":"nope"}),
                actions: None,
                queue_only: Some(true),
                wait_timeout_ms: None,
                include_snapshot: None,
            })
            .unwrap_err()
        });
        let msg = format!("{err:?}");
        assert!(msg.contains("unknown") || msg.contains("invalid"), "{msg}");
        assert!(
            !gdir.join("drive-inbox.jsonl").exists(),
            "invalid action must not create inbox"
        );
    }

    #[test]
    fn drive_queues_valid_key_action() {
        let dir = tempdir().unwrap();
        let pubkey = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let label = "playground-key";
        let gdir = dir.path().join(label);
        fs::create_dir_all(&gdir).unwrap();
        fs::write(
            gdir.join("grant.json"),
            format!(
                r#"{{"agentPubkey":"{pubkey}","mode":"drive","webviewLabel":"{label}"}}"#
            ),
        )
        .unwrap();
        let result = with_env(dir.path(), pubkey, || {
            drive(DriveParams {
                webview_label: label.into(),
                surface_id: None,
                action: json!({"kind":"key","key":"Enter"}),
                actions: None,
                queue_only: Some(true),
                wait_timeout_ms: None,
                include_snapshot: None,
            })
            .unwrap()
        });
        let text = format!("{result:?}");
        assert!(text.contains("queued"), "{text}");
        let inbox = fs::read_to_string(gdir.join("drive-inbox.jsonl")).unwrap();
        assert!(inbox.contains("Enter"), "{inbox}");
        assert!(inbox.contains("key"), "{inbox}");
    }

    #[test]
    fn drive_string_action_parsed() {
        let dir = tempdir().unwrap();
        let pubkey = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let label = "playground-s";
        let gdir = dir.path().join(label);
        fs::create_dir_all(&gdir).unwrap();
        fs::write(
            gdir.join("grant.json"),
            format!(
                r#"{{"agentPubkey":"{pubkey}","mode":"drive","webviewLabel":"{label}"}}"#
            ),
        )
        .unwrap();
        with_env(dir.path(), pubkey, || {
            drive(DriveParams {
                webview_label: label.into(),
                surface_id: None,
                action: Value::String(r#"{"kind":"scroll","dy":100}"#.into()),
                actions: None,
                queue_only: Some(true),
                wait_timeout_ms: None,
                include_snapshot: None,
            })
            .unwrap()
        });
        let inbox = fs::read_to_string(gdir.join("drive-inbox.jsonl")).unwrap();
        assert!(inbox.contains("scroll"), "{inbox}");
    }

    #[test]
    fn drive_batch_queues_all_or_rejects() {
        let dir = tempdir().unwrap();
        let pubkey = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let label = "playground-batch";
        let gdir = dir.path().join(label);
        fs::create_dir_all(&gdir).unwrap();
        fs::write(
            gdir.join("grant.json"),
            format!(
                r#"{{"agentPubkey":"{pubkey}","mode":"drive","webviewLabel":"{label}"}}"#
            ),
        )
        .unwrap();
        with_env(dir.path(), pubkey, || {
            let err = drive(DriveParams {
                webview_label: label.into(),
                surface_id: None,
                action: json!({}),
                actions: Some(vec![
                    json!({"kind":"scroll","dy":1}),
                    json!({"kind":"nope"}),
                ]),
                queue_only: Some(true),
                wait_timeout_ms: None,
                include_snapshot: None,
            })
            .unwrap_err();
            assert!(format!("{err:?}").contains("actions[1]"));
            // First action must not have been queued when batch validation fails early.
            assert!(!gdir.join("drive-inbox.jsonl").exists());

            drive(DriveParams {
                webview_label: label.into(),
                surface_id: None,
                action: json!({}),
                actions: Some(vec![
                    json!({"kind":"key","key":"Tab"}),
                    json!({"kind":"scroll","dy":10}),
                ]),
                queue_only: Some(true),
                wait_timeout_ms: None,
                include_snapshot: None,
            })
            .unwrap();
        });
        let inbox = fs::read_to_string(gdir.join("drive-inbox.jsonl")).unwrap();
        assert!(inbox.contains("Tab") && inbox.contains("scroll"), "{inbox}");
    }

    #[test]
    fn resolve_prefers_surface_id_over_stale_label() {
        let dir = tempdir().unwrap();
        let pubkey = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        let stale = "playground-demo";
        let live = "playground-demo--pop";
        // Only the live mirror remains after Desktop rebind (old grant.json removed).
        let gdir = dir.path().join(live);
        fs::create_dir_all(&gdir).unwrap();
        fs::write(
            gdir.join("grant.json"),
            format!(
                r#"{{"agentPubkey":"{pubkey}","mode":"drive","webviewLabel":"{live}","surfaceId":"demo"}}"#
            ),
        )
        .unwrap();
        with_env(dir.path(), pubkey, || {
            let (resolved_dir, grant, label) =
                resolve_grant_target(pubkey, stale, Some("demo")).unwrap();
            assert_eq!(label, live);
            assert_eq!(grant_surface_id(&grant), Some("demo"));
            assert!(resolved_dir.ends_with(live));
        });
    }

    #[test]
    fn drive_wait_returns_results_from_events_fixture() {
        let dir = tempdir().unwrap();
        let pubkey = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        let label = "playground-wait";
        let gdir = dir.path().join(label);
        fs::create_dir_all(&gdir).unwrap();
        fs::write(
            gdir.join("grant.json"),
            format!(
                r#"{{"agentPubkey":"{pubkey}","mode":"drive","webviewLabel":"{label}","surfaceId":"wait1"}}"#
            ),
        )
        .unwrap();

        // Background: after a short delay, write drive results for queued ids.
        let events_path = gdir.join("events.jsonl");
        let events_path2 = events_path.clone();
        let handle = std::thread::spawn(move || {
            // Wait until inbox exists with an id.
            for _ in 0..200 {
                if let Ok(inbox) = fs::read_to_string(gdir.join("drive-inbox.jsonl")) {
                    if let Some(line) = inbox.lines().next() {
                        if let Ok(v) = serde_json::from_str::<Value>(line) {
                            if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
                                let mut f = fs::OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open(&events_path2)
                                    .unwrap();
                                writeln!(
                                    f,
                                    r#"{{"id":1,"webviewLabel":"playground-wait","kind":"drive","atMs":1,"payload":{{"id":"{id}","ok":true,"kind":"scroll","url":"https://example.com/done"}}}}"#
                                )
                                .unwrap();
                                return;
                            }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            panic!("inbox never appeared");
        });

        let result = with_env(dir.path(), pubkey, || {
            drive_with_poll(
                DriveParams {
                    webview_label: label.into(),
                    surface_id: None,
                    action: json!({"kind":"scroll","dy":1}),
                    actions: None,
                    queue_only: Some(false),
                    wait_timeout_ms: Some(500),
                    include_snapshot: None,
                },
                5,
            )
            .unwrap()
        });
        handle.join().unwrap();
        let text = format!("{result:?}");
        assert!(text.contains("complete"), "{text}");
        assert!(text.contains("example.com/done") || text.contains("scroll"), "{text}");
        assert!(text.contains("\"ok\":true") || text.contains("ok: true") || text.contains("results"), "{text}");
    }

    #[test]
    fn collect_drive_results_reads_fixture_without_wall_clock() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"id":1,"kind":"drive","payload":{{"id":"a1","ok":true,"kind":"click","url":"https://x.test"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"id":2,"kind":"drive_error","payload":{{"id":"a2","ok":false,"kind":"type","error":"boom"}}}}"#
        )
        .unwrap();
        let wanted = vec!["a1".into(), "a2".into()];
        let (results, url) = collect_drive_results_from_events(&path, &wanted);
        assert_eq!(results.len(), 2);
        assert_eq!(url.as_deref(), Some("https://x.test"));
        let (partial, _) = collect_drive_results_from_events(&path, &["a1".into(), "missing".into()]);
        assert_eq!(partial.len(), 1);
    }

    #[test]
    fn drive_queue_only_skips_wait() {
        let dir = tempdir().unwrap();
        let pubkey = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let label = "playground-qo";
        let gdir = dir.path().join(label);
        fs::create_dir_all(&gdir).unwrap();
        fs::write(
            gdir.join("grant.json"),
            format!(
                r#"{{"agentPubkey":"{pubkey}","mode":"drive","webviewLabel":"{label}"}}"#
            ),
        )
        .unwrap();
        let result = with_env(dir.path(), pubkey, || {
            drive(DriveParams {
                webview_label: label.into(),
                surface_id: None,
                action: json!({"kind":"key","key":"Tab"}),
                actions: None,
                queue_only: Some(true),
                wait_timeout_ms: Some(10),
                include_snapshot: None,
            })
            .unwrap()
        });
        let text = format!("{result:?}");
        assert!(text.contains("queued"), "{text}");
        assert!(!text.contains("complete"), "{text}");
    }
    #[test]
    fn tabs_reads_mirrored_tabs_json() {
        let dir = tempdir().unwrap();
        let pubkey = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let label = "playground-demo";
        let gdir = dir.path().join(label);
        fs::create_dir_all(&gdir).unwrap();
        fs::write(
            gdir.join("grant.json"),
            format!(
                r#"{{"agentPubkey":"{pubkey}","mode":"observe","webviewLabel":"{label}","surfaceId":"demo","surface":"playground"}}"#
            ),
        )
        .unwrap();
        fs::write(
            gdir.join("tabs.json"),
            r#"{"browserId":"demo","mainTabSid":"demo","activeTabSid":"extra","tabs":[{"surfaceId":"demo","url":"https://a.example","title":"Main","isMain":true},{"surfaceId":"extra","url":"https://b.example","title":"Extra","isMain":false}]}"# ,
        )
        .unwrap();
        let result = with_env(dir.path(), pubkey, || {
            tabs(TabsParams {
                webview_label: Some(label.into()),
                surface_id: None,
            })
            .unwrap()
        });
        let text = format!("{result:?}");
        assert!(text.contains("mainTabSid"), "{text}");
        assert!(text.contains("extra"), "{text}");
        assert!(text.contains("isMain"), "{text}");
    }

}



#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunbookGetParams {
    /// Live webview label (fragile across popout). Prefer `surface_id`.
    #[serde(default)]
    pub webview_label: String,
    #[serde(default)]
    pub surface_id: Option<String>,
    /// When set, return that procedure's full steps from runbook-full.json.
    #[serde(default)]
    pub procedure_id: Option<String>,
}

/// Return runbook inject index, or one procedure's full steps.
pub fn runbook_get(p: RunbookGetParams) -> Result<CallToolResult, ErrorData> {
    let Some(pubkey) = caller_pubkey() else {
        return Err(ErrorData::invalid_params(
            "BUZZ_AGENT_PUBKEY required for browser_runbook_get",
            None,
        ));
    };
    let (dir, grant, label) = resolve_grant_target(
        &pubkey,
        &p.webview_label,
        p.surface_id.as_deref(),
    )?;
    let inject = read_json(&dir.join("runbook.json")).unwrap_or_else(|| json!({
        "agentBrief": "",
        "procedures": []
    }));
    if let Some(proc_id) = p.procedure_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let full = read_json(&dir.join("runbook-full.json")).unwrap_or(json!({}));
        let procedures = full
            .get("procedures")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let found = procedures.into_iter().find(|entry| {
            entry
                .get("id")
                .and_then(|v| v.as_str())
                .map(|id| id == proc_id)
                .unwrap_or(false)
        });
        let Some(procedure) = found else {
            return Err(ErrorData::invalid_params(
                format!("procedure_id={proc_id} not found in runbook"),
                None,
            ));
        };
        let body = json!({
            "webviewLabel": label,
            "surfaceId": grant_surface_id(&grant),
            "procedure": procedure,
        });
        return Ok(CallToolResult::success(vec![Content::text(body.to_string())]));
    }
    let body = json!({
        "webviewLabel": label,
        "surfaceId": grant_surface_id(&grant),
        "runbook": inject,
        "note": "Pass procedure_id to fetch full steps for one active/pending/archived entry."
    });
    Ok(CallToolResult::success(vec![Content::text(body.to_string())]))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunbookProposeParams {
    /// Live webview label (fragile across popout). Prefer `surface_id`.
    #[serde(default)]
    pub webview_label: String,
    #[serde(default)]
    pub surface_id: Option<String>,
    pub title: String,
    pub steps: String,
}

/// Queue an agent-authored procedure. Desktop auto-activates it unless a
/// human-persisted procedure with the same title blocks the write.
pub fn runbook_propose(p: RunbookProposeParams) -> Result<CallToolResult, ErrorData> {
    let Some(pubkey) = caller_pubkey() else {
        return Err(ErrorData::invalid_params(
            "BUZZ_AGENT_PUBKEY required for browser_runbook_propose",
            None,
        ));
    };
    let title = p.title.trim().to_string();
    if title.is_empty() {
        return Err(ErrorData::invalid_params("title is required", None));
    }
    let steps = p.steps;
    if steps.trim().is_empty() {
        return Err(ErrorData::invalid_params(
            "steps markdown is required for browser_runbook_propose",
            None,
        ));
    }
    let (dir, grant, label) = resolve_grant_target(
        &pubkey,
        &p.webview_label,
        p.surface_id.as_deref(),
    )?;
    // Reject early when mirrored full runbook marks this title as persisted.
    if let Some(full) = read_json(&dir.join("runbook-full.json")) {
        if let Some(procs) = full.get("procedures").and_then(|v| v.as_array()) {
            let title_l = title.to_ascii_lowercase();
            for proc in procs {
                let persisted = proc
                    .get("persisted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !persisted {
                    continue;
                }
                let existing = proc
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                if existing == title_l {
                    return Err(ErrorData::invalid_params(
                        format!(
                            "procedure {title:?} is persisted (human lock); agents cannot modify it"
                        ),
                        None,
                    ));
                }
            }
        }
    }
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("runbook-propose.jsonl");
    let line = json!({
        "title": title,
        "steps": steps,
        "status": "active",
        "autoActivate": true,
        "sourceAgent": pubkey,
        "sourceChannel": grant.get("channelId").and_then(|v| v.as_str()),
        "surfaceId": grant_surface_id(&grant),
        "atMs": now_ms(),
    });
    let _guard = inbox_lock_acquire(&dir).map_err(|e| ErrorData::internal_error(e, None))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    writeln!(file, "{line}")
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    file.flush()
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    drop(_guard);
    let _ = fs::write(dir.join("drive-wake"), format!("{}
", now_ms()));
    let _ = fs::write(dir.join("runbook-propose-wake"), format!("{}
", now_ms()));
    let body = json!({
        "ok": true,
        "queued": true,
        "status": "active",
        "autoActivate": true,
        "webviewLabel": label,
        "surfaceId": grant_surface_id(&grant),
        "title": title,
        "note": "Desktop auto-activates agent procedures. Persisted (human-locked) titles are rejected. Agent brief is human-owned."
    });
    Ok(CallToolResult::success(vec![Content::text(body.to_string())]))
}



#[derive(Debug, Deserialize, JsonSchema)]
pub struct FillFieldParams {
    #[serde(default)]
    pub webview_label: String,
    #[serde(default)]
    pub surface_id: Option<String>,
    pub text: String,
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default, rename = "ref")]
    #[schemars(description = "Snapshot interactive ref from browser_snapshot (e.g. e0)")]
    pub ref_id: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub clear: Option<bool>,
    #[serde(default)]
    pub queue_only: Option<bool>,
    #[serde(default)]
    pub wait_timeout_ms: Option<u64>,
    #[serde(default)]
    pub include_snapshot: Option<bool>,
}

/// One-shot click + type + verify for a form field.
pub fn fill_field(p: FillFieldParams) -> Result<CallToolResult, ErrorData> {
    let Some(pubkey) = caller_pubkey() else {
        return Err(ErrorData::invalid_params(
            "BUZZ_AGENT_PUBKEY required for browser_fill_field",
            None,
        ));
    };
    let (dir, grant, label) = require_drive_grant_resolved(
        &pubkey,
        &p.webview_label,
        p.surface_id.as_deref(),
    )?;
    let action = DriveActionParam {
        kind: "fill".into(),
        id: None,
        url: None,
        x: p.x,
        y: p.y,
        text: Some(p.text),
        selector: p.selector,
        ref_id: p.ref_id,
        dx: None,
        dy: None,
        key: None,
        url_contains: None,
        timeout_ms: None,
        clear: p.clear,
    };
    validate_drive_action(&action).map_err(|e| ErrorData::invalid_params(e, None))?;
    let id = queue_action(&dir, &pubkey, action)?;
    let queue_only = p.queue_only.unwrap_or(false);
    let wait_timeout_ms = p.wait_timeout_ms.unwrap_or(15_000).min(60_000);
    let include_snapshot = p.include_snapshot.unwrap_or(false);
    drive_wait_response(
        &dir,
        &label,
        &grant,
        &[id],
        false,
        queue_only,
        wait_timeout_ms,
        50,
        include_snapshot,
        &pubkey,
    )
}

#[cfg(test)]
mod fill_field_tests {
    use super::*;
    use tempfile::tempdir;

    fn with_env<R>(dir: &Path, pubkey: &str, f: impl FnOnce() -> R) -> R {
        TEST_AGENT_DIR.with(|c| *c.borrow_mut() = Some(dir.to_path_buf()));
        TEST_PUBKEY.with(|c| *c.borrow_mut() = Some(pubkey.to_string()));
        let out = f();
        TEST_AGENT_DIR.with(|c| *c.borrow_mut() = None);
        TEST_PUBKEY.with(|c| *c.borrow_mut() = None);
        out
    }

    #[test]
    fn fill_field_queues_fill_action() {
        let tmp = tempdir().unwrap();
        let pubkey = "aa".repeat(32);
        let label = "playground-fill";
        with_env(tmp.path(), &pubkey, || {
            let gdir = tmp.path().join(label);
            fs::create_dir_all(&gdir).unwrap();
            fs::write(
                gdir.join("grant.json"),
                json!({
                    "agentPubkey": pubkey,
                    "mode": "drive",
                    "webviewLabel": label,
                    "surfaceId": "fill",
                    "surface": "playground",
                    "webviewHidden": true,
                    "parked": true
                })
                .to_string(),
            )
            .unwrap();
            let result = fill_field(FillFieldParams {
                webview_label: String::new(),
                surface_id: Some("fill".into()),
                text: "user@example.com".into(),
                selector: Some("#email".into()),
                ref_id: None,
                x: None,
                y: None,
                clear: Some(true),
                queue_only: Some(true),
                wait_timeout_ms: None,
                include_snapshot: None,
            })
            .expect("fill_field");
            let text = format!("{result:?}");
            assert!(text.contains("queued"), "{text}");
            let inbox = fs::read_to_string(gdir.join("drive-inbox.jsonl")).unwrap();
            assert!(inbox.contains("\"kind\":\"fill\""), "{inbox}");
        });
    }

    #[test]
    fn observe_poll_exposes_parked_and_drive_context() {
        let tmp = tempdir().unwrap();
        let pubkey = "bb".repeat(32);
        let label = "playground-park";
        with_env(tmp.path(), &pubkey, || {
            let gdir = tmp.path().join(label);
            fs::create_dir_all(&gdir).unwrap();
            fs::write(
                gdir.join("grant.json"),
                json!({
                    "agentPubkey": pubkey,
                    "mode": "observe",
                    "webviewLabel": label,
                    "surfaceId": "park",
                    "surface": "playground",
                    "webviewHidden": true,
                    "parked": true
                })
                .to_string(),
            )
            .unwrap();
            let result = observe_poll(ObservePollParams {
                webview_label: String::new(),
                surface_id: Some("park".into()),
                after_id: None,
                limit: None,
            })
            .expect("observe");
            let text = format!("{result:?}");
            assert!(text.contains("webviewHidden"), "{text}");
            assert!(text.contains("driveContext"), "{text}");
            assert!(text.contains("browser_fill_field"), "{text}");
        });
    }

    #[test]
    fn runbook_propose_rejects_persisted_title() {
        let tmp = tempdir().unwrap();
        let pubkey = "cc".repeat(32);
        let label = "playground-lock";
        with_env(tmp.path(), &pubkey, || {
            let gdir = tmp.path().join(label);
            fs::create_dir_all(&gdir).unwrap();
            fs::write(
                gdir.join("grant.json"),
                json!({
                    "agentPubkey": pubkey,
                    "mode": "drive",
                    "webviewLabel": label,
                    "surfaceId": "lock",
                    "surface": "playground"
                })
                .to_string(),
            )
            .unwrap();
            fs::write(
                gdir.join("runbook-full.json"),
                json!({
                    "agentBrief": "human brief",
                    "procedures": [{
                        "id": "p1",
                        "title": "SSO Login",
                        "steps": "locked",
                        "status": "active",
                        "persisted": true
                    }]
                })
                .to_string(),
            )
            .unwrap();
            let err = runbook_propose(RunbookProposeParams {
                webview_label: label.into(),
                surface_id: None,
                title: "SSO Login".into(),
                steps: "hack".into(),
            })
            .unwrap_err();
            assert!(format!("{err:?}").to_lowercase().contains("persisted"), "{err:?}");
        });
    }
}

#[cfg(test)]
mod runbook_tests {
    use super::*;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    fn with_env<R>(dir: &Path, pubkey: &str, f: impl FnOnce() -> R) -> R {
        let _g = LOCK.lock().unwrap();
        TEST_AGENT_DIR.with(|c| *c.borrow_mut() = Some(dir.to_path_buf()));
        TEST_PUBKEY.with(|c| *c.borrow_mut() = Some(pubkey.to_string()));
        let out = f();
        TEST_AGENT_DIR.with(|c| *c.borrow_mut() = None);
        TEST_PUBKEY.with(|c| *c.borrow_mut() = None);
        out
    }

    fn seed_grant(dir: &Path, label: &str, pubkey: &str, sid: &str) {
        let gdir = dir.join(label);
        fs::create_dir_all(&gdir).unwrap();
        let grant = json!({
            "webviewLabel": label,
            "surface": "playground",
            "surfaceId": sid,
            "agentId": pubkey,
            "agentPubkey": pubkey,
            "channelId": "ch1",
            "mode": "drive",
            "userHasControl": false,
            "createdAtMs": 1
        });
        fs::write(gdir.join("grant.json"), grant.to_string()).unwrap();
    }

    #[test]
    fn runbook_get_returns_inject_and_procedure() {
        let dir = tempfile::tempdir().unwrap();
        let pubkey = "aa".repeat(32);
        let label = "playground-sid1";
        seed_grant(dir.path(), label, &pubkey, "sid1");
        let gdir = dir.path().join(label);
        fs::write(
            gdir.join("runbook.json"),
            json!({
                "agentBrief": "Use SSO",
                "procedures": [{"id":"p1","title":"Login","summary":"Click Sign in"}]
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            gdir.join("runbook-full.json"),
            json!({
                "agentBrief": "Use SSO",
                "procedures": [{
                    "id":"p1",
                    "title":"Login",
                    "steps":"1. Click Sign in\n2. Wait",
                    "status":"active",
                    "createdAt":1,
                    "updatedAt":1
                }],
                "updatedAt": 1
            })
            .to_string(),
        )
        .unwrap();

        with_env(dir.path(), &pubkey, || {
            let index = runbook_get(RunbookGetParams {
                webview_label: label.into(),
                surface_id: None,
                procedure_id: None,
            })
            .unwrap();
            let text = format!("{index:?}");
            assert!(text.contains("Use SSO"), "{text}");
            assert!(text.contains("Login"), "{text}");

            let detail = runbook_get(RunbookGetParams {
                webview_label: String::new(),
                surface_id: Some("sid1".into()),
                procedure_id: Some("p1".into()),
            })
            .unwrap();
            let text = format!("{detail:?}");
            assert!(text.contains("Click Sign in"), "{text}");
        });
    }

    #[test]
    fn runbook_propose_appends_pending_line() {
        let dir = tempfile::tempdir().unwrap();
        let pubkey = "bb".repeat(32);
        let label = "playground-sid2";
        seed_grant(dir.path(), label, &pubkey, "sid2");
        with_env(dir.path(), &pubkey, || {
            runbook_propose(RunbookProposeParams {
                webview_label: label.into(),
                surface_id: None,
                title: "How to filter".into(),
                steps: "Open Filters".into(),
            })
            .unwrap();
            let raw = fs::read_to_string(dir.path().join(label).join("runbook-propose.jsonl"))
                .unwrap();
            assert!(raw.contains("How to filter"));
            assert!(raw.contains("Open Filters"));
            assert!(raw.contains(&pubkey));
        });
    }

    #[test]
    fn observe_poll_includes_runbook() {
        let dir = tempfile::tempdir().unwrap();
        let pubkey = "cc".repeat(32);
        let label = "playground-sid3";
        seed_grant(dir.path(), label, &pubkey, "sid3");
        fs::write(
            dir.path().join(label).join("runbook.json"),
            json!({"agentBrief":"brief","procedures":[]}).to_string(),
        )
        .unwrap();
        with_env(dir.path(), &pubkey, || {
            let result = observe_poll(ObservePollParams {
                webview_label: label.into(),
                surface_id: None,
                after_id: None,
                limit: None,
            })
            .unwrap();
            let text = format!("{result:?}");
            assert!(text.contains("runbook"), "{text}");
            assert!(text.contains("brief"), "{text}");
        });
    }
}
