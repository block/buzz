//! Authenticated App HTTP callback → exactly-once relay-signed kind 9.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use buzz_core::app::{
    parse_canonical_app_id, AppStatus, APP_CALLBACK_BODY_MAX_BYTES,
    APP_CALLBACK_METADATA_MAX_BYTES, APP_CALLBACK_METADATA_MAX_DEPTH,
    APP_CALLBACK_METADATA_MAX_NODES, APP_SECRET_BYTES,
};
use buzz_core::kind::{KIND_GIT_REPO_ANNOUNCEMENT, KIND_PROJECT, KIND_STREAM_MESSAGE};
use buzz_core::tenant::{CommunityId, TenantContext};
use buzz_db::app_admission::{
    AppAdmissionGuard, AppDeliveryFailure, AppDeliveryRecord, AppDeliveryStatus, AppRouteSnapshot,
    BeginAppAdmission,
};
use buzz_workflow::routing::{canonical_payload_hash, hash_idempotency_key, strip_idempotency_key};
use buzz_workflow::RouteFailure;
use nostr::{Event, EventBuilder, Kind, Tag};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::api::api_error_with_code;
use crate::handlers::event::dispatch_persistent_event;
use crate::message_mentions::resolve_mention_pubkeys;
use crate::project_route::{
    authorize_unique_project_route, project_head_from_event, repository_head_from_event,
    resolve_repository_identity, ProjectHead, RepositoryHead,
};
use crate::state::AppState;

const RATE_WINDOW_SECS: u64 = 60;
const RATE_LIMIT: u64 = 60;
const IDEMPOTENCY_KEY_MAX_BYTES: usize = 512;
const REPOSITORY_NAME_MAX_BYTES: usize = 512;
/// Matches the `channels.name` column width.
const CHANNEL_NAME_MAX_BYTES: usize = 255;

const ALLOWED_KEYS: &[&str] = &[
    "idempotency_key",
    "repository_name",
    "channel_name",
    "event_type",
    "content",
    "metadata",
];

/// Handle `POST /hooks/apps/{app_id}` after the bridge extracts route data.
pub async fn handle_app_callback(
    state: Arc<AppState>,
    peer_ip: IpAddr,
    app_id_str: String,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let started = Instant::now();
    match handle_inner(state, peer_ip, app_id_str, headers, body).await {
        Ok((status, json, observation)) => {
            record_observation(observation, started.elapsed().as_millis());
            (status, json).into_response()
        }
        Err((response, observation)) => {
            record_observation(observation, started.elapsed().as_millis());
            response
        }
    }
}

struct Observation {
    community_id: Option<Uuid>,
    app_id: Option<Uuid>,
    delivery_id: Option<Uuid>,
    outcome: &'static str,
    code: &'static str,
}

fn record_observation(observation: Observation, duration_ms: u128) {
    metrics::counter!(
        "buzz_app_callback_admission_total",
        "outcome" => observation.outcome,
        "code" => observation.code
    )
    .increment(1);
    tracing::info!(
        community_id = ?observation.community_id,
        app_id = ?observation.app_id,
        delivery_id = ?observation.delivery_id,
        outcome = observation.outcome,
        code = observation.code,
        duration_ms,
        "app callback admission"
    );
}

fn observe(
    community_id: Option<Uuid>,
    app_id: Option<Uuid>,
    delivery_id: Option<Uuid>,
    outcome: &'static str,
    code: &'static str,
) -> Observation {
    Observation {
        community_id,
        app_id,
        delivery_id,
        outcome,
        code,
    }
}

fn json_err(status: StatusCode, msg: &str, code: &str) -> (StatusCode, Json<Value>) {
    api_error_with_code(status, msg, code)
}

fn invalid_app_id_err() -> Response {
    json_err(StatusCode::BAD_REQUEST, "invalid app id", "invalid_app_id").into_response()
}

fn app_not_found_err() -> Response {
    json_err(StatusCode::NOT_FOUND, "app not found", "app_not_found").into_response()
}

fn unauthorized_err() -> Response {
    json_err(
        StatusCode::UNAUTHORIZED,
        "authentication failed",
        "unauthorized",
    )
    .into_response()
}

fn invalid_callback_err() -> Response {
    json_err(
        StatusCode::BAD_REQUEST,
        "invalid callback",
        "invalid_callback",
    )
    .into_response()
}

