//! Owner-private, read-only Buzz Tasks HTTP API.
//!
//! The API resolves the tenant from `Host`, authenticates the exact GET URL
//! with NIP-98, applies the shared replay and relay-membership fences, and then
//! reads only rows addressed to the authenticated owner in channels they can
//! currently access. Native URLs are constructed only after those checks.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, RawQuery, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::Json,
};
use base64::Engine;
use buzz_auth::Scope;
use buzz_core::task::TaskTarget;
use buzz_core::TenantContext;
use buzz_db::task::{TaskDueBucket, TaskListQuery, TaskRecord, TaskStatus};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::state::AppState;

use super::bridge::{check_nip98_replay, nip98_expected_url, verify_bridge_auth};
use super::{api_error, internal_error};

const DEFAULT_PAGE_SIZE: i64 = 50;
const MAX_PAGE_SIZE: i64 = 100;

/// Supported query parameters for the Buzz Tasks list endpoint.
#[derive(Debug, Deserialize, Default)]
pub struct TaskListParams {
    status: Option<String>,
    bucket: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
    tz_offset_minutes: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskCursor {
    offset: i64,
    as_of: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskPage {
    items: Vec<TaskResponse>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskResponse {
    id: Uuid,
    community_id: Uuid,
    assignee_pubkey: String,
    channel_id: Uuid,
    source_event_id: String,
    agent_pubkey: String,
    agent_name: String,
    task_type: String,
    title: String,
    context: Option<String>,
    priority: String,
    due_at: Option<DateTime<Utc>>,
    status: &'static str,
    source_created_at: DateTime<Utc>,
    source_version: i64,
    source_updated_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
    navigation_url: String,
}

/// `GET /api/buzz-tasks` — owner-private task list.
pub async fn list_tasks(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    Query(params): Query<TaskListParams>,
) -> Result<(HeaderMap, Json<Value>), (StatusCode, Json<Value>)> {
    let (tenant, pubkey) =
        authorize_task_read(&state, &headers, "/api/buzz-tasks", raw_query.as_deref()).await?;
    let owner_bytes = pubkey.to_bytes();
    let accessible_channels = state
        .get_accessible_channel_ids_cached(tenant.community(), owner_bytes.as_slice())
        .await
        .map_err(|error| internal_error(&format!("task channel access lookup: {error}")))?;

    let page_size = params
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let cursor = params
        .cursor
        .as_deref()
        .map(decode_cursor)
        .transpose()?
        .unwrap_or_else(|| TaskCursor {
            offset: 0,
            as_of: Utc::now(),
        });
    let query = TaskListQuery {
        status: parse_status(params.status.as_deref())?,
        bucket: parse_bucket(
            params.bucket.as_deref(),
            params.tz_offset_minutes,
            cursor.as_of,
        )?,
        limit: page_size + 1,
        offset: cursor.offset,
        as_of: cursor.as_of,
    };
    let mut rows = state
        .db
        .list_tasks_for_owner(
            tenant.community(),
            owner_bytes.as_slice(),
            &accessible_channels,
            &query,
        )
        .await
        .map_err(|error| internal_error(&format!("task list query: {error}")))?;
    let has_more = rows.len() as i64 > page_size;
    if has_more {
        rows.pop();
    }
    let items = rows
        .into_iter()
        .map(TaskResponse::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = has_more
        .then(|| {
            encode_cursor(&TaskCursor {
                offset: cursor.offset + page_size,
                as_of: cursor.as_of,
            })
        })
        .transpose()?;

    serde_json::to_value(TaskPage { items, next_cursor })
        .map(private_json)
        .map_err(|error| internal_error(&format!("task list response: {error}")))
}

/// `GET /api/buzz-tasks/{task_id}` — owner-private task detail.
pub async fn get_task(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    Path(task_id): Path<Uuid>,
) -> Result<(HeaderMap, Json<Value>), (StatusCode, Json<Value>)> {
    let path = format!("/api/buzz-tasks/{task_id}");
    let (tenant, pubkey) =
        authorize_task_read(&state, &headers, &path, raw_query.as_deref()).await?;
    let owner_bytes = pubkey.to_bytes();
    let accessible_channels = state
        .get_accessible_channel_ids_cached(tenant.community(), owner_bytes.as_slice())
        .await
        .map_err(|error| internal_error(&format!("task channel access lookup: {error}")))?;
    let row = state
        .db
        .get_task_for_owner(tenant.community(), owner_bytes.as_slice(), task_id)
        .await
        .map_err(|error| internal_error(&format!("task detail query: {error}")))?
        .filter(|task| accessible_channels.contains(&task.channel_id))
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "task not found"))?;
    let response = TaskResponse::try_from(row)?;
    serde_json::to_value(response)
        .map(private_json)
        .map_err(|error| internal_error(&format!("task detail response: {error}")))
}

async fn authorize_task_read(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    raw_query: Option<&str>,
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
    let url = task_read_expected_url(&state.config.relay_url, &tenant, path, raw_query);
    let (pubkey, event_id) = verify_bridge_auth(headers, "GET", &url, None, true)?;
    super::bridge::enforce_http_admission(state, &tenant, &pubkey).await?;
    check_nip98_replay(state, &tenant, event_id).await?;

    // Pure-Nostr HTTP authentication maps a verified signer to the complete
    // scope set. This explicit check preserves the scope boundary if the bridge
    // later carries restricted credentials; channel and owner checks remain the
    // effective data-authorization fences today.
    buzz_auth::require_scope(&Scope::all_known(), Scope::MessagesRead).map_err(|_| {
        api_error(
            StatusCode::FORBIDDEN,
            "restricted: messages:read scope required",
        )
    })?;

    let pubkey_bytes = pubkey.to_bytes();
    let auth_tag = headers
        .get("x-auth-tag")
        .and_then(|value| value.to_str().ok());
    super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        pubkey_bytes.as_slice(),
        auth_tag,
    )
    .await?;
    Ok((tenant, pubkey))
}

fn task_read_expected_url(
    config_relay_url: &str,
    tenant: &TenantContext,
    path: &str,
    raw_query: Option<&str>,
) -> String {
    let path_with_query = match raw_query {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path.to_string(),
    };
    nip98_expected_url(config_relay_url, tenant, &path_with_query)
}

fn private_json(value: Value) -> (HeaderMap, Json<Value>) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::VARY,
        HeaderValue::from_static("authorization, x-auth-tag, host"),
    );
    (headers, Json(value))
}

