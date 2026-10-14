use buzz_core_pkg::WORKFLOW_TIMEZONE_EXTENSION;
use std::collections::HashSet;

use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::{
    app_state::AppState,
    events,
    relay::{
        assert_expected_relay_scope, assert_expected_signer, get_relay_json,
        parse_command_response, query_relay, query_relay_at_with_keys,
        relay_api_base_url_with_override, submit_event, submit_event_at_with_keys,
    },
};

// ── Wire shapes (snake_case, consumed by tauriWorkflows.ts) ──────────────────

/// A workflow definition as the desktop frontend expects it. Mirrors the
/// `RawWorkflow` type in `desktop/src/shared/api/tauriWorkflows.ts`.
///
/// The relay stores a workflow as a single kind:30620 event whose content is
/// the raw YAML. Everything the UI needs is derived from that event:
/// - `id` / `channel_id` from the `d` / `h` tags,
/// - `definition` from parsing the YAML body into a free-form object,
/// - `name` from `definition.name`,
/// - `owner_pubkey` / timestamps from the event itself.
///
/// `status` is always `"active"` here: the relay's disable/archive lifecycle is
/// not reflected back into the kind:30620 event, and the UI derives a
/// "disabled" display state from `definition.enabled` on its own
/// (`getWorkflowDisplayStatus`).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkflowWire {
    pub id: String,
    /// Event id of the current kind:30620 revision, used for conflict-protected updates.
    pub revision: String,
    pub name: String,
    pub owner_pubkey: String,
    pub channel_id: Option<String>,
    pub definition: Value,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Response shape for create/update. Mirrors `RawWorkflowSaveResponse` in the
/// frontend: a full workflow record plus an optional webhook secret (only
/// present for webhook-triggered workflows on creation).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkflowSaveWire {
    #[serde(flatten)]
    pub workflow: WorkflowWire,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, Serialize, PartialEq)]
pub struct WorkflowRunCursorWire {
    pub before: String,
    pub before_id: String,
}

#[derive(Debug, Clone, serde::Deserialize, Serialize, PartialEq)]
pub struct WorkflowRunsWire {
    pub runs: Vec<Value>,
    pub next: Option<WorkflowRunCursorWire>,
}

#[derive(Debug, Clone, serde::Deserialize, Serialize, PartialEq)]
pub struct WorkflowApprovalsWire {
    pub approvals: Vec<Value>,
}

/// Canonical trigger acknowledgement consumed by the Desktop client.
///
/// The relay currently returns only `run_id`; the workflow id is the command
/// input and a newly-created run always begins pending. Keeping that adaptation
/// here prevents the frontend from guessing fields or confusing the trigger
/// event id with the persisted run id.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkflowTriggerWire {
    pub run_id: String,
    pub workflow_id: String,
    pub status: String,
}

#[derive(Debug, serde::Deserialize)]
struct WorkflowTriggerAck {
    run_id: String,
}

// ── Reads ────────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_channel_workflows(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<WorkflowWire>, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30620],
            "#h": [channel_id],
        })],
    )
    .await?;

    Ok(events.iter().map(workflow_from_event).collect())
}

// Keep this aligned with the relay's aggregate explicit-`#h` request bound.
// Each filter below carries exactly one explicit value so old relays retain the
// known-compatible shape while current relays cannot reject large memberships.
const WORKFLOW_QUERY_CHANNEL_BATCH_SIZE: usize = 128;

