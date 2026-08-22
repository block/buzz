//! Authorized reads and writes for task state.
//!
//! Tasks are relay-owned database rows, not Nostr events — the same modeling
//! choice `api::workflows` makes for runs and approvals, and for the same
//! reason: there is no synthetic event worth inventing for a work item whose
//! whole value is a queryable, mutable read model.
//!
//! Every route is scoped to the **host-derived** tenant, like the rest of the
//! relay's HTTP surface. There is no `/communities/{id}/…` path segment
//! anywhere in Buzz: `crate::tenant::bind_community` resolves the community
//! from the `Host` header, and NIP-98 signatures are bound to that same host,
//! so a client cannot name a community it did not connect to.
//!
//! Channel-bound tasks additionally require the caller to have access to the
//! bound channel, mirroring `api::workflows::authorize_workflow_read`.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use buzz_core::task::{TaskAction, TaskStatus};
use buzz_core::TenantContext;
use buzz_db::task::{NewTask, TaskEventRecord, TaskFilter, TaskPatch, TaskRecord};

use crate::{
    api::{api_error, bridge, internal_error},
    state::AppState,
};

const DEFAULT_TASK_LIMIT: i64 = 50;
const MAX_TASK_LIMIT: i64 = 200;
const MAX_TITLE_CHARS: usize = 200;

/// Query filters for `GET /api/tasks`.
#[derive(Debug, Deserialize, Default)]
pub struct TasksQuery {
    status: Option<String>,
    assignee: Option<String>,
    channel: Option<Uuid>,
    include_archived: Option<bool>,
    limit: Option<i64>,
}

/// Body of `POST /api/tasks`.
#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    title: String,
    body: Option<String>,
    channel_id: Option<Uuid>,
    parent_task_id: Option<Uuid>,
    assignee: Option<String>,
    priority: Option<i32>,
    due_at: Option<DateTime<Utc>>,
    source: Option<String>,
    source_ref: Option<String>,
}

/// Body of `PATCH /api/tasks/{id}`.
///
/// `assignee` and `due_at` are doubly optional on the wire: an absent key
/// leaves the field alone, while an explicit `null` clears it. `serde`'s
/// `double_option` shape (`Option<Option<T>>` with
/// `skip_serializing_if`/`default`) is what distinguishes the two.
#[derive(Debug, Deserialize, Default)]
pub struct UpdateTaskRequest {
    status: Option<String>,
    title: Option<String>,
    priority: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    due_at: Option<Option<DateTime<Utc>>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    assignee: Option<Option<String>>,
}

/// Body of `POST /api/tasks/{id}/events`.
#[derive(Debug, Deserialize)]
pub struct AppendTaskEventRequest {
    action: Option<String>,
    body: Option<String>,
}

fn deserialize_double_option<'de, D, T>(
    deserializer: D,
) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

fn request_path(path: &str, raw_query: Option<&str>) -> String {
    match raw_query {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path.to_string(),
    }
}

/// Parse a 32-byte pubkey from lowercase hex, rejecting anything else.
fn parse_pubkey(field: &str, raw: &str) -> Result<Vec<u8>, (StatusCode, Json<Value>)> {
    let bytes = hex::decode(raw)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, &format!("{field} must be hex")))?;
    if bytes.len() != 32 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            &format!("{field} must be a 32-byte pubkey"),
        ));
    }
    Ok(bytes)
}

fn parse_status(raw: &str) -> Result<TaskStatus, (StatusCode, Json<Value>)> {
    raw.parse::<TaskStatus>()
        .map_err(|message| api_error(StatusCode::BAD_REQUEST, &message))
}

/// Reject titles the database's `CHECK (length(title) BETWEEN 1 AND 200)`
/// would reject, so the caller gets a 400 instead of a 500.
///
/// The check counts characters, matching PostgreSQL's `length()` on `TEXT`
/// (which counts characters, not bytes) — using `String::len` here would let a
/// 200-character multi-byte title fail in the database after passing this gate.
fn validate_title(title: &str) -> Result<String, (StatusCode, Json<Value>)> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "title must not be empty",
        ));
    }
    if trimmed.chars().count() > MAX_TITLE_CHARS {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "title must be at most 200 characters",
        ));
    }
    Ok(trimmed.to_owned())
}