fn invalid_control_err() -> Response {
    let failure = RouteFailure::InvalidControlFields;
    json_err(
        StatusCode::UNPROCESSABLE_ENTITY,
        failure.redacted_message(),
        failure.code(),
    )
    .into_response()
}

fn unavailable_err() -> Response {
    json_err(
        StatusCode::SERVICE_UNAVAILABLE,
        "service unavailable",
        "service_unavailable",
    )
    .into_response()
}

fn rate_limited_err(reset_in_secs: u64) -> Response {
    let mut response = json_err(
        StatusCode::TOO_MANY_REQUESTS,
        "rate limited",
        "rate_limited",
    )
    .into_response();
    if let Ok(value) = HeaderValue::from_str(&reset_in_secs.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

fn route_failure_err(failure: RouteFailure) -> Response {
    let status = match failure.http_status() {
        409 => StatusCode::CONFLICT,
        _ => StatusCode::UNPROCESSABLE_ENTITY,
    };
    json_err(status, failure.redacted_message(), failure.code()).into_response()
}

/// Exact Redis key for the App callback quota.
pub(crate) fn app_callback_rate_limit_key(
    community_id: CommunityId,
    app_id: Uuid,
    peer_ip: IpAddr,
) -> String {
    format!("buzz:{community_id}:ratelimit:app_callback:{app_id}:{peer_ip}")
}

fn content_type_is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        })
        .unwrap_or(false)
}

fn provided_secret_bytes(headers: &HeaderMap) -> Option<[u8; APP_SECRET_BYTES]> {
    let raw = headers.get("x-webhook-secret")?.to_str().ok()?;
    let decoded = URL_SAFE_NO_PAD.decode(raw).ok()?;
    <[u8; APP_SECRET_BYTES]>::try_from(decoded).ok()
}

fn secret_matches(provided: &[u8; APP_SECRET_BYTES], stored_hash: &[u8]) -> bool {
    let Ok(stored) = <[u8; 32]>::try_from(stored_hash) else {
        return false;
    };
    let digest: [u8; 32] = Sha256::digest(*provided).into();
    bool::from(digest.ct_eq(&stored))
}

pub(crate) fn valid_event_type(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    let first = bytes[0];
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    bytes[1..].iter().all(|b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b':' | b'-')
    })
}

fn metadata_bounds(value: &Value) -> Result<(), ()> {
    let serialized = serde_json::to_vec(value).map_err(|_| ())?;
    if serialized.len() > APP_CALLBACK_METADATA_MAX_BYTES {
        return Err(());
    }
    let (depth, nodes) = walk_metadata(value, 0)?;
    if depth > APP_CALLBACK_METADATA_MAX_DEPTH || nodes > APP_CALLBACK_METADATA_MAX_NODES {
        return Err(());
    }
    Ok(())
}

fn walk_metadata(value: &Value, depth: usize) -> Result<(usize, usize), ()> {
    match value {
        Value::Object(map) => {
            let this_depth = depth + 1;
            if this_depth > APP_CALLBACK_METADATA_MAX_DEPTH {
                return Err(());
            }
            let mut nodes = map.len();
            let mut max_depth = this_depth;
            for child in map.values() {
                let (child_depth, child_nodes) = walk_metadata(child, this_depth)?;
                max_depth = max_depth.max(child_depth);
                nodes += child_nodes;
                if nodes > APP_CALLBACK_METADATA_MAX_NODES {
                    return Err(());
                }
            }
            Ok((max_depth, nodes))
        }
        Value::Array(items) => {
            let this_depth = depth + 1;
            if this_depth > APP_CALLBACK_METADATA_MAX_DEPTH {
                return Err(());
            }
            let mut nodes = items.len();
            let mut max_depth = this_depth;
            for child in items {
                let (child_depth, child_nodes) = walk_metadata(child, this_depth)?;
                max_depth = max_depth.max(child_depth);
                nodes += child_nodes;
                if nodes > APP_CALLBACK_METADATA_MAX_NODES {
                    return Err(());
                }
            }
            Ok((max_depth, nodes))
        }
        _ => Ok((depth, 0)),
    }
}

#[derive(Debug)]
enum ParseError {
    InvalidCallback,
    InvalidControl,
}