/// Fetch workflows across many channels using bounded relay round-trips.
///
/// The Workflows overview screen previously issued one `get_channel_workflows`
/// query per member channel (`Promise.all` fanout in `WorkflowsView`), i.e. N
/// relay POSTs. This sends one single-channel filter per channel, in requests of
/// at most 128 filters. Using one multi-value `#h` filter is equivalent under
/// NIP-01, but older relays incorrectly narrowed that shape to its first
/// channel. Each `WorkflowWire` carries its own `channel_id` (from the event's
/// `h` tag), so the frontend can still group results by channel. Neither this
/// nor the per-channel command sets a `limit`, so batching does not change
/// result completeness. Results are deduplicated by signed event ID in case a
/// caller supplies duplicate channel IDs.
#[tauri::command]
pub async fn get_channels_workflows(
    channel_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<WorkflowWire>, String> {
    let filter_batches = channel_workflow_filter_batches(channel_ids)?;
    let mut seen_event_ids = HashSet::new();
    let mut workflows = Vec::new();

    for filters in filter_batches {
        let events = query_relay(&state, &filters).await?;
        append_unique_workflows(&mut workflows, &mut seen_event_ids, &events);
    }

    Ok(workflows)
}

fn append_unique_workflows(
    workflows: &mut Vec<WorkflowWire>,
    seen_event_ids: &mut HashSet<nostr::EventId>,
    events: &[nostr::Event],
) {
    workflows.extend(
        events
            .iter()
            .filter(|event| seen_event_ids.insert(event.id))
            .map(workflow_from_event),
    );
}

fn channel_workflow_filter_batches(channel_ids: Vec<String>) -> Result<Vec<Vec<Value>>, String> {
    let filters = channel_workflow_filters(channel_ids)?;
    Ok(filters
        .chunks(WORKFLOW_QUERY_CHANNEL_BATCH_SIZE)
        .map(<[Value]>::to_vec)
        .collect())
}

fn channel_workflow_filters(channel_ids: Vec<String>) -> Result<Vec<Value>, String> {
    channel_ids
        .into_iter()
        .map(|channel_id| {
            let channel_id = uuid::Uuid::parse_str(channel_id.trim())
                .map_err(|_| "invalid channel id".to_string())?;
            Ok(serde_json::json!({
                "kinds": [30620],
                "#h": [channel_id.to_string()],
            }))
        })
        .collect()
}

#[tauri::command]
pub async fn get_workflow(
    workflow_id: String,
    state: State<'_, AppState>,
) -> Result<WorkflowWire, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30620],
            "#d": [workflow_id],
            "limit": 1
        })],
    )
    .await?;

    events
        .first()
        .map(workflow_from_event)
        .ok_or_else(|| "workflow not found".to_string())
}

