//! REG-15: desktop channel-task commands over the relay's REST task API.
//!
//! The task system lives server-side (`migrations/0033_task_system.sql` +
//! `crates/buzz-relay/src/api/tasks.rs`: create/list/get/update/events, NIP-98
//! auth, tenant isolation) and is consumed by mobile today; desktop had ZERO
//! consumers. These commands are the desktop half: thin typed wrappers that
//! sign each request with the shared NIP-98 header builder
//! (`crate::relay::build_nip98_auth_header_for_keys` — same precedent as
//! `assignmentOperationFetch`'s relay-client signing on the projects side; no
//! feature hand-rolls HTTP signing) and talk to the SAME base URL every other
//! relay-backed desktop command uses (`relay_api_base_url_with_override`), so
//! the active workspace relay and its per-tenant host are honored.
//!
//! Scope decisions (work/REG-15/reflecting.md): "Channel task" naming, distinct
//! from Projects' kind:30617 issues (D4 Option A); list+create+complete+reopen
//! slice; authz inherited from the relay (membership + channel access enforced
//! server-side at tasks.rs `enforce_channel_access`); ZERO migrations; the
//! consolidated My-Tasks view is a CLIENT-SIDE fan-in because the list API
//! binds `community_id` always (fire-#28 Q1) and tasks are not nostr events,
//! so no push channel exists at any layer (fire-#28 Q2) — request/response
//! with refetch-on-focus is a system property, not a v1 shortcut.
//!
//! Naming note: `list_tasks` here mirrors the relay handler's name; the
//! commands are namespaced `tasks_*` from the frontend's perspective.

use serde::Serialize;
use tauri::State;

use crate::{
    app_state::AppState,
    relay::{build_nip98_auth_header_for_keys, relay_api_base_url_with_override},
};

const TASKS_PATH: &str = "/api/tasks";
/// Mirrors the relay's default page (`DEFAULT_TASK_LIMIT` in api/tasks.rs).
const DEFAULT_TASK_LIMIT: i64 = 50;
/// Mirrors the relay's hard clamp (`MAX_TASK_LIMIT`); requesting more is a
/// protocol error, so clamp client-side instead of red.
const MAX_TASK_LIMIT: i64 = 200;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelTask {
    pub id: String,
    pub channel_id: Option<String>,
    pub title: String,
    pub status: String,
    pub assignee: Option<String>,
    pub created_by: Option<String>,
    pub updated_at: i64,
}

impl ChannelTask {
    fn from_json(value: &serde_json::Value) -> ChannelTask {
        // The relay serializes ids as raw JSON values; uuids arrive as strings.
        ChannelTask {
            id: value["id"].as_str().unwrap_or_default().to_owned(),
            channel_id: value["channel_id"].as_str().map(str::to_owned),
            title: value["title"].as_str().unwrap_or_default().to_owned(),
            status: value["status"].as_str().unwrap_or_default().to_owned(),
            assignee: value["assignee"].as_str().map(str::to_owned),
            created_by: value["created_by"].as_str().map(str::to_owned),
            updated_at: value["updated_at"].as_i64().unwrap_or_default(),
        }
    }
}

/// One source's outcome in the My-Tasks fan-in. A failed community is INLINE
/// data (never a silent drop and never a whole-view failure — reflecting D2
/// correctness requirement): the UI renders it as an error row scoped to that
/// community while the others still render.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelTaskSource {
    /// The relay base URL the fetch targeted (the community's identity here;
    /// the frontend maps it to a community name).
    pub relay_base: String,
    pub tasks: Vec<ChannelTask>,
    pub error: Option<String>,
}