/// Where the callback asks to be delivered. A named channel wins over a
/// repository when both are present.
enum CallbackTarget {
    Channel(String),
    Repository(String),
}

struct ValidatedCallback {
    key_hash: [u8; 32],
    target: CallbackTarget,
    event_type: String,
    content: String,
    payload: Value,
}

fn non_empty_string(
    obj: &Map<String, Value>,
    key: &str,
    max_bytes: usize,
) -> Result<String, ParseError> {
    let Some(value) = obj.get(key) else {
        return Err(ParseError::InvalidControl);
    };
    let Some(text) = value.as_str() else {
        return Err(ParseError::InvalidControl);
    };
    if text.is_empty() || text.len() > max_bytes {
        return Err(ParseError::InvalidControl);
    }
    Ok(text.to_owned())
}

fn parse_callback(body: Value) -> Result<ValidatedCallback, ParseError> {
    let Value::Object(obj) = &body else {
        return Err(ParseError::InvalidCallback);
    };
    for key in obj.keys() {
        if !ALLOWED_KEYS.contains(&key.as_str()) {
            return Err(ParseError::InvalidCallback);
        }
    }
    let idempotency_key = non_empty_string(obj, "idempotency_key", IDEMPOTENCY_KEY_MAX_BYTES)?;
    let target = if obj.contains_key("channel_name") {
        CallbackTarget::Channel(non_empty_string(
            obj,
            "channel_name",
            CHANNEL_NAME_MAX_BYTES,
        )?)
    } else {
        CallbackTarget::Repository(non_empty_string(
            obj,
            "repository_name",
            REPOSITORY_NAME_MAX_BYTES,
        )?)
    };
    let event_type = non_empty_string(obj, "event_type", 64)?;
    if !valid_event_type(&event_type) {
        return Err(ParseError::InvalidControl);
    }
    let content = non_empty_string(obj, "content", APP_CALLBACK_BODY_MAX_BYTES)?;
    if let Some(metadata) = obj.get("metadata") {
        if !metadata.is_object() || metadata_bounds(metadata).is_err() {
            return Err(ParseError::InvalidControl);
        }
    }
    let payload = strip_idempotency_key(body);
    let key_hash = hash_idempotency_key(&idempotency_key);
    drop(idempotency_key);
    Ok(ValidatedCallback {
        key_hash,
        target,
        event_type,
        content,
        payload,
    })
}

fn delivered_json(record: &AppDeliveryRecord, replayed: bool) -> (StatusCode, Json<Value>) {
    let event_id = record.event_id.map(hex::encode).unwrap_or_default();
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "delivery_id": record.id,
            "event_id": event_id,
            "status": "delivered",
            "replayed": replayed,
        })),
    )
}

enum ResolveRouteError {
    Deterministic(RouteFailure),
    Transient,
}

async fn resolve_app_route(
    guard: &mut AppAdmissionGuard,
    community_id: CommunityId,
    target: &CallbackTarget,
) -> Result<AppRouteSnapshot, ResolveRouteError> {
    match target {
        CallbackTarget::Channel(name) => resolve_channel_route(guard, community_id, name).await,
        CallbackTarget::Repository(name) => {
            resolve_repository_route(guard, community_id, name).await
        }
    }
}

async fn resolve_channel_route(
    guard: &mut AppAdmissionGuard,
    community_id: CommunityId,
    channel_name: &str,
) -> Result<AppRouteSnapshot, ResolveRouteError> {
    let channels = guard
        .find_live_stream_channels_by_name(channel_name)
        .await
        .map_err(|_| ResolveRouteError::Transient)?;
    let channel_id = match channels.as_slice() {
        [id] => *id,
        [] => {
            return Err(ResolveRouteError::Deterministic(
                RouteFailure::ChannelMissing,
            ))
        }
        _ => {
            return Err(ResolveRouteError::Deterministic(
                RouteFailure::ChannelAmbiguous,
            ))
        }
    };
    Ok(AppRouteSnapshot {
        community_id: *community_id.as_uuid(),
        repository_coordinate: None,
        project_coordinate: None,
        channel_id,
    })
}

