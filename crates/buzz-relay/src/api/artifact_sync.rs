//! Incremental reads with signed progress and bounded current-identity reads.
use crate::state::AppState;
use axum::{http::StatusCode, Json};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use buzz_core::{
    artifact::{ArtifactQuery, ArtifactView},
    TenantContext,
};
use buzz_db::artifact_sync::{Candidate, Position, PAGE_ROWS};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use uuid::Uuid;

const OVERLAP: i64 = 120;
const CHECKPOINT_DAYS: i64 = 30;
const RESPONSE_BYTES: usize = 8_388_608;
static WORK: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
type Response = Result<Json<Value>, (StatusCode, Json<Value>)>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    artifact: String,
    query: Value,
    action: Option<String>,
    checkpoint: Option<String>,
    cursor: Option<String>,
    artifact_ids: Option<Vec<String>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Token {
    version: u8,
    community: Uuid,
    reader: String,
    query: String,
    at: DateTime<Utc>,
    policy: DateTime<Utc>,
    since: Option<DateTime<Utc>>,
    position: Option<Position>,
    cursor: bool,
}

fn error(status: StatusCode, code: &str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"error":code,"code":code})))
}
fn invalid() -> (StatusCode, Json<Value>) {
    error(StatusCode::BAD_REQUEST, "incremental_invalid")
}
fn unavailable() -> (StatusCode, Json<Value>) {
    error(StatusCode::SERVICE_UNAVAILABLE, "incremental_unavailable")
}
fn limit() -> (StatusCode, Json<Value>) {
    error(StatusCode::PAYLOAD_TOO_LARGE, "incremental_limit")
}
fn reset(reason: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::CONFLICT,
        Json(json!({"error":"incremental_reset","code":"incremental_reset","reason":reason})),
    )
}
fn db_error(e: buzz_db::DbError) -> (StatusCode, Json<Value>) {
    match e {
        buzz_db::DbError::ArtifactSyncLimit => limit(),
        buzz_db::DbError::ArtifactSyncUnsupportedScope => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "incremental_unsupported_scope",
        ),
        error => {
            tracing::warn!(%error,"incremental artifact read failed");
            unavailable()
        }
    }
}

/// Bounds advertised to clients that use the incremental protocol.
pub(crate) fn capability() -> Value {
    json!({"version":1,"overlapSeconds":OVERLAP,"pageRows":PAGE_ROWS,
        "currentRows":PAGE_ROWS,"responseBytes":RESPONSE_BYTES,"permanentChannelsOnly":true,
        "checkpointLifetimeSeconds":CHECKPOINT_DAYS*86400})
}

fn mac(state: &AppState) -> Result<Hmac<Sha256>, (StatusCode, Json<Value>)> {
    let mut digest = Sha256::new();
    digest.update(b"buzz-artifact-incremental-v1\0");
    digest.update(state.relay_keypair.secret_key().as_secret_bytes());
    Hmac::<Sha256>::new_from_slice(&digest.finalize()).map_err(|_| unavailable())
}
fn encode(state: &AppState, token: &Token) -> Result<String, (StatusCode, Json<Value>)> {
    let data = serde_json::to_vec(token).map_err(|_| unavailable())?;
    let mut signature = mac(state)?;
    signature.update(&data);
    let encoded = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&data),
        URL_SAFE_NO_PAD.encode(signature.finalize().into_bytes())
    );
    if encoded.len() > 200_000 {
        return Err(invalid());
    }
    Ok(encoded)
}
fn decode(state: &AppState, value: &str) -> Result<Token, (StatusCode, Json<Value>)> {
    if value.len() > 200_000 {
        return Err(invalid());
    }
    let (data, signature) = value.split_once('.').ok_or_else(invalid)?;
    let data = URL_SAFE_NO_PAD.decode(data).map_err(|_| invalid())?;
    let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| invalid())?;
    let mut check = mac(state)?;
    check.update(&data);
    check.verify_slice(&signature).map_err(|_| invalid())?;
    serde_json::from_slice(&data).map_err(|_| invalid())
}