#[tauri::command]
pub async fn get_workflow_runs(
    workflow_id: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<WorkflowRunsWire, String> {
    let workflow_id =
        uuid::Uuid::parse_str(&workflow_id).map_err(|_| "invalid workflow id".to_string())?;
    let limit = limit.unwrap_or(20).clamp(1, 100);
    get_relay_json(
        &state,
        &format!("/workflows/{workflow_id}/runs?limit={limit}"),
    )
    .await
}

// ── Writes ───────────────────────────────────────────────────────────────────

const WORKFLOW_CAPABILITY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const WORKFLOW_CAPABILITY_BODY_CAP: u64 = 65_536;
const WORKFLOW_TIMEZONE_UNSUPPORTED_ERROR: &str = "This server does not support workflow timezones. Update the server, or use a legacy UTC schedule without a timezone.";
const WORKFLOW_TIMEZONE_CHECK_ERROR: &str = "Could not verify whether this server supports workflow timezones. Check the server connection and try again.";

#[derive(serde::Deserialize)]
struct RelayWorkflowCapabilities {
    #[serde(default)]
    supported_extensions: Option<Vec<String>>,
}

/// Whether a workflow definition would rely on relay-side timezone handling.
///
/// Malformed definitions remain the relay's validation responsibility. Any
/// explicit `timezone` field on a schedule trigger needs the mixed-version
/// safety gate: an older relay may otherwise ignore an unfamiliar value.
fn workflow_uses_schedule_timezone(yaml_definition: &str) -> bool {
    let Ok(Value::Object(definition)) = serde_yaml::from_str::<Value>(yaml_definition) else {
        return false;
    };
    let Some(Value::Object(trigger)) = definition.get("trigger") else {
        return false;
    };
    trigger.get("on").and_then(Value::as_str) == Some("schedule")
        && trigger.contains_key("timezone")
}

async fn read_workflow_capability_body(response: reqwest::Response) -> Result<Vec<u8>, String> {
    use futures_util::StreamExt;

    if let Some(length) = response.content_length() {
        if length > WORKFLOW_CAPABILITY_BODY_CAP {
            return Err("relay capability document is too large".to_string());
        }
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| crate::relay::classify_request_error(&error))?;
        if body.len() as u64 + chunk.len() as u64 > WORKFLOW_CAPABILITY_BODY_CAP {
            return Err("relay capability document is too large".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn require_workflow_timezone_capability_at(
    yaml_definition: &str,
    relay_api_base: &str,
    client: &reqwest::Client,
) -> Result<(), String> {
    if !workflow_uses_schedule_timezone(yaml_definition) {
        return Ok(());
    }

    let expected_origin = url::Url::parse(relay_api_base)
        .map_err(|_| WORKFLOW_TIMEZONE_CHECK_ERROR.to_string())?
        .origin()
        .ascii_serialization();
    let response = client
        .get(format!("{}/info", relay_api_base.trim_end_matches('/')))
        .header(reqwest::header::ACCEPT, "application/nostr+json")
        .timeout(WORKFLOW_CAPABILITY_TIMEOUT)
        .send()
        .await
        .map_err(|_| WORKFLOW_TIMEZONE_CHECK_ERROR.to_string())?;

    let status = response.status();
    if response.url().origin().ascii_serialization() != expected_origin || !status.is_success() {
        return Err(WORKFLOW_TIMEZONE_CHECK_ERROR.to_string());
    }

    let body = read_workflow_capability_body(response)
        .await
        .map_err(|_| WORKFLOW_TIMEZONE_CHECK_ERROR.to_string())?;
    let capabilities: RelayWorkflowCapabilities =
        serde_json::from_slice(&body).map_err(|_| WORKFLOW_TIMEZONE_CHECK_ERROR.to_string())?;

    let supported = capabilities
        .supported_extensions
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|extension| extension == WORKFLOW_TIMEZONE_EXTENSION);
    if !supported {
        return Err(WORKFLOW_TIMEZONE_UNSUPPORTED_ERROR.to_string());
    }
    Ok(())
}

async fn build_workflow_definition_for_save(
    workflow_id: &str,
    channel_id: &str,
    yaml_definition: &str,
    expected_revision: Option<&str>,
    relay_api_base: &str,
    client: &reqwest::Client,
) -> Result<nostr::EventBuilder, String> {
    let builder = events::build_workflow_definition(
        workflow_id,
        channel_id,
        yaml_definition,
        expected_revision,
    )?;
    require_workflow_timezone_capability_at(yaml_definition, relay_api_base, client).await?;
    Ok(builder)
}

fn assert_workflow_save_scope_current(
    state: &AppState,
    relay_api_base: &str,
    signing_keys: &nostr::Keys,
) -> Result<(), String> {
    let active_relay = relay_api_base_url_with_override(state);
    assert_expected_relay_scope(Some(relay_api_base), &active_relay)?;
    let active_keys = state.signing_keys()?;
    assert_expected_signer(
        Some(&signing_keys.public_key().to_hex()),
        &active_keys.public_key().to_hex(),
    )
}

#[tauri::command]
pub async fn create_workflow(
    channel_id: String,
    yaml_definition: String,
    state: State<'_, AppState>,
) -> Result<WorkflowSaveWire, String> {
    let workflow_id = uuid::Uuid::new_v4().to_string();
    let relay_api_base = relay_api_base_url_with_override(&state);
    let signing_keys = state.signing_keys()?;
    let builder = build_workflow_definition_for_save(
        &workflow_id,
        &channel_id,
        &yaml_definition,
        None,
        &relay_api_base,
        &state.media_fetch_client,
    )
    .await?;
    assert_workflow_save_scope_current(&state, &relay_api_base, &signing_keys)?;
    let result = submit_event_at_with_keys(builder, &state, &relay_api_base, &signing_keys).await?;

    // The relay returns `webhook_secret` in the OK response message for
    // webhook-triggered workflows. Everything else in the save record is built
    // locally from the inputs we already hold — the relay's create response
    // only carries `{ workflow_id, webhook_secret? }`.
    let webhook_secret = parse_command_response::<Value>(&result.message)
        .ok()
        .and_then(|v| {
            v.get("webhook_secret")
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    let now = now_secs();
    let workflow = workflow_record(
        workflow_id,
        result.event_id,
        Some(channel_id),
        signing_keys.public_key().to_hex(),
        &yaml_definition,
        now,
        now,
    );

    Ok(WorkflowSaveWire {
        workflow,
        webhook_secret,
    })
}

#[tauri::command]
pub async fn update_workflow(
    workflow_id: String,
    yaml_definition: String,
    expected_revision: String,
    state: State<'_, AppState>,
) -> Result<WorkflowSaveWire, String> {
    let relay_api_base = relay_api_base_url_with_override(&state);
    let signing_keys = state.signing_keys()?;
    // Find the channel id (and creation time) from the existing workflow event
    // so the new event carries the same `h` tag — kind:30620 is replaceable by
    // (pubkey, d-tag).
    let prior = query_relay_at_with_keys(
        &state,
        &relay_api_base,
        &[serde_json::json!({
            "kinds": [30620],
            "#d": [workflow_id.clone()],
            "limit": 1
        })],
        &signing_keys,
        None,
    )
    .await?;

    let prior_event = prior
        .first()
        .ok_or_else(|| "workflow not found".to_string())?;
    if prior_event.id.to_hex() != expected_revision {
        return Err("workflow changed since it was loaded; refresh and try again".to_string());
    }
    let channel_id = tag_value(prior_event, "h").ok_or_else(|| "workflow not found".to_string())?;
    let created_at = prior_event.created_at.as_secs() as i64;

    let builder = build_workflow_definition_for_save(
        &workflow_id,
        &channel_id,
        &yaml_definition,
        Some(&expected_revision),
        &relay_api_base,
        &state.media_fetch_client,
    )
    .await?;
    assert_workflow_save_scope_current(&state, &relay_api_base, &signing_keys)?;
    let result = submit_event_at_with_keys(builder, &state, &relay_api_base, &signing_keys).await?;

    let updated_at = now_secs();
    let workflow = workflow_record(
        workflow_id,
        result.event_id,
        Some(channel_id),
        signing_keys.public_key().to_hex(),
        &yaml_definition,
        created_at,
        updated_at,
    );

    Ok(WorkflowSaveWire {
        workflow,
        // Updates never rotate the webhook secret.
        webhook_secret: None,
    })
}

#[tauri::command]
pub async fn delete_workflow(
    workflow_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // The NIP-09 `a` coordinate must name the workflow's actual author, not
    // the caller: kind:30620 is addressable by (author, d-tag), and workflows
    // are commonly agent-created. Addressing the delete at the caller's own
    // pubkey targets a record that doesn't exist — the relay accepts the
    // kind:5 and deletes nothing.
    let prior = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30620],
            "#d": [workflow_id.clone()],
            "limit": 1
        })],
    )
    .await?;
    let owner_pubkey = prior
        .first()
        .map(|ev| ev.pubkey.to_hex())
        .ok_or_else(|| "workflow not found".to_string())?;

    let builder = events::build_workflow_delete(&workflow_id, &owner_pubkey)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn trigger_workflow(
    workflow_id: String,
    state: State<'_, AppState>,
) -> Result<WorkflowTriggerWire, String> {
    let builder = events::build_workflow_trigger(&workflow_id)?;
    let result = submit_event(builder, &state).await?;
    trigger_wire_from_message(workflow_id, &result.message)
}

// ── Approvals ────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_run_approvals(
    workflow_id: String,
    run_id: String,
    state: State<'_, AppState>,
) -> Result<WorkflowApprovalsWire, String> {
    let workflow_id =
        uuid::Uuid::parse_str(&workflow_id).map_err(|_| "invalid workflow id".to_string())?;
    let run_id =
        uuid::Uuid::parse_str(&run_id).map_err(|_| "invalid workflow run id".to_string())?;
    get_relay_json(
        &state,
        &format!("/workflows/{workflow_id}/runs/{run_id}/approvals"),
    )
    .await
}

