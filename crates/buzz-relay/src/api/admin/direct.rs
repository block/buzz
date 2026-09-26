//! Report-less ("direct") staff actions: ban, timeout, and delete taken from
//! the admin console without a report.
//!
//! Order: authorize → parse → tenant → stored-action fast path → target
//! validation → staff guard → atomic accept → drive. A validation failure
//! re-reads the stored action first, so a retry racing its own first attempt
//! (for example a delete whose event the first attempt already removed)
//! replays instead of failing.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, Uri},
    Json,
};
use buzz_db::relay_admin_actions::{AdminActionRecord, DirectActionInput, DirectClaim};
use serde::Deserialize;
use uuid::Uuid;

use super::auth::{
    admin_role_str, authorize, lookup_admin_principal, require_mutation_principal, AdminRole,
};
use super::error::ApiError;
use super::{compute_timeout_until, decode_hex_pubkey, CommunityQuery};
use crate::handlers::report_resolution::{drive_direct_action, ResolutionError};
use crate::state::AppState;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DirectBody {
    request_id: Uuid,
    reason: Option<String>,
    expiration_secs: Option<u64>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Ban,
    Timeout,
    Delete,
}

pub(super) async fn ban(
    State(state): State<Arc<AppState>>,
    uri: Uri,
    headers: HeaderMap,
    Path(target): Path<String>,
    Query(q): Query<CommunityQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    direct_action(state, uri, headers, Kind::Ban, target, q, body).await
}

pub(super) async fn timeout(
    State(state): State<Arc<AppState>>,
    uri: Uri,
    headers: HeaderMap,
    Path(target): Path<String>,
    Query(q): Query<CommunityQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    direct_action(state, uri, headers, Kind::Timeout, target, q, body).await
}

pub(super) async fn delete_event(
    State(state): State<Arc<AppState>>,
    uri: Uri,
    headers: HeaderMap,
    Path(target): Path<String>,
    Query(q): Query<CommunityQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    direct_action(state, uri, headers, Kind::Delete, target, q, body).await
}

/// 409 `target_is_staff` when `target` is on the effective staff roster
/// (config, owner fallback, or DB — which includes the actor). A roster lookup
/// failure propagates as 500: fail closed, never "not staff".
pub(super) async fn refuse_staff_target(state: &AppState, target: &[u8]) -> Result<(), ApiError> {
    let pubkey: [u8; 32] = target.try_into().map_err(|_| ApiError::internal())?;
    match lookup_admin_principal(state, pubkey).await? {
        None => Ok(()),
        Some(_) => Err(ApiError {
            status: StatusCode::CONFLICT,
            code: "target_is_staff",
            message: "target is relay staff; an operator must remove them from staff first".into(),
        }),
    }
}

/// Shared mapping for a failed enforcement drive (report and direct paths):
/// one status, 422 `enforcement_failed`.
pub(super) fn enforcement_failed(action_id: Uuid, error: &str) -> ApiError {
    ApiError::unprocessable(&format!(
        "enforcement failed (action_id={action_id}): {error}"
    ))
}