/// Map a database error onto the narrowest status the caller can act on.
///
/// Foreign-key violations here always mean the caller named a channel, parent
/// task, or assignee that does not exist in this community — a request error,
/// not a server fault.
fn map_task_error(context: &str, error: buzz_db::DbError) -> (StatusCode, Json<Value>) {
    match &error {
        buzz_db::DbError::NotFound(_) => api_error(StatusCode::NOT_FOUND, "task not found"),
        buzz_db::DbError::InvalidData(message) => api_error(StatusCode::BAD_REQUEST, message),
        buzz_db::DbError::AccessDenied(_) => api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "community writes are temporarily unavailable",
        ),
        buzz_db::DbError::Sqlx(sqlx::Error::Database(db_error))
            if db_error.code().as_deref() == Some("23503") =>
        {
            api_error(
                StatusCode::BAD_REQUEST,
                "channel, parent task, or assignee does not exist in this community",
            )
        }
        buzz_db::DbError::Sqlx(sqlx::Error::Database(db_error))
            if db_error.code().as_deref() == Some("23514") =>
        {
            api_error(StatusCode::BAD_REQUEST, "task violates a field constraint")
        }
        _ => internal_error(&format!("{context}: {error}")),
    }
}

/// Authenticate the caller and bind the request to its host-derived tenant.
///
/// `body` is `Some` for writes; NIP-98 then additionally requires a `payload`
/// tag covering it, so a signature cannot be replayed against a different body.
async fn authorize_task_request(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    raw_query: Option<&str>,
    body: Option<&[u8]>,
) -> Result<(TenantContext, nostr::PublicKey), (StatusCode, Json<Value>)> {
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
        })?;

    let path_with_query = request_path(path, raw_query);
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, &path_with_query);
    let (pubkey, event_id_bytes) = bridge::verify_bridge_auth_with_options(
        headers,
        method,
        &url,
        body,
        state.config.require_auth_token,
        body.is_some(),
    )?;
    bridge::enforce_http_admission(state, &tenant, &pubkey).await?;
    bridge::check_nip98_replay(state, &tenant, event_id_bytes).await?;

    let pubkey_bytes = pubkey.to_bytes().to_vec();
    let auth_tag = headers
        .get("x-auth-tag")
        .and_then(|value| value.to_str().ok());
    super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
    )
    .await?;

    Ok((tenant, pubkey))
}

/// Reject access to a channel-bound task the caller cannot see.
///
/// A task with no channel is community-wide and needs no further check; relay
/// membership already gated it.
async fn enforce_channel_access(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    pubkey: &nostr::PublicKey,
    channel_id: Option<Uuid>,
) -> Result<(), (StatusCode, Json<Value>)> {
    let Some(channel_id) = channel_id else {
        return Ok(());
    };
    let accessible = state
        .get_accessible_channel_ids_cached(tenant.community(), &pubkey.to_bytes())
        .await
        .map_err(|error| internal_error(&format!("task channel access lookup: {error}")))?;
    if !accessible.contains(&channel_id) {
        // 404, not 403: the caller cannot see this channel, so it must not
        // learn that a task exists in it.
        return Err(api_error(StatusCode::NOT_FOUND, "task not found"));
    }
    Ok(())
}