/// Execute one incremental read without a manifest or server-side cursor state.
#[tracing::instrument(skip_all, fields(community_id = %tenant.community(), reader = %reader))]
pub(super) async fn query(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &nostr::PublicKey,
    filter: &Value,
) -> Response {
    if !state.config.artifact_incremental_enabled {
        return Err(unavailable());
    }
    let _permit = WORK.try_acquire().map_err(|_| unavailable())?;
    tokio::time::timeout(
        std::time::Duration::from_secs(8),
        execute(state, tenant, reader, filter),
    )
    .await
    .map_err(|_| unavailable())?
}

async fn execute(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &nostr::PublicKey,
    filter: &Value,
) -> Response {
    let request: Request = serde_json::from_value(filter.clone()).map_err(|_| invalid())?;
    if request.artifact != "incremental"
        || request.query.get("limit").is_some()
        || request.query.get("offset").is_some()
    {
        return Err(invalid());
    }
    let parsed = buzz_core::artifact::parse_query(&request.query).map_err(|_| invalid())?;
    let query_binding = URL_SAFE_NO_PAD.encode(Sha256::digest(
        serde_json::to_vec(&request.query).map_err(|_| invalid())?,
    ));
    if parsed.view != ArtifactView::Current {
        return Err(invalid());
    }
    let (now, policy) = state
        .db
        .artifact_sync_boundary(tenant.community())
        .await
        .map_err(db_error)?;
    if request.action.as_deref() == Some("current") {
        if request.checkpoint.is_some() || request.cursor.is_some() {
            return Err(invalid());
        }
        let ids = request.artifact_ids.ok_or_else(invalid)?;
        if ids.is_empty()
            || ids.len() > PAGE_ROWS
            || ids.iter().any(|id| id.is_empty() || id.len() > 256)
            || ids.iter().collect::<HashSet<_>>().len() != ids.len()
        {
            return Err(invalid());
        }
        let valid = ids
            .iter()
            .filter_map(|id| buzz_core::artifact::canonical_uuid(id).ok())
            .collect::<Vec<_>>();
        let resolved = current(state, tenant, reader, &parsed, &valid).await?;
        let resolved = resolved
            .into_iter()
            .filter_map(|value| {
                value
                    .get("artifactId")
                    .and_then(Value::as_str)
                    .map(|id| (id.to_owned(), value.clone()))
            })
            .collect::<HashMap<_, _>>();
        let changes = ids
            .iter()
            .map(|id| {
                resolved
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| json!({"type":"remove","artifactId":id}))
            })
            .collect::<Vec<_>>();
        let response = json!({"version":1,"startedAt":now,"changes":changes});
        if serde_json::to_vec(&response)
            .map_err(|_| unavailable())?
            .len()
            > RESPONSE_BYTES
        {
            return Err(limit());
        }
        return Ok(Json(response));
    }
    if request.action.is_some()
        || request.artifact_ids.is_some()
        || (request.checkpoint.is_some() && request.cursor.is_some())
    {
        return Err(invalid());
    }
    let encoded = request.cursor.as_ref().or(request.checkpoint.as_ref());
    let mut token = if let Some(encoded) = encoded {
        let previous = decode(state, encoded)?;
        if previous.version != 1
            || previous.community != *tenant.community().as_uuid()
            || previous.reader != reader.to_hex()
            || previous.query != query_binding
            || previous.cursor != request.cursor.is_some()
            || previous.at > now
        {
            return Err(invalid());
        }
        if previous.at < now - Duration::days(CHECKPOINT_DAYS) {
            return Err(reset("checkpoint_expired"));
        }
        if previous.policy != policy {
            return Err(reset("policy_changed"));
        }
        if previous.cursor {
            previous
        } else {
            Token {
                since: Some(previous.at - Duration::seconds(OVERLAP)),
                at: now,
                position: None,
                cursor: true,
                ..previous
            }
        }
    } else {
        Token {
            version: 1,
            community: *tenant.community().as_uuid(),
            reader: reader.to_hex(),
            query: query_binding,
            at: now,
            policy,
            since: None,
            position: None,
            cursor: true,
        }
    };
    let candidates = state
        .db
        .artifact_sync_candidates(
            tenant.community(),
            reader.as_bytes(),
            token.since,
            token.at,
            token.position.as_ref(),
            &parsed,
        )
        .await
        .map_err(db_error)?;
    let mut selected = candidates.len().min(PAGE_ROWS);
    let resolved = loop {
        let ids = candidates[..selected]
            .iter()
            .filter_map(|c| c.artifact_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        match current(state, tenant, reader, &parsed, &ids).await {
            Err((StatusCode::PAYLOAD_TOO_LARGE, _)) if selected > 1 => selected /= 2,
            result => break result?,
        }
    };
    let more = candidates.len() > selected;
    let candidates = &candidates[..selected];
    let resolved = resolved
        .into_iter()
        .filter_map(|value| {
            value
                .get("artifactId")
                .and_then(Value::as_str)
                .map(|id| (id.to_owned(), value.clone()))
        })
        .collect::<HashMap<_, _>>();
    let mut changes = Vec::new();
    let mut size = 0;
    let mut processed = 0;
    let mut emitted = HashSet::new();
    for candidate in candidates {
        let change = change(candidate, &resolved)?;
        let key = change.to_string();
        if !emitted.insert(key) {
            token.position = Some(candidate.position.clone());
            processed += 1;
            continue;
        }
        let bytes = serde_json::to_vec(&change)
            .map_err(|_| unavailable())?
            .len();
        // Leave room for the signed continuation and response fields.
        if size + bytes > RESPONSE_BYTES - 262_144 {
            if processed == 0 {
                return Err(limit());
            }
            break;
        }
        size += bytes;
        changes.push(change);
        token.position = Some(candidate.position.clone());
        processed += 1;
    }
    let (_, final_policy) = state
        .db
        .artifact_sync_boundary(tenant.community())
        .await
        .map_err(db_error)?;
    if final_policy != token.policy {
        return Err(reset("policy_changed"));
    }
    let more = more || processed < candidates.len();
    let (cursor, checkpoint) = if more {
        (Some(encode(state, &token)?), None)
    } else {
        token.cursor = false;
        token.position = None;
        token.since = None;
        (None, Some(encode(state, &token)?))
    };
    Ok(Json(
        json!({"version":1,"startedAt":token.at,"overlapSeconds":OVERLAP,
        "changes":changes,"nextCursor":cursor,"checkpoint":checkpoint}),
    ))
}

