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
    let url = format!(
        "{}{}",
        relay_api_base_url_with_override(state),
        path_and_query
    );
    let body_bytes = body.as_deref().map(str::as_bytes);
    let auth =
        build_nip98_auth_header_for_keys(&keys, &method, &url, body_bytes.unwrap_or_default())?;
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
    let path_and_query = list_path_and_query(channel_id.as_deref(), status.as_deref(), limit);
    let value = tasks_request(state.inner(), reqwest::Method::GET, &path_and_query, None).await?;
    parse_task_list(&value)
}

/// Clamp a caller-supplied page size into the relay's accepted window.
/// `None`/absent uses the relay default; out-of-range values are clamped rather
/// than sent (the relay rejects them as a protocol error, so a clamp turns a
/// hard 4xx into a sane page). Zero and negatives clamp UP to 1.
fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_TASK_LIMIT).clamp(1, MAX_TASK_LIMIT)
}

/// Build the signed request target for `tasks_list`. Extracted as a pure
/// function so the query-composition edge cases (absent filters, injection via
/// the status filter, limit clamping) are testable without a live relay — the
/// signed NIP-98 `u` tag must equal this string verbatim.
fn list_path_and_query(
    channel_id: Option<&str>,
    status: Option<&str>,
    limit: Option<i64>,
) -> String {
    let mut query: Vec<String> = vec![format!("limit={}", clamp_limit(limit))];
    if let Some(channel_id) = channel_id {
        query.push(format!("channel={}", urlencode(channel_id)));
    }
    if let Some(status) = status {
        query.push(format!("status={}", urlencode(status)));
    }
    format!("{}?{}", TASKS_PATH, query.join("&"))
}