#[tauri::command]
pub async fn grant_approval(
    token: String,
    note: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let builder = events::build_approval_grant(&token, note.as_deref())?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({ "event_id": result.event_id }))
}

#[tauri::command]
pub async fn deny_approval(
    token: String,
    note: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let builder = events::build_approval_deny(&token, note.as_deref())?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({ "event_id": result.event_id }))
}

// ── Helpers (pure, unit-tested in workflows_tests.rs) ─────────────────────────

fn trigger_wire_from_message(
    workflow_id: String,
    message: &str,
) -> Result<WorkflowTriggerWire, String> {
    let ack: WorkflowTriggerAck = parse_command_response(message)?;
    if ack.run_id.trim().is_empty() {
        return Err("workflow trigger response contained an empty run_id".to_string());
    }
    Ok(WorkflowTriggerWire {
        run_id: ack.run_id,
        workflow_id,
        status: "pending".to_string(),
    })
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// First value of the tag whose name matches `name` (e.g. `d`, `h`).
fn tag_value(ev: &nostr::Event, name: &str) -> Option<String> {
    ev.tags.iter().find_map(|t| {
        let s = t.as_slice();
        (s.len() >= 2 && s[0] == name).then(|| s[1].clone())
    })
}

/// Parse a workflow's YAML body into a free-form JSON object. The frontend
/// consumes `definition` as `Record<string, unknown>`, so we preserve the full
/// document. On parse failure (or a non-object document) we fall back to an
/// empty object rather than failing the whole list query — a single malformed
/// workflow must not break the page.
fn parse_definition(yaml: &str) -> Value {
    match serde_yaml::from_str::<Value>(yaml) {
        Ok(v @ Value::Object(_)) => v,
        _ => Value::Object(serde_json::Map::new()),
    }
}

/// Build a [`WorkflowWire`] record from its parts. Shared by the read path
/// (from a relay event) and the write path (from local inputs).
fn workflow_record(
    id: String,
    revision: String,
    channel_id: Option<String>,
    owner_pubkey: String,
    yaml_definition: &str,
    created_at: i64,
    updated_at: i64,
) -> WorkflowWire {
    let definition = parse_definition(yaml_definition);
    let name = definition
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| id.clone());

    WorkflowWire {
        id,
        revision,
        name,
        owner_pubkey,
        channel_id,
        definition,
        status: "active".to_string(),
        created_at,
        updated_at,
    }
}

/// Convert a kind:30620 workflow definition event into a [`WorkflowWire`].
fn workflow_from_event(ev: &nostr::Event) -> WorkflowWire {
    let id = tag_value(ev, "d").unwrap_or_default();
    let channel_id = tag_value(ev, "h");
    let ts = ev.created_at.as_secs() as i64;
    workflow_record(
        id,
        ev.id.to_hex(),
        channel_id,
        ev.pubkey.to_hex(),
        &ev.content,
        ts,
        ts,
    )
}

#[cfg(test)]
#[path = "workflows_tests.rs"]
mod tests;