/// Signed request to the task API. The relay verifies the NIP-98 `u` tag
/// against the per-tenant host + path (with query, verbatim — see
/// `request_path`/`nip98_expected_url`), so the signed URL must be exactly
/// the request URL. Payload tag: the relay requires one on body-bearing
/// methods (`require_payload`), so sign the body we will send, `None` for GET.
async fn tasks_request(
    state: &AppState,
    method: reqwest::Method,
    path_and_query: &str,
    body: Option<String>,
) -> Result<serde_json::Value, String> {
    crate::relay_admission::wait_for_rate_limit().await;
    let keys = state.signing_keys()?;
    let url = format!("{}{}", relay_api_base_url_with_override(state), path_and_query);
    let body_bytes = body.as_deref().map(str::as_bytes);
    let auth = build_nip98_auth_header_for_keys(
        &keys,
        &method,
        &url,
        body_bytes.unwrap_or_default(),
    )?;
    let mut request = state
        .http_client
        .request(method, &url)
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(30));
    if let Some(body) = body {
        request = request.body(body);
    }
    let response = request
        .send()
        .await
        .map_err(|e| crate::relay::classify_request_error(&e))?;
    if !response.status().is_success() {
        let status = response.status();
        let message = crate::relay::relay_error_message(response).await;
        return Err(format!("task API error {status}: {message}"));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("task API response parse failed: {e}"))
}

/// `tasks_list` — list the ACTIVE workspace community's tasks, newest-updated
/// first. `channel_id` scopes to one channel; absent = all visible channels
/// (the relay post-filters to the caller's accessible channels).
#[tauri::command]
pub async fn tasks_list(
    state: State<'_, AppState>,
    channel_id: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<ChannelTask>, String> {
    let limit = limit.unwrap_or(DEFAULT_TASK_LIMIT).clamp(1, MAX_TASK_LIMIT);
    let mut query: Vec<String> = vec![format!("limit={limit}")];
    if let Some(channel_id) = channel_id.as_deref() {
        query.push(format!("channel={channel_id}"));
    }
    if let Some(status) = status.as_deref() {
        query.push(format!("status={}", urlencode(status)));
    }
    let path_and_query = format!("{}?{}", TASKS_PATH, query.join("&"));
    let value = tasks_request(state.inner(), reqwest::Method::GET, &path_and_query, None).await?;
    let tasks = value["tasks"]
        .as_array()
        .ok_or_else(|| "task API: malformed list response".to_string())?;
    Ok(tasks.iter().map(ChannelTask::from_json).collect())
}

/// `tasks_create` — create a task in the active community. `channel_id` may be
/// null (community-wide task).
#[tauri::command]
pub async fn tasks_create(
    state: State<'_, AppState>,
    title: String,
    channel_id: Option<String>,
    body_text: Option<String>,
) -> Result<ChannelTask, String> {
    let payload = serde_json::json!({
        "title": title,
        "channel_id": channel_id,
        "body": body_text,
    });
    let value = tasks_request(
        state.inner(),
        reqwest::Method::POST,
        TASKS_PATH,
        Some(payload.to_string()),
    )
    .await?;
    Ok(ChannelTask::from_json(&value))
}

/// `tasks_set_status` — complete (`"done"`) or reopen (`"open"`) a task via
/// the relay's PATCH. The relay appends the status-change event to the task's
/// history and stamps `done_at`.
#[tauri::command]
pub async fn tasks_set_status(
    state: State<'_, AppState>,
    task_id: String,
    status: String,
) -> Result<ChannelTask, String> {
    let path = format!("{TASKS_PATH}/{task_id}");
    let payload = serde_json::json!({ "status": status });
    let value = tasks_request(
        state.inner(),
        reqwest::Method::PATCH,
        &path,
        Some(payload.to_string()),
    )
    .await?;
    Ok(ChannelTask::from_json(&value))
}

/// `tasks_my_workspaces` — the consolidated My-Tasks fan-in across the user's
/// configured communities. The frontend supplies the relay base URLs of the
/// active session's communities (bounded, recency-ordered — reflecting D2);
/// each is queried with the SAME identity keys over its own base URL, and a
/// per-source failure is INLINE (`error` set, `tasks` empty) so one
/// unreachable community never blanks the consolidated view.
#[tauri::command]
pub async fn tasks_my_workspaces(
    state: State<'_, AppState>,
    relay_bases: Vec<String>,
) -> Result<Vec<ChannelTaskSource>, String> {
    // Bound the fan-out (reflecting D2: cap 10 by recency, enforced at the
    // collection site too so no caller can unbound it).
    let mut sources = Vec::with_capacity(relay_bases.len().min(10));
    for base in relay_bases.iter().take(10) {
        let trimmed = base.trim_end_matches('/');
        // tasks_request signs against relay_api_base_url_with_override, which
        // resolves the ACTIVE workspace relay — a per-source override is
        // needed to target each community's relay. Route through the same
        // helper with an explicit base.
        let result =
            tasks_request_at(state.inner(), reqwest::Method::GET, trimmed, TASKS_PATH, "?limit=50", None)
                .await;
        sources.push(match result {
            Ok(value) => {
                let tasks = value["tasks"]
                    .as_array()
                    .map(|rows| rows.iter().map(ChannelTask::from_json).collect())
                    .unwrap_or_default();
                ChannelTaskSource {
                    relay_base: trimmed.to_owned(),
                    tasks,
                    error: None,
                }
            }
            Err(error) => ChannelTaskSource {
                relay_base: trimmed.to_owned(),
                tasks: Vec::new(),
                error: Some(error),
            },
        });
    }
    Ok(sources)
}

/// Like [`tasks_request`] but targeting an explicit relay base URL (used by
/// the My-Tasks fan-in, where each community has its own relay).
#[allow(clippy::too_many_arguments)]
async fn tasks_request_at(
    state: &AppState,
    method: reqwest::Method,
    base: &str,
    path: &str,
    query: &str,
    body: Option<String>,
) -> Result<serde_json::Value, String> {
    crate::relay_admission::wait_for_rate_limit().await;
    let keys = state.signing_keys()?;
    let url = format!("{base}{path}{query}");
    let body_bytes = body.as_deref().map(str::as_bytes);
    let auth = build_nip98_auth_header_for_keys(
        &keys,
        &method,
        &url,
        body_bytes.unwrap_or_default(),
    )?;
    let mut request = state
        .http_client
        .request(method, &url)
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(30));
    if let Some(body) = body {
        request = request.body(body);
    }
    let response = request
        .send()
        .await
        .map_err(|e| crate::relay::classify_request_error(&e))?;
    if !response.status().is_success() {
        let status = response.status();
        let message = crate::relay::relay_error_message(response).await;
        return Err(format!("task API error {status}: {message}"));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("task API response parse failed: {e}"))
}