async fn resolve_repository_route(
    guard: &mut AppAdmissionGuard,
    community_id: CommunityId,
    repository_name: &str,
) -> Result<AppRouteSnapshot, ResolveRouteError> {
    let repository_events = guard
        .list_latest_parameterized_heads(KIND_GIT_REPO_ANNOUNCEMENT as i32)
        .await
        .map_err(|_| ResolveRouteError::Transient)?;
    let repositories: Vec<RepositoryHead> = repository_events
        .iter()
        .filter_map(|stored| repository_head_from_event(&stored.event))
        .collect();
    let aliases = BTreeMap::new();
    let (repository_coordinate, _tier) =
        resolve_repository_identity(repository_name, &aliases, &repositories)
            .map_err(ResolveRouteError::Deterministic)?;
    let repository = repositories
        .iter()
        .find(|head| head.coordinate == repository_coordinate)
        .ok_or(ResolveRouteError::Deterministic(
            RouteFailure::RepositoryMissing,
        ))?;
    let project_events = guard
        .list_latest_parameterized_heads(KIND_PROJECT as i32)
        .await
        .map_err(|_| ResolveRouteError::Transient)?;
    let projects: Vec<ProjectHead> = project_events
        .iter()
        .filter_map(|stored| project_head_from_event(&stored.event))
        .collect();
    let project = authorize_unique_project_route(repository, &projects)
        .map_err(ResolveRouteError::Deterministic)?;
    let channel_id = project
        .buzz_channel
        .as_deref()
        .ok_or(ResolveRouteError::Deterministic(
            RouteFailure::ProjectChannelInvalid,
        ))?
        .parse::<Uuid>()
        .map_err(|_| ResolveRouteError::Deterministic(RouteFailure::ProjectChannelInvalid))?;
    guard
        .load_live_destination_channel(channel_id)
        .await
        .map_err(|_| ResolveRouteError::Transient)?
        .ok_or(ResolveRouteError::Deterministic(
            RouteFailure::ProjectChannelInvalid,
        ))?;
    Ok(AppRouteSnapshot {
        community_id: *community_id.as_uuid(),
        repository_coordinate: Some(repository_coordinate),
        project_coordinate: Some(project.coordinate.clone()),
        channel_id,
    })
}

fn named_member_pairs(
    members: &[buzz_db::app_admission::AppAdmissionNamedMember],
) -> Vec<(String, String)> {
    members
        .iter()
        .filter_map(|member| {
            let name = member.display_name.as_ref()?.clone();
            if name.trim().is_empty() {
                return None;
            }
            let pubkey = nostr::PublicKey::from_slice(&member.pubkey).ok()?.to_hex();
            Some((name, pubkey))
        })
        .collect()
}

fn build_app_event(
    state: &AppState,
    app_id: Uuid,
    delivery_id: Uuid,
    event_type: &str,
    content: &str,
    route: &AppRouteSnapshot,
    mention_pubkeys: &[String],
) -> Result<Event, ()> {
    let mut tags = vec![
        Tag::parse(["h", &route.channel_id.to_string()]).map_err(|_| ())?,
        Tag::parse(["buzz:app", &app_id.to_string()]).map_err(|_| ())?,
    ];
    if let Some(repository) = &route.repository_coordinate {
        tags.push(Tag::parse(["a", repository, "repository"]).map_err(|_| ())?);
    }
    if let Some(project) = &route.project_coordinate {
        tags.push(Tag::parse(["a", project, "project"]).map_err(|_| ())?);
    }
    tags.push(Tag::parse(["buzz:app-delivery", &delivery_id.to_string()]).map_err(|_| ())?);
    tags.push(Tag::parse(["buzz:app-event", event_type]).map_err(|_| ())?);
    for pubkey in mention_pubkeys {
        tags.push(Tag::parse(["p", pubkey]).map_err(|_| ())?);
    }
    EventBuilder::new(Kind::from(KIND_STREAM_MESSAGE as u16), content)
        .tags(tags)
        .sign_with_keys(&state.relay_keypair)
        .map_err(|_| ())
}