/// `POST /api/tasks` — create a task. Requires relay membership.
pub async fn create_task(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (tenant, pubkey) =
        authorize_task_request(&state, &headers, "POST", "/api/tasks", None, Some(&body)).await?;

    let request: CreateTaskRequest = serde_json::from_slice(&body)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid task JSON: {e}")))?;

    let title = validate_title(&request.title)?;
    let assignee = request
        .assignee
        .as_deref()
        .map(|raw| parse_pubkey("assignee", raw))
        .transpose()?;

    enforce_channel_access(&state, &tenant, &pubkey, request.channel_id).await?;

    // `tasks.created_by_pubkey` is a community-scoped FK into `users`. An
    // authenticated member may still have no `users` row yet (it is created
    // lazily on first profile write), so materialize it before the insert.
    let creator = pubkey.to_bytes().to_vec();
    state
        .db
        .ensure_user(tenant.community(), &creator)
        .await
        .map_err(|error| internal_error(&format!("ensure task creator: {error}")))?;

    let task = state
        .db
        .create_task(
            tenant.community(),
            NewTask {
                channel_id: request.channel_id,
                created_by_pubkey: Some(creator),
                assignee_pubkey: assignee,
                parent_task_id: request.parent_task_id,
                title,
                body: request.body,
                priority: request.priority.unwrap_or(0),
                // 'app' marks a task typed by a person in a Buzz client, as
                // distinct from one a harness opened on their behalf.
                source: Some(request.source.unwrap_or_else(|| "app".to_owned())),
                source_ref: request.source_ref,
                due_at: request.due_at,
            },
        )
        .await
        .map_err(|error| map_task_error("create task", error))?;

    Ok(Json(task_json(&task)))
}

/// `GET /api/tasks` — list this community's tasks, newest-modified first.
pub async fn list_tasks(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    Query(query): Query<TasksQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let limit = query.limit.unwrap_or(DEFAULT_TASK_LIMIT);
    if !(1..=MAX_TASK_LIMIT).contains(&limit) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "limit must be between 1 and 200",
        ));
    }
    let status = query.status.as_deref().map(parse_status).transpose()?;
    let assignee = query
        .assignee
        .as_deref()
        .map(|raw| parse_pubkey("assignee", raw))
        .transpose()?;

    let (tenant, pubkey) = authorize_task_request(
        &state,
        &headers,
        "GET",
        "/api/tasks",
        raw_query.as_deref(),
        None,
    )
    .await?;

    if let Some(channel_id) = query.channel {
        enforce_channel_access(&state, &tenant, &pubkey, Some(channel_id)).await?;
    }

    let tasks = state
        .db
        .list_tasks(
            tenant.community(),
            &TaskFilter {
                status,
                assignee_pubkey: assignee,
                channel_id: query.channel,
                include_archived: query.include_archived.unwrap_or(false),
                limit,
            },
        )
        .await
        .map_err(|error| map_task_error("list tasks", error))?;

    // Channel-bound tasks the caller cannot see are filtered out rather than
    // failing the whole page: a list is a view of what you may see.
    let accessible = state
        .get_accessible_channel_ids_cached(tenant.community(), &pubkey.to_bytes())
        .await
        .map_err(|error| internal_error(&format!("task channel access lookup: {error}")))?;
    let visible: Vec<Value> = tasks
        .iter()
        .filter(|task| {
            task.channel_id
                .is_none_or(|channel_id| accessible.contains(&channel_id))
        })
        .map(task_json)
        .collect();

    Ok(Json(serde_json::json!({ "tasks": visible })))
}

/// `GET /api/tasks/{id}` — one task plus its full event history.
pub async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = format!("/api/tasks/{task_id}");
    let (tenant, pubkey) =
        authorize_task_request(&state, &headers, "GET", &path, None, None).await?;

    let task = state
        .db
        .get_task(tenant.community(), task_id)
        .await
        .map_err(|error| map_task_error("get task", error))?;
    enforce_channel_access(&state, &tenant, &pubkey, task.channel_id).await?;

    let events = state
        .db
        .list_task_events(tenant.community(), task_id)
        .await
        .map_err(|error| map_task_error("list task events", error))?;

    Ok(Json(serde_json::json!({
        "task": task_json(&task),
        "events": events.iter().map(task_event_json).collect::<Vec<_>>(),
    })))
}