async fn current(
    state: &AppState,
    tenant: &TenantContext,
    reader: &nostr::PublicKey,
    query: &ArtifactQuery,
    ids: &[Uuid],
) -> Result<Vec<Value>, (StatusCode, Json<Value>)> {
    let events = state
        .db
        .artifact_sync_current(tenant.community(), reader.as_bytes(), query, ids)
        .await
        .map_err(db_error)?;
    let mut by_id = HashMap::new();
    for stored in events {
        let env = buzz_core::artifact::validate(&stored.event).map_err(|_| unavailable())?;
        by_id.insert(env.id, stored.event);
    }
    Ok(ids
        .iter()
        .map(|id| match by_id.remove(id) {
            Some(event) => json!({"type":"upsert","artifactId":id,"event":event}),
            None => json!({"type":"remove","artifactId":id}),
        })
        .collect())
}

fn change(
    candidate: &Candidate,
    resolved: &HashMap<String, Value>,
) -> Result<Value, (StatusCode, Json<Value>)> {
    if let Some(channel) = candidate.removed_channel {
        return Ok(json!({"type":"removeChannel","channelId":channel}));
    }
    let id = candidate.artifact_id.ok_or_else(unavailable)?;
    resolved
        .get(&id.to_string())
        .cloned()
        .ok_or_else(unavailable)
}

#[cfg(test)]
pub(super) fn expired_checkpoint_for_test(
    state: &AppState,
    value: &str,
) -> Result<String, (StatusCode, Json<Value>)> {
    let mut token = decode(state, value)?;
    token.at -= Duration::days(31);
    encode(state, &token)
}