async fn handle_inner(
    state: Arc<AppState>,
    peer_ip: IpAddr,
    app_id_str: String,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<Value>, Observation), (Response, Observation)> {
    let app_id = parse_canonical_app_id(&app_id_str).map_err(|_| {
        (
            invalid_app_id_err(),
            observe(None, None, None, "rejected", "invalid_app_id"),
        )
    })?;

    let raw_host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            (
                app_not_found_err(),
                observe(None, Some(app_id), None, "rejected", "app_not_found"),
            )
        })?;
    let community_id = tenant.community();
    let community_uuid = *community_id.as_uuid();

    let app = state.db.get_app(community_id, app_id).await.map_err(|_| {
        (
            unavailable_err(),
            observe(
                Some(community_uuid),
                Some(app_id),
                None,
                "unavailable",
                "none",
            ),
        )
    })?;
    let Some(app) = app else {
        return Err((
            app_not_found_err(),
            observe(
                Some(community_uuid),
                Some(app_id),
                None,
                "rejected",
                "app_not_found",
            ),
        ));
    };
    if app.status != AppStatus::Active {
        return Err((
            app_not_found_err(),
            observe(
                Some(community_uuid),
                Some(app_id),
                None,
                "rejected",
                "app_not_found",
            ),
        ));
    }

    let rate_key = app_callback_rate_limit_key(community_id, app_id, peer_ip);
    let rate = state
        .admission_rate_limiter
        .check_named_key(&rate_key, RATE_WINDOW_SECS, RATE_LIMIT)
        .await
        .map_err(|_| {
            (
                unavailable_err(),
                observe(
                    Some(community_uuid),
                    Some(app_id),
                    None,
                    "unavailable",
                    "none",
                ),
            )
        })?;
    if !rate.allowed {
        return Err((
            rate_limited_err(rate.reset_in_secs),
            observe(
                Some(community_uuid),
                Some(app_id),
                None,
                "rejected",
                "rate_limited",
            ),
        ));
    }

    let Some(provided) = provided_secret_bytes(&headers) else {
        return Err((
            unauthorized_err(),
            observe(
                Some(community_uuid),
                Some(app_id),
                None,
                "rejected",
                "unauthorized",
            ),
        ));
    };
    if !secret_matches(&provided, &app.secret_hash) {
        return Err((
            unauthorized_err(),
            observe(
                Some(community_uuid),
                Some(app_id),
                None,
                "rejected",
                "unauthorized",
            ),
        ));
    }

    let bytes = to_bytes(body, APP_CALLBACK_BODY_MAX_BYTES)
        .await
        .map_err(|_| {
            (
                invalid_callback_err(),
                observe(
                    Some(community_uuid),
                    Some(app_id),
                    None,
                    "rejected",
                    "invalid_callback",
                ),
            )
        })?;
    if !content_type_is_json(&headers) {
        return Err((
            invalid_callback_err(),
            observe(
                Some(community_uuid),
                Some(app_id),
                None,
                "rejected",
                "invalid_callback",
            ),
        ));
    }
    let parsed_json: Value = serde_json::from_slice(&bytes).map_err(|_| {
        (
            invalid_callback_err(),
            observe(
                Some(community_uuid),
                Some(app_id),
                None,
                "rejected",
                "invalid_callback",
            ),
        )
    })?;
    let validated = match parse_callback(parsed_json) {
        Ok(validated) => validated,
        Err(ParseError::InvalidCallback) => {
            return Err((
                invalid_callback_err(),
                observe(
                    Some(community_uuid),
                    Some(app_id),
                    None,
                    "rejected",
                    "invalid_callback",
                ),
            ));
        }
        Err(ParseError::InvalidControl) => {
            return Err((
                invalid_control_err(),
                observe(
                    Some(community_uuid),
                    Some(app_id),
                    None,
                    "rejected",
                    "invalid_control_fields",
                ),
            ));
        }
    };

    let payload_hash = canonical_payload_hash(&validated.payload).map_err(|_| {
        (
            unavailable_err(),
            observe(
                Some(community_uuid),
                Some(app_id),
                None,
                "unavailable",
                "none",
            ),
        )
    })?;

    let begin = state
        .db
        .begin_app_admission(
            community_id,
            app_id,
            &validated.key_hash,
            &payload_hash,
            &validated.event_type,
        )
        .await
        .map_err(|_| {
            (
                unavailable_err(),
                observe(
                    Some(community_uuid),
                    Some(app_id),
                    None,
                    "unavailable",
                    "none",
                ),
            )
        })?;

    match begin {
        BeginAppAdmission::Existing(record) => match record.status {
            AppDeliveryStatus::Delivered => {
                let ok = delivered_json(&record, true);
                Ok((
                    ok.0,
                    ok.1,
                    observe(
                        Some(community_uuid),
                        Some(app_id),
                        Some(record.id),
                        "accepted",
                        "none",
                    ),
                ))
            }
            AppDeliveryStatus::Rejected => {
                let failure = record
                    .failure_code
                    .as_deref()
                    .and_then(RouteFailure::from_code)
                    .ok_or_else(|| {
                        (
                            unavailable_err(),
                            observe(
                                Some(community_uuid),
                                Some(app_id),
                                Some(record.id),
                                "unavailable",
                                "none",
                            ),
                        )
                    })?;
                Err((
                    route_failure_err(failure),
                    observe(
                        Some(community_uuid),
                        Some(app_id),
                        Some(record.id),
                        "rejected",
                        failure.code(),
                    ),
                ))
            }
        },
        BeginAppAdmission::PayloadConflict { existing } => {
            let failure = RouteFailure::IdempotencyConflict;
            Err((
                route_failure_err(failure),
                observe(
                    Some(community_uuid),
                    Some(app_id),
                    Some(existing.id),
                    "conflict",
                    failure.code(),
                ),
            ))
        }
        BeginAppAdmission::Vacant(guard) => {
            admit_vacant(state, tenant, community_id, app_id, validated, guard).await
        }
    }
}