fn parse_status(value: Option<&str>) -> Result<Option<TaskStatus>, (StatusCode, Json<Value>)> {
    match value.unwrap_or("open") {
        "open" => Ok(Some(TaskStatus::Open)),
        "resolved" => Ok(Some(TaskStatus::Resolved)),
        "withdrawn" => Ok(Some(TaskStatus::Withdrawn)),
        "all" => Ok(None),
        _ => Err(api_error(
            StatusCode::BAD_REQUEST,
            "status must be open, resolved, withdrawn, or all",
        )),
    }
}

fn parse_bucket(
    value: Option<&str>,
    tz_offset_minutes: Option<i32>,
    as_of: DateTime<Utc>,
) -> Result<TaskDueBucket, (StatusCode, Json<Value>)> {
    match value.unwrap_or("all") {
        "all" => Ok(TaskDueBucket::All),
        "today" | "later" => {
            let offset = tz_offset_minutes.ok_or_else(|| {
                api_error(
                    StatusCode::BAD_REQUEST,
                    "tz_offset_minutes is required for today/later",
                )
            })?;
            if !(-840..=840).contains(&offset) {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "tz_offset_minutes must be between -840 and 840",
                ));
            }
            let local_now = as_of
                .checked_add_signed(Duration::minutes(offset.into()))
                .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "invalid local-day offset"))?;
            let next_date = local_now
                .date_naive()
                .succ_opt()
                .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "invalid local-day boundary"))?;
            let local_midnight = next_date
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "invalid local-day boundary"))?
                .and_utc();
            let boundary = local_midnight
                .checked_sub_signed(Duration::minutes(offset.into()))
                .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "invalid local-day offset"))?;
            if value == Some("today") {
                Ok(TaskDueBucket::Today { end: boundary })
            } else {
                Ok(TaskDueBucket::Later { start: boundary })
            }
        }
        _ => Err(api_error(
            StatusCode::BAD_REQUEST,
            "bucket must be all, today, or later",
        )),
    }
}

fn encode_cursor(cursor: &TaskCursor) -> Result<String, (StatusCode, Json<Value>)> {
    let bytes = serde_json::to_vec(cursor)
        .map_err(|error| internal_error(&format!("task cursor encode: {error}")))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn decode_cursor(value: &str) -> Result<TaskCursor, (StatusCode, Json<Value>)> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid task cursor"))?;
    let cursor: TaskCursor = serde_json::from_slice(&bytes)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid task cursor"))?;
    if cursor.offset < 0 || cursor.as_of > Utc::now() + Duration::minutes(5) {
        return Err(api_error(StatusCode::BAD_REQUEST, "invalid task cursor"));
    }
    Ok(cursor)
}

impl TryFrom<TaskRecord> for TaskResponse {
    type Error = (StatusCode, Json<Value>);