/// `PATCH /api/tasks/{id}` — update a task, appending its history entries.
pub async fn update_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<Uuid>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = format!("/api/tasks/{task_id}");
    let (tenant, pubkey) =
        authorize_task_request(&state, &headers, "PATCH", &path, None, Some(&body)).await?;

    let request: UpdateTaskRequest = serde_json::from_slice(&body)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid task JSON: {e}")))?;

    let patch = TaskPatch {
        status: request.status.as_deref().map(parse_status).transpose()?,
        title: request.title.as_deref().map(validate_title).transpose()?,
        priority: request.priority,
        due_at: request.due_at,
        assignee_pubkey: match request.assignee {
            None => None,
            Some(None) => Some(None),
            Some(Some(raw)) => Some(Some(parse_pubkey("assignee", &raw)?)),
        },
    };
    if patch.is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "patch must change at least one field",
        ));
    }

    // Authorize against the task's *current* channel before mutating it.
    let existing = state
        .db
        .get_task(tenant.community(), task_id)
        .await
        .map_err(|error| map_task_error("get task", error))?;
    enforce_channel_access(&state, &tenant, &pubkey, existing.channel_id).await?;

    let task = state
        .db
        .update_task(
            tenant.community(),
            task_id,
            &patch,
            Some(&pubkey.to_bytes()),
        )
        .await
        .map_err(|error| map_task_error("update task", error))?;

    Ok(Json(task_json(&task)))
}

/// `POST /api/tasks/{id}/events` — append a comment or summary.
pub async fn append_task_event(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<Uuid>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = format!("/api/tasks/{task_id}/events");
    let (tenant, pubkey) =
        authorize_task_request(&state, &headers, "POST", &path, None, Some(&body)).await?;

    let request: AppendTaskEventRequest = serde_json::from_slice(&body)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid event JSON: {e}")))?;

    let action = match request.action.as_deref() {
        None => TaskAction::Commented,
        Some(raw) => raw
            .parse::<TaskAction>()
            .map_err(|message| api_error(StatusCode::BAD_REQUEST, &message))?,
    };
    // Lifecycle actions are derived from the mutation that caused them; letting
    // a caller post one directly would let it fabricate a transition history
    // that never happened.
    if !matches!(action, TaskAction::Commented | TaskAction::SummaryPersisted) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "only 'commented' and 'summary_persisted' may be posted directly",
        ));
    }
    let event_body = request
        .body
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "body must not be empty"))?;

    let task = state
        .db
        .get_task(tenant.community(), task_id)
        .await
        .map_err(|error| map_task_error("get task", error))?;
    enforce_channel_access(&state, &tenant, &pubkey, task.channel_id).await?;

    let actor = pubkey.to_bytes().to_vec();
    state
        .db
        .ensure_user(tenant.community(), &actor)
        .await
        .map_err(|error| internal_error(&format!("ensure task actor: {error}")))?;

    let event = state
        .db
        .append_task_event(
            tenant.community(),
            task_id,
            Some(&actor),
            action,
            Some(event_body),
        )
        .await
        .map_err(|error| map_task_error("append task event", error))?;

    Ok(Json(task_event_json(&event)))
}

fn task_json(task: &TaskRecord) -> Value {
    serde_json::json!({
        "id": task.id,
        "channel_id": task.channel_id,
        "created_by": task.created_by_pubkey.as_ref().map(hex::encode),
        "assignee": task.assignee_pubkey.as_ref().map(hex::encode),
        "parent_task_id": task.parent_task_id,
        "title": task.title,
        "body": task.body,
        "status": task.status.as_str(),
        "priority": task.priority,
        "source": task.source,
        "source_ref": task.source_ref,
        "due_at": task.due_at.map(|value| value.timestamp()),
        "done_at": task.done_at.map(|value| value.timestamp()),
        "archived_at": task.archived_at.map(|value| value.timestamp()),
        "created_at": task.created_at.timestamp(),
        "updated_at": task.updated_at.timestamp(),
    })
}