async fn admit_vacant(
    state: Arc<AppState>,
    tenant: TenantContext,
    community_id: CommunityId,
    app_id: Uuid,
    validated: ValidatedCallback,
    mut guard: AppAdmissionGuard,
) -> Result<(StatusCode, Json<Value>, Observation), (Response, Observation)> {
    let community_uuid = *community_id.as_uuid();
    let snapshot = match resolve_app_route(&mut guard, community_id, &validated.target).await {
        Ok(snapshot) => snapshot,
        Err(ResolveRouteError::Deterministic(failure)) => {
            return reject_vacant(community_uuid, app_id, guard, failure).await;
        }
        Err(ResolveRouteError::Transient) => {
            drop(guard);
            return Err((
                unavailable_err(),
                observe(
                    Some(community_uuid),
                    Some(app_id),
                    None,
                    "unavailable",
                    "none",
                ),
            ));
        }
    };

    let members = match guard
        .list_named_destination_members(snapshot.channel_id)
        .await
    {
        Ok(members) => members,
        Err(_) => {
            drop(guard);
            return Err((
                unavailable_err(),
                observe(
                    Some(community_uuid),
                    Some(app_id),
                    None,
                    "unavailable",
                    "none",
                ),
            ));
        }
    };
    let mention_pubkeys =
        resolve_mention_pubkeys(&validated.content, &named_member_pairs(&members));
    let delivery_id = guard.delivery_id();
    let event = match build_app_event(
        &state,
        app_id,
        delivery_id,
        &validated.event_type,
        &validated.content,
        &snapshot,
        &mention_pubkeys,
    ) {
        Ok(event) => event,
        Err(()) => {
            drop(guard);
            return Err((
                unavailable_err(),
                observe(
                    Some(community_uuid),
                    Some(app_id),
                    None,
                    "unavailable",
                    "none",
                ),
            ));
        }
    };

    let commit = match guard.deliver(&event, &snapshot).await {
        Ok(commit) => commit,
        Err(_) => {
            return Err((
                unavailable_err(),
                observe(
                    Some(community_uuid),
                    Some(app_id),
                    None,
                    "unavailable",
                    "none",
                ),
            ));
        }
    };

    let relay_hex = state.relay_keypair.public_key().to_hex();
    dispatch_persistent_event(
        &tenant,
        &state,
        &commit.stored_event,
        KIND_STREAM_MESSAGE,
        &relay_hex,
        None,
    )
    .await;

    let ok = delivered_json(&commit.record, false);
    Ok((
        ok.0,
        ok.1,
        observe(
            Some(community_uuid),
            Some(app_id),
            Some(commit.record.id),
            "accepted",
            "none",
        ),
    ))
}