    fn try_from(task: TaskRecord) -> Result<Self, Self::Error> {
        let target =
            TaskTarget::from_bytes(task.community_id, task.channel_id, &task.source_event_id)
                .map_err(|error| internal_error(&format!("invalid stored task target: {error}")))?;
        let assignee_pubkey = nostr::PublicKey::from_slice(&task.assignee_pubkey)
            .map_err(|_| internal_error("invalid stored task owner pubkey"))?;
        let agent_pubkey = nostr::PublicKey::from_slice(&task.agent_pubkey)
            .map_err(|_| internal_error("invalid stored task agent pubkey"))?;
        let status = match task.status {
            TaskStatus::Open => "open",
            TaskStatus::Resolved => "resolved",
            TaskStatus::Withdrawn => "withdrawn",
        };
        Ok(Self {
            id: task.id,
            community_id: *task.community_id.as_uuid(),
            assignee_pubkey: assignee_pubkey.to_hex(),
            channel_id: task.channel_id,
            source_event_id: target.source_event_id().to_hex(),
            agent_pubkey: agent_pubkey.to_hex(),
            agent_name: task.agent_name,
            task_type: task.task_type,
            title: task.title,
            context: task.context,
            priority: task.priority,
            due_at: task.due_at,
            status,
            source_created_at: task.source_created_at,
            source_version: task.source_version,
            source_updated_at: task.source_updated_at,
            resolved_at: task.resolved_at,
            navigation_url: target.navigation_url(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::CommunityId;
    use chrono::TimeZone;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn nip98_headers(keys: &Keys, url: &str) -> HeaderMap {
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::parse(["u", url]).unwrap(),
                Tag::parse(["method", "GET"]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap();
        let json = serde_json::to_vec(&event).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!(
                "Nostr {}",
                base64::engine::general_purpose::STANDARD.encode(json)
            )
            .parse()
            .unwrap(),
        );
        headers
    }

    #[test]
    fn cursor_round_trip_preserves_snapshot_and_offset() {
        let cursor = TaskCursor {
            offset: 50,
            as_of: Utc.with_ymd_and_hms(2026, 8, 13, 9, 30, 0).unwrap(),
        };
        assert_eq!(
            decode_cursor(&encode_cursor(&cursor).unwrap()).unwrap(),
            cursor
        );
        assert!(decode_cursor("not-base64!").is_err());
    }

    #[test]
    fn local_day_bucket_uses_explicit_offset() {
        let as_of = Utc.with_ymd_and_hms(2026, 8, 13, 22, 30, 0).unwrap();
        assert_eq!(
            parse_bucket(Some("today"), Some(120), as_of).unwrap(),
            TaskDueBucket::Today {
                end: Utc.with_ymd_and_hms(2026, 8, 14, 22, 0, 0).unwrap()
            }
        );
        assert!(parse_bucket(Some("later"), None, as_of).is_err());
    }

    #[test]
    fn response_builds_exact_navigation_url_from_validated_identity() {
        let community_id = CommunityId::from_uuid(Uuid::new_v4());
        let channel_id = Uuid::parse_str("1487447e-0f26-4bc5-8865-f5be07195579").unwrap();
        let response = TaskResponse::try_from(TaskRecord {
            community_id,
            id: Uuid::new_v4(),
            assignee_pubkey: vec![1; 32],
            channel_id,
            source_event_id: vec![0xab; 32],
            agent_pubkey: vec![2; 32],
            agent_name: "Agent".into(),
            task_type: "review".into(),
            title: "Review".into(),
            context: None,
            priority: "medium".into(),
            due_at: None,
            status: TaskStatus::Open,
            source_created_at: Utc.with_ymd_and_hms(2026, 8, 13, 8, 0, 0).unwrap(),
            source_version: 1,
            source_updated_at: Utc.with_ymd_and_hms(2026, 8, 13, 8, 1, 0).unwrap(),
            resolved_at: None,
        })
        .unwrap();
        assert_eq!(
            response.navigation_url,
            "buzz://message?channel=1487447e-0f26-4bc5-8865-f5be07195579&id=abababababababababababababababababababababababababababababababab"
        );
    }

    #[test]
    fn task_get_auth_is_bound_to_tenant_host_path_and_raw_query() {
        let tenant =
            TenantContext::resolved(CommunityId::from_uuid(Uuid::new_v4()), "tasks.example");
        let expected = task_read_expected_url(
            "wss://deployment.example",
            &tenant,
            "/api/buzz-tasks",
            Some("status=open&bucket=today&tz_offset_minutes=120"),
        );
        let keys = Keys::generate();
        let headers = nip98_headers(&keys, &expected);
        assert_eq!(
            verify_bridge_auth(&headers, "GET", &expected, None, true)
                .unwrap()
                .0,
            keys.public_key()
        );

        let bare =
            task_read_expected_url("wss://deployment.example", &tenant, "/api/buzz-tasks", None);
        assert_eq!(
            verify_bridge_auth(&headers, "GET", &bare, None, true)
                .unwrap_err()
                .0,
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn task_get_never_accepts_dev_pubkey_as_nip98_proof() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-pubkey",
            Keys::generate().public_key().to_hex().parse().unwrap(),
        );
        assert_eq!(
            verify_bridge_auth(
                &headers,
                "GET",
                "https://tasks.example/api/buzz-tasks",
                None,
                true,
            )
            .unwrap_err()
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
}