fn task_event_json(event: &TaskEventRecord) -> Value {
    serde_json::json!({
        "id": event.id,
        "task_id": event.task_id,
        "actor": event.actor_pubkey.as_ref().map(hex::encode),
        "action": event.action.as_str(),
        "from_status": event.from_status.map(|status| status.as_str()),
        "to_status": event.to_status.map(|status| status.as_str()),
        "body": event.body,
        "created_at": event.created_at.timestamp(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_path_preserves_signed_query_verbatim() {
        assert_eq!(
            request_path("/api/tasks", Some("status=todo&limit=10")),
            "/api/tasks?status=todo&limit=10"
        );
        assert_eq!(request_path("/api/tasks", None), "/api/tasks");
        assert_eq!(request_path("/api/tasks", Some("")), "/api/tasks");
    }

    #[test]
    fn title_length_is_counted_in_characters_not_bytes() {
        // 200 multi-byte characters is 600 bytes but a legal title; counting
        // bytes here would 400 a request the database would have accepted.
        let multibyte = "é".repeat(200);
        assert_eq!(
            validate_title(&multibyte).expect("200 chars is legal"),
            multibyte
        );
        assert!(validate_title(&"é".repeat(201)).is_err());
    }

    #[test]
    fn title_is_trimmed_and_must_not_be_blank() {
        assert_eq!(validate_title("  ship it  ").expect("trims"), "ship it");
        assert!(validate_title("   ").is_err());
        assert!(validate_title("").is_err());
    }

    #[test]
    fn assignee_must_be_a_32_byte_hex_pubkey() {
        let valid = "ab".repeat(32);
        assert_eq!(
            parse_pubkey("assignee", &valid).expect("valid"),
            vec![0xab; 32]
        );
        assert!(parse_pubkey("assignee", "not-hex").is_err());
        assert!(parse_pubkey("assignee", &"ab".repeat(31)).is_err());
        assert!(parse_pubkey("assignee", &"ab".repeat(33)).is_err());
    }

    #[test]
    fn absent_and_null_assignee_are_different_patches() {
        // The whole point of the double option: `{}` leaves the assignee
        // alone, `{"assignee": null}` unassigns.
        let absent: UpdateTaskRequest = serde_json::from_str("{}").expect("absent");
        assert_eq!(absent.assignee, None);

        let cleared: UpdateTaskRequest =
            serde_json::from_str(r#"{"assignee": null}"#).expect("null");
        assert_eq!(cleared.assignee, Some(None));

        let set: UpdateTaskRequest = serde_json::from_str(r#"{"assignee": "abc"}"#).expect("set");
        assert_eq!(set.assignee, Some(Some("abc".to_owned())));
    }

    #[test]
    fn absent_and_null_due_at_are_different_patches() {
        let absent: UpdateTaskRequest = serde_json::from_str("{}").expect("absent");
        assert_eq!(absent.due_at, None);

        let cleared: UpdateTaskRequest = serde_json::from_str(r#"{"due_at": null}"#).expect("null");
        assert_eq!(cleared.due_at, Some(None));
    }

    #[test]
    fn task_wire_renders_status_and_hex_pubkeys() {
        let task = TaskRecord {
            id: Uuid::nil(),
            channel_id: None,
            created_by_pubkey: Some(vec![0xab; 32]),
            assignee_pubkey: None,
            parent_task_id: None,
            title: "ship it".to_owned(),
            body: None,
            status: TaskStatus::InProgress,
            priority: 3,
            source: Some("claude".to_owned()),
            source_ref: None,
            due_at: None,
            done_at: None,
            archived_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let wire = task_json(&task);
        assert_eq!(wire["status"], "in_progress");
        assert_eq!(wire["created_by"], hex::encode([0xab; 32]));
        assert!(wire["assignee"].is_null());
        assert_eq!(wire["priority"], 3);
        // Raw bytes must never reach the wire.
        assert!(wire.get("created_by_pubkey").is_none());
    }

    #[test]
    fn task_event_wire_renders_both_status_ends() {
        let event = TaskEventRecord {
            id: 7,
            task_id: Uuid::nil(),
            actor_pubkey: None,
            action: TaskAction::StatusChanged,
            from_status: Some(TaskStatus::Todo),
            to_status: Some(TaskStatus::Done),
            body: None,
            created_at: Utc::now(),
        };
        let wire = task_event_json(&event);
        assert_eq!(wire["action"], "status_changed");
        assert_eq!(wire["from_status"], "todo");
        assert_eq!(wire["to_status"], "done");
    }
}