async fn reject_vacant(
    community_uuid: Uuid,
    app_id: Uuid,
    guard: AppAdmissionGuard,
    failure: RouteFailure,
) -> Result<(StatusCode, Json<Value>, Observation), (Response, Observation)> {
    match guard
        .reject(AppDeliveryFailure {
            code: failure.code(),
        })
        .await
    {
        Ok(record) => Err((
            route_failure_err(failure),
            observe(
                Some(community_uuid),
                Some(app_id),
                Some(record.id),
                "rejected",
                failure.code(),
            ),
        )),
        Err(_) => Err((
            unavailable_err(),
            observe(
                Some(community_uuid),
                Some(app_id),
                None,
                "unavailable",
                "none",
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::net::Ipv4Addr;

    #[tokio::test]
    async fn unavailable_error_has_stable_code() {
        let response = unavailable_err();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), 1024)
            .await
            .expect("read response body");
        assert_eq!(
            serde_json::from_slice::<Value>(&body).expect("parse response body"),
            json!({
                "error": "service unavailable",
                "code": "service_unavailable",
            })
        );
    }

    #[test]
    fn app_callback_rate_limit_key_is_fully_scoped() {
        let community = CommunityId::from_uuid(
            Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
        );
        let app = Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));
        assert_eq!(
            app_callback_rate_limit_key(community, app, ip),
            "buzz:11111111-1111-1111-1111-111111111111:ratelimit:app_callback:aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa:192.0.2.10"
        );
    }

    #[test]
    fn event_type_accepts_lowercase_ascii_contract() {
        assert!(valid_event_type("workflow.run.completed"));
        assert!(valid_event_type("a"));
        assert!(valid_event_type("9push:ok_1"));
        assert!(!valid_event_type(""));
        assert!(!valid_event_type("Workflow.run"));
        assert!(!valid_event_type("-leading"));
        assert!(!valid_event_type(&"a".repeat(65)));
    }

    #[test]
    fn parse_callback_rejects_unknown_and_reserved_keys() {
        for key in [
            "channel_id",
            "project_id",
            "community_id",
            "app_id",
            "pubkey",
            "author",
            "tags",
            "kind",
            "extra",
        ] {
            let mut body = json!({
                "idempotency_key": "k",
                "repository_name": "repo",
                "event_type": "push",
                "content": "hi",
            });
            body[key] = json!("x");
            assert!(
                matches!(parse_callback(body), Err(ParseError::InvalidCallback)),
                "key {key} must be invalid_callback"
            );
        }
    }

    #[test]
    fn parse_callback_prefers_channel_name_over_repository() {
        let both = parse_callback(json!({
            "idempotency_key": "k",
            "channel_name": "3q-chess-dev",
            "repository_name": "repo",
            "event_type": "push",
            "content": "hi",
        }))
        .unwrap();
        assert!(matches!(both.target, CallbackTarget::Channel(ref name) if name == "3q-chess-dev"));

        let channel_only = parse_callback(json!({
            "idempotency_key": "k",
            "channel_name": "3q-chess-dev",
            "event_type": "push",
            "content": "hi",
        }))
        .unwrap();
        assert!(matches!(channel_only.target, CallbackTarget::Channel(_)));

        let repository_only = parse_callback(json!({
            "idempotency_key": "k",
            "repository_name": "repo",
            "event_type": "push",
            "content": "hi",
        }))
        .unwrap();
        assert!(
            matches!(repository_only.target, CallbackTarget::Repository(ref name) if name == "repo")
        );
    }

    #[test]
    fn parse_callback_rejects_invalid_channel_name() {
        for channel_name in [json!(""), json!(7), json!("x".repeat(256))] {
            let body = json!({
                "idempotency_key": "k",
                "channel_name": channel_name,
                "repository_name": "repo",
                "event_type": "push",
                "content": "hi",
            });
            assert!(matches!(
                parse_callback(body),
                Err(ParseError::InvalidControl)
            ));
        }
        let neither = json!({
            "idempotency_key": "k",
            "event_type": "push",
            "content": "hi",
        });
        assert!(matches!(
            parse_callback(neither),
            Err(ParseError::InvalidControl)
        ));
    }

    #[test]
    fn parse_callback_rejects_non_object() {
        assert!(matches!(
            parse_callback(json!([])),
            Err(ParseError::InvalidCallback)
        ));
        assert!(matches!(
            parse_callback(json!("nope")),
            Err(ParseError::InvalidCallback)
        ));
    }

    #[test]
    fn parse_callback_requires_control_fields() {
        let missing = json!({
            "idempotency_key": "k",
            "repository_name": "repo",
            "event_type": "push",
        });
        assert!(matches!(
            parse_callback(missing),
            Err(ParseError::InvalidControl)
        ));
        let empty = json!({
            "idempotency_key": "k",
            "repository_name": "repo",
            "event_type": "push",
            "content": "",
        });
        assert!(matches!(
            parse_callback(empty),
            Err(ParseError::InvalidControl)
        ));
        let bad_type = json!({
            "idempotency_key": "k",
            "repository_name": "repo",
            "event_type": "Push",
            "content": "hi",
        });
        assert!(matches!(
            parse_callback(bad_type),
            Err(ParseError::InvalidControl)
        ));
    }

    #[test]
    fn metadata_must_be_object_within_bounds() {
        let not_object = json!({
            "idempotency_key": "k",
            "repository_name": "repo",
            "event_type": "push",
            "content": "hi",
            "metadata": [],
        });
        assert!(matches!(
            parse_callback(not_object),
            Err(ParseError::InvalidControl)
        ));

        let mut nested = json!({});
        for _ in 0..17 {
            nested = json!({ "c": nested });
        }
        let deep = json!({
            "idempotency_key": "k",
            "repository_name": "repo",
            "event_type": "push",
            "content": "hi",
            "metadata": nested,
        });
        assert!(matches!(
            parse_callback(deep),
            Err(ParseError::InvalidControl)
        ));
    }

    #[test]
    fn metadata_node_limit_counts_members_and_elements() {
        let items: Vec<Value> = (0..1025).map(Value::from).collect();
        let too_many = json!({
            "idempotency_key": "k",
            "repository_name": "repo",
            "event_type": "push",
            "content": "hi",
            "metadata": { "items": items },
        });
        assert!(matches!(
            parse_callback(too_many),
            Err(ParseError::InvalidControl)
        ));
    }

    #[test]
    fn canonical_payload_hash_ignores_object_key_order() {
        let a = parse_callback(json!({
            "content": "hi",
            "event_type": "push",
            "idempotency_key": "k1",
            "repository_name": "repo",
        }))
        .expect("a");
        let b = parse_callback(json!({
            "repository_name": "repo",
            "idempotency_key": "k2",
            "event_type": "push",
            "content": "hi",
        }))
        .expect("b");
        assert_eq!(
            canonical_payload_hash(&a.payload).unwrap(),
            canonical_payload_hash(&b.payload).unwrap()
        );
    }

    #[test]
    fn canonical_payload_keeps_array_order_and_metadata_presence() {
        let with_meta = parse_callback(json!({
            "idempotency_key": "k",
            "repository_name": "repo",
            "event_type": "push",
            "content": "hi",
            "metadata": { "items": [1, 2] },
        }))
        .expect("meta");
        let swapped = parse_callback(json!({
            "idempotency_key": "k",
            "repository_name": "repo",
            "event_type": "push",
            "content": "hi",
            "metadata": { "items": [2, 1] },
        }))
        .expect("swapped");
        let omitted = parse_callback(json!({
            "idempotency_key": "k",
            "repository_name": "repo",
            "event_type": "push",
            "content": "hi",
        }))
        .expect("omitted");
        let empty = parse_callback(json!({
            "idempotency_key": "k",
            "repository_name": "repo",
            "event_type": "push",
            "content": "hi",
            "metadata": {},
        }))
        .expect("empty");
        assert_ne!(
            canonical_payload_hash(&with_meta.payload).unwrap(),
            canonical_payload_hash(&swapped.payload).unwrap()
        );
        assert_ne!(
            canonical_payload_hash(&omitted.payload).unwrap(),
            canonical_payload_hash(&empty.payload).unwrap()
        );
    }

    #[test]
    fn secret_compare_is_length_checked_and_hashed() {
        let secret = [7u8; APP_SECRET_BYTES];
        let hash: [u8; 32] = Sha256::digest(secret).into();
        assert!(secret_matches(&secret, &hash));
        let mut other = secret;
        other[0] ^= 1;
        assert!(!secret_matches(&other, &hash));
        assert!(!secret_matches(&secret, &[0u8; 8]));
    }

    #[test]
    fn content_type_accepts_json_with_charset() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        assert!(content_type_is_json(&headers));
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        assert!(!content_type_is_json(&headers));
        headers.remove(header::CONTENT_TYPE);
        assert!(!content_type_is_json(&headers));
    }
}