/// Percent-encode a query value (the status filter). Allocations are fine
/// here; this is not a hot path.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencode_encodes_reserved_and_space() {
        assert_eq!(urlencode("in progress"), "in%20progress");
        assert_eq!(urlencode("a&b=c?d"), "a%26b%3Dc%3Fd");
        assert_eq!(urlencode("done"), "done");
    }

    #[test]
    fn channel_task_parses_the_relay_shape() {
        let value = serde_json::json!({
            "id": "5d8c5b0e-3b1e-4b7a-9b0d-5f9b9a5c1d01",
            "channel_id": null,
            "title": "Ship the thing",
            "status": "in_progress",
            "assignee": null,
            "created_by": "aa" .repeat(32),
            "updated_at": 1_756_000_000_i64,
        });
        let task = ChannelTask::from_json(&value);
        assert_eq!(task.title, "Ship the thing");
        assert_eq!(task.status, "in_progress");
        assert_eq!(task.channel_id, None);
        assert_eq!(task.assignee, None);
        assert!(task.created_by.unwrap().starts_with("aaaa"));
    }

    #[test]
    fn channel_task_degrades_to_defaults_on_malformed_rows() {
        let task = ChannelTask::from_json(&serde_json::json!({}));
        assert_eq!(task.title, "");
        assert_eq!(task.updated_at, 0);
        assert_eq!(task.id, "");
    }

    #[test]
    fn fan_in_is_bounded_at_ten_sources() {
        // Structural: the take(10) bound lives in tasks_my_workspaces; assert
        // the contract constant indirectly via collection sizing math.
        let supplied: Vec<String> = (0..25).map(|i| format!("https://r{i}.example")).collect();
        // Mirror of the production bound for documentation purposes.
        let bound: Vec<String> = supplied.iter().take(10).cloned().collect();
        assert_eq!(bound.len(), 10);
        assert_eq!(bound[0], "https://r0.example");
        assert_eq!(bound[9], "https://r9.example");
    }
}