#[allow(clippy::too_many_arguments)]
async fn direct_action(
    state: Arc<AppState>,
    uri: Uri,
    headers: HeaderMap,
    kind: Kind,
    target_hex: String,
    q: CommunityQuery,
    body_bytes: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let path = uri
        .path_and_query()
        .map_or_else(|| uri.path(), |pq| pq.as_str());
    let principal = require_mutation_principal(
        authorize(&state, &headers, path, "POST", Some(&body_bytes)).await?,
    )?;
    let body: DirectBody = serde_json::from_slice(&body_bytes)
        .map_err(|_| ApiError::bad_request("invalid_body", "invalid JSON body"))?;
    let target = decode_hex_pubkey(&target_hex)?;
    let host = buzz_core::tenant::validate_community_host(&q.community_host)
        .map_err(|msg| ApiError::bad_request("invalid_community_host", msg))?;
    let tenant = match crate::tenant::bind_community(&state.db, &host).await {
        Ok(t) => t,
        Err(crate::tenant::BindError::UnmappedHost) => {
            return Err(ApiError::bad_request(
                "unknown_community_host",
                "no community is served at this host",
            ))
        }
        Err(crate::tenant::BindError::Lookup(_)) => return Err(ApiError::internal()),
    };
    let timeout_secs = match (kind, body.expiration_secs) {
        (Kind::Timeout, Some(secs)) => Some(secs),
        (Kind::Timeout, None) => {
            return Err(ApiError::bad_request(
                "invalid_expiration",
                "expirationSecs is required",
            ))
        }
        (_, None) => None,
        (_, Some(_)) => {
            return Err(ApiError::bad_request(
                "invalid_expiration",
                "expirationSecs is only valid for timeout",
            ))
        }
    };
    let (action, event_target, pubkey_target) = match kind {
        Kind::Ban => ("ban", None, Some(target.as_slice())),
        Kind::Timeout => ("timeout", None, Some(target.as_slice())),
        Kind::Delete => ("delete", Some(target.as_slice()), None),
    };
    let actor_authority = match principal.role {
        AdminRole::Operator => "relay_operator",
        AdminRole::Moderator => "relay_moderator",
    };
    let mut input = DirectActionInput {
        community_id: tenant.community(),
        request_id: body.request_id,
        actor_pubkey: &principal.pubkey,
        actor_role: admin_role_str(principal.role),
        actor_authority,
        action,
        reason: body.reason.as_deref(),
        timeout_secs: timeout_secs.map(|s| s as i64),
        timeout_until: None,
        target_pubkey: pubkey_target,
        target_event_id: event_target,
        channel_id: None,
    };

    // Fast path: an accepted request replays (or conflicts) without re-validating.
    if let Some(claim) = state.db.find_direct_action(&input).await? {
        return drive(&state, &tenant, claim).await;
    }

    #[cfg(test)]
    test_hook::pause(body.request_id).await;

    // New request: validate against current state. On failure, the stored
    // action (if a concurrent attempt accepted it meanwhile) wins.
    let validated: Result<_, ApiError> = async {
        let mut channel = None;
        let mut until = None;
        match kind {
            Kind::Delete => {
                let event = state
                    .db
                    .get_event_by_id(tenant.community(), &target)
                    .await?
                    .ok_or(ApiError {
                        status: StatusCode::NOT_FOUND,
                        code: "event_not_in_community",
                        message: "no such event in this community".into(),
                    })?;
                channel = event.channel_id;
                Ok((Some(event.event.pubkey.to_bytes().to_vec()), channel, until))
            }
            Kind::Timeout | Kind::Ban => {
                if kind == Kind::Timeout {
                    until = Some(compute_timeout_until(timeout_secs.unwrap_or_default())?);
                }
                refuse_staff_target(&state, &target).await?;
                Ok((None, channel, until))
            }
        }
    }
    .await;
    let (author, channel, until) = match validated {
        Ok(v) => v,
        Err(e) => {
            return match state.db.find_direct_action(&input).await? {
                Some(claim) => drive(&state, &tenant, claim).await,
                None => Err(e),
            }
        }
    };
    if author.is_some() {
        input.target_pubkey = author.as_deref();
    }
    input.channel_id = channel;
    input.timeout_until = until;

    let claim = state.db.claim_direct_action(&input).await?;
    drive(&state, &tenant, claim).await
}

async fn drive(
    state: &Arc<AppState>,
    tenant: &buzz_core::tenant::TenantContext,
    claim: DirectClaim,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let (rec, replayed): (AdminActionRecord, bool) = match claim {
        DirectClaim::Claimed(rec) => (rec, false),
        DirectClaim::Existing(rec) => (rec, true),
        DirectClaim::Conflict => {
            return Err(ApiError {
                status: StatusCode::CONFLICT,
                code: "request_id_conflict",
                message: "requestId was already used for a different action".into(),
            })
        }
    };
    let body = |status: &str| {
        Json(serde_json::json!({ "actionId": rec.id, "state": status, "replayed": replayed }))
    };
    match drive_direct_action(state, tenant, &rec, None).await {
        Ok(_) => Ok((StatusCode::OK, body("succeeded"))),
        Err(ResolutionError::EnforcementFailed { action_id, error }) => {
            Err(enforcement_failed(action_id, &error))
        }
        // Lease contention or a transient fault after acceptance: the action is
        // durable and the recovery worker finishes it; the same requestId replays.
        Err(e) => {
            tracing::warn!(action_id = %rec.id, error = ?e, "direct action accepted but not yet converged");
            Ok((StatusCode::ACCEPTED, body("pending")))
        }
    }
}

/// Test-only pause point between the stored-action lookup and validation, so a
/// test can interleave a concurrent attempt deterministically. One-shot per
/// `request_id`: the first request to reach it takes the barrier.
#[cfg(test)]
pub(super) mod test_hook {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::sync::Barrier;
    use uuid::Uuid;

    pub(in super::super) static PAUSE: Mutex<Option<HashMap<Uuid, Arc<Barrier>>>> =
        Mutex::new(None);

    pub(super) async fn pause(request_id: Uuid) {
        let barrier = PAUSE
            .lock()
            .unwrap()
            .as_mut()
            .and_then(|m| m.remove(&request_id));
        if let Some(b) = barrier {
            b.wait().await;
        }
    }
}