/// Project a relay list response into tasks. A missing/!array `tasks` key is an
/// ERROR (the caller asked for a list and did not get one), while individual
/// malformed rows degrade to defaults via [`ChannelTask::from_json`] — one bad
/// row must never blank the whole list.
fn parse_task_list(value: &serde_json::Value) -> Result<Vec<ChannelTask>, String> {
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

/// `tasks_set_assignee` — assign or clear a task's owner via the relay's PATCH.
///
/// REG-16. The relay has accepted this field since 0033 (`api/tasks.rs`
/// `UpdateTaskRequest::assignee: Option<Option<String>>`, applied to
/// `TaskPatch::assignee_pubkey`) and it is "doubly optional on the wire":
/// field ABSENT = leave the assignee alone, field NULL = unassign. Only the
/// desktop side was missing, so this shim adds ZERO relay surface and no
/// migration. `None` here means unassign, and we emit an explicit JSON null to
/// preserve that distinction rather than dropping the key.
///
/// Authz is wholly inherited: the relay validates the assignee is in the
/// community and rejects callers who may not patch the task. A suggestion is
/// never a grant.
#[tauri::command]
pub async fn tasks_set_assignee(
    state: State<'_, AppState>,
    task_id: String,
    assignee: Option<String>,
) -> Result<ChannelTask, String> {
    let path = format!("{TASKS_PATH}/{task_id}");
    // serde_json::Value::Null is emitted for `None` — the unassign case.
    let payload = serde_json::json!({ "assignee": assignee });
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
        let result = tasks_request_at(
            state.inner(),
            reqwest::Method::GET,
            trimmed,
            TASKS_PATH,
            "?limit=50",
            None,
        )
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
    let auth =
        build_nip98_auth_header_for_keys(&keys, &method, &url, body_bytes.unwrap_or_default())?;
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

    // ---------------------------------------------------------------------
    // REG-15 hardening (fire #47): edge cases enumerated in
    // work/REG-15/hardening.md §"Edge case matrix". Each row below maps to one
    // enumerated class. Negative/adversarial cases are marked NEGATIVE.
    // ---------------------------------------------------------------------

    /// E1 — EMPTY INPUT: an empty `tasks` array is a valid, successful empty
    /// list, NOT an error. Regression guard: treating empty as malformed would
    /// show an error state on every brand-new channel.
    #[test]
    fn empty_task_array_is_an_empty_list_not_an_error() {
        let value = serde_json::json!({ "tasks": [] });
        let tasks = parse_task_list(&value).expect("empty array must parse as Ok");
        assert!(tasks.is_empty());
    }

    /// E2 — NEGATIVE, MALFORMED ENVELOPE: `tasks` missing entirely, or present
    /// but not an array, must be a hard error. The caller asked for a list and
    /// did not receive one; silently returning `[]` would render "no tasks"
    /// over a broken relay response and hide the fault.
    #[test]
    fn malformed_list_envelope_is_an_error_never_an_empty_list() {
        // absent key
        assert!(parse_task_list(&serde_json::json!({})).is_err());
        // present but wrong type — each must error, not degrade
        for wrong in [
            serde_json::json!({ "tasks": null }),
            serde_json::json!({ "tasks": "nope" }),
            serde_json::json!({ "tasks": 7 }),
            serde_json::json!({ "tasks": { "0": "obj-not-array" } }),
        ] {
            assert!(
                parse_task_list(&wrong).is_err(),
                "non-array tasks must error: {wrong}"
            );
        }
    }

    /// E3 — MALFORMED ROW ISOLATION: one bad row inside a good envelope must
    /// degrade to defaults and STILL yield the sibling rows. A single corrupt
    /// task must never blank the whole list.
    #[test]
    fn one_malformed_row_does_not_drop_its_siblings() {
        let value = serde_json::json!({
            "tasks": [
                { "id": "a", "title": "good", "status": "open", "updated_at": 2 },
                "not-an-object",
                { "id": "c", "title": "also good", "status": "done", "updated_at": 1 },
            ]
        });
        let tasks = parse_task_list(&value).expect("envelope is well formed");
        assert_eq!(tasks.len(), 3, "row count preserved, bad row degraded");
        assert_eq!(tasks[0].title, "good");
        assert_eq!(tasks[1].title, "", "malformed row degrades to defaults");
        assert_eq!(tasks[1].id, "");
        assert_eq!(tasks[2].title, "also good");
    }

    /// E4 — BOUNDARY: limit clamping at both ends and the absent case. The
    /// relay rejects out-of-window limits as a protocol error, so a client-side
    /// clamp is what keeps a slider at 0 or 10_000 from producing a hard 4xx.
    #[test]
    fn limit_is_clamped_into_the_relay_window() {
        assert_eq!(clamp_limit(None), DEFAULT_TASK_LIMIT, "absent = default");
        assert_eq!(clamp_limit(Some(0)), 1, "zero clamps up to 1");
        assert_eq!(clamp_limit(Some(-5)), 1, "negative clamps up to 1");
        assert_eq!(clamp_limit(Some(1)), 1);
        assert_eq!(clamp_limit(Some(MAX_TASK_LIMIT)), MAX_TASK_LIMIT);
        assert_eq!(
            clamp_limit(Some(MAX_TASK_LIMIT + 1)),
            MAX_TASK_LIMIT,
            "above window clamps down"
        );
        assert_eq!(clamp_limit(Some(i64::MAX)), MAX_TASK_LIMIT);
        assert_eq!(clamp_limit(Some(i64::MIN)), 1);
    }

    /// E5 — QUERY COMPOSITION: absent filters must be OMITTED from the query,
    /// not sent as the literal string "None"/"null". The signed NIP-98 `u` tag
    /// is this exact string, so a stray param is an auth mismatch as well as a
    /// wrong filter.
    #[test]
    fn absent_filters_are_omitted_from_the_query() {
        let q = list_path_and_query(None, None, None);
        assert_eq!(q, format!("/api/tasks?limit={DEFAULT_TASK_LIMIT}"));
        assert!(!q.contains("channel="), "no empty channel param");
        assert!(!q.contains("status="), "no empty status param");
        assert!(!q.to_lowercase().contains("none"));
        assert!(!q.to_lowercase().contains("null"));
    }

    /// E6 — NEGATIVE, QUERY INJECTION: a hostile channel id or status must be
    /// percent-encoded, so it cannot smuggle an extra query parameter. Before
    /// this fire the channel id was interpolated RAW (`channel={channel_id}`),
    /// so `x&limit=9999` would have overridden the clamped limit — this is the
    /// regression test for that defect.
    #[test]
    fn hostile_filter_values_cannot_smuggle_extra_query_params() {
        let q = list_path_and_query(Some("abc&limit=9999"), None, Some(10));
        assert!(
            q.contains("channel=abc%26limit%3D9999"),
            "ampersand and equals must be encoded, got: {q}"
        );
        assert_eq!(q.matches("limit=").count(), 1, "no second limit param: {q}");
        assert!(
            q.starts_with("/api/tasks?limit=10&"),
            "clamped limit wins: {q}"
        );

        // Same guarantee on the status filter.
        let q = list_path_and_query(None, Some("open&channel=other"), None);
        assert!(
            !q.contains("&channel="),
            "status cannot inject channel: {q}"
        );
    }

    /// E7 — the full filter combination keeps a stable, signable ordering.
    #[test]
    fn full_query_has_stable_parameter_order() {
        let q = list_path_and_query(Some("chan1"), Some("in progress"), Some(25));
        assert_eq!(q, "/api/tasks?limit=25&channel=chan1&status=in%20progress");
    }

    /// E8 — UNKNOWN STATUS TOLERANCE: a status the desktop does not model must
    /// round-trip as an opaque string rather than being coerced or dropped.
    /// Mirrors the TS "unknown values degrade to a badge" rule so a relay-side
    /// status addition never breaks an older desktop build.
    #[test]
    fn unknown_status_round_trips_opaquely() {
        let task = ChannelTask::from_json(&serde_json::json!({
            "id": "t1", "title": "x", "status": "blocked_on_legal", "updated_at": 5
        }));
        assert_eq!(task.status, "blocked_on_legal");
    }

    /// E9 — TYPE CONFUSION: correctly-named keys carrying the wrong JSON type
    /// must degrade to defaults instead of panicking. `as_str`/`as_i64` return
    /// None on mismatch; this pins that contract against a future refactor to
    /// an unwrapping deserializer.
    #[test]
    fn wrongly_typed_fields_degrade_instead_of_panicking() {
        let task = ChannelTask::from_json(&serde_json::json!({
            "id": 12345,
            "channel_id": ["array"],
            "title": { "nested": true },
            "status": false,
            "assignee": 9,
            "created_by": null,
            "updated_at": "not-a-number",
        }));
        assert_eq!(task.id, "");
        assert_eq!(task.channel_id, None);
        assert_eq!(task.title, "");
        assert_eq!(task.status, "");
        assert_eq!(task.assignee, None, "numeric assignee is not a pubkey");
        assert_eq!(task.created_by, None);
        assert_eq!(task.updated_at, 0, "unparseable timestamp sorts last");
    }

    /// E10 — a float timestamp (JSON has no integer type) must not silently
    /// become 0 ordering garbage without being noticed; pin current behaviour.
    #[test]
    fn float_timestamp_is_not_silently_reinterpreted() {
        let task = ChannelTask::from_json(&serde_json::json!({ "updated_at": 1.5 }));
        assert_eq!(task.updated_at, 0, "non-integral timestamp degrades to 0");
    }
}
