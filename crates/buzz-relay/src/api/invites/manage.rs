//! `GET /api/invites` and `POST /api/invites/revoke`.
//!
//! Neither response nor access line carries an invite code or its hash, and
//! request errors are fixed strings that never echo input.

use std::sync::Arc;

use axum::{
    extract::{RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse as _, Json, Response},
};
use buzz_core::invite::{hash_v2_code, validate_v2_code};
use buzz_db::relay_invite::{InviteSelector, InviteSummary};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use super::{admit_invite_caller, api_error, internal_error, InviteGate};
use crate::access::{AccessSlot, AuthReason, AuthResult};
use crate::state::AppState;

const DEFAULT_LIST_LIMIT: i64 = 50;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    limit: Option<i64>,
    before: Option<String>,
}

/// The list cursor: the last row's `created_at` in Unix microseconds (the
/// column's precision) and its id, so a page never depends on that row
/// still existing.
fn encode_cursor(invite: &InviteSummary) -> String {
    format!("{}_{}", invite.created_at.timestamp_micros(), invite.id)
}

fn decode_cursor(cursor: &str) -> Option<(chrono::DateTime<chrono::Utc>, Uuid)> {
    let (micros, id) = cursor.split_once('_')?;
    Some((
        chrono::DateTime::from_timestamp_micros(micros.parse().ok()?)?,
        id.parse().ok()?,
    ))
}

/// Body for `POST /api/invites/revoke`: exactly one of `id` or `code`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeInviteRequest {
    id: Option<Uuid>,
    code: Option<String>,
}

/// List invites — `GET /api/invites?limit=&before=`, NIP-98 signed by an
/// owner/admin over the full URL, query included. Newest first; when a page
/// is full, `next_before` is the opaque cursor to pass as `before`.
pub async fn list_invites(
    State(state): State<Arc<AppState>>,
    slot: AccessSlot,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let path = match &query {
        Some(query) => format!("/api/invites?{query}"),
        None => "/api/invites".to_owned(),
    };
    let gate = InviteGate::Admin("only relay owners and admins can list invites");
    let body = axum::body::Bytes::new();
    let (tenant, _) =
        match admit_invite_caller(&state, &slot, &headers, "GET", &path, &body, gate).await {
            Ok(admitted) => admitted,
            Err(response) => return response,
        };

    let query: ListQuery = match serde_urlencoded::from_str(query.as_deref().unwrap_or("")) {
        Ok(query) => query,
        Err(_) => return api_error(StatusCode::BAD_REQUEST, "invalid query").into_response(),
    };
    let before = match query.before.as_deref().map(decode_cursor) {
        None => None,
        Some(Some(cursor)) => Some(cursor),
        Some(None) => return api_error(StatusCode::BAD_REQUEST, "invalid before").into_response(),
    };
    let limit = query.limit.unwrap_or(DEFAULT_LIST_LIMIT);
    if !(1..=buzz_db::relay_invite::MAX_INVITE_LIST_LIMIT).contains(&limit) {
        return api_error(
            StatusCode::BAD_REQUEST,
            &format!(
                "limit must be between 1 and {}",
                buzz_db::relay_invite::MAX_INVITE_LIST_LIMIT
            ),
        )
        .into_response();
    }

    let invites = match state
        .db
        .list_relay_invites(tenant.community(), limit, before)
        .await
    {
        Ok(invites) => invites,
        Err(e) => return internal_error(&format!("invite list: {e}")).into_response(),
    };
    let next_before = (invites.len() as i64 == limit)
        .then(|| invites.last().map(encode_cursor))
        .flatten();
    let now = chrono::Utc::now();
    Json(serde_json::json!({
        "invites": invites.iter().map(|invite| invite_json(invite, now)).collect::<Vec<_>>(),
        "next_before": next_before,
    }))
    .into_response()
}

fn invite_json(invite: &InviteSummary, now: chrono::DateTime<chrono::Utc>) -> Value {
    serde_json::json!({
        "id": invite.id,
        "created_by": invite.created_by,
        "created_at": invite.created_at.timestamp(),
        "expires_at": invite.expires_at.map(|at| at.timestamp()),
        "max_uses": invite.max_uses,
        "use_count": invite.use_count,
        "revoked_at": invite.revoked_at.map(|at| at.timestamp()),
        "status": invite.status(now).as_str(),
    })
}

/// Revoke an invite — `POST /api/invites/revoke` with `{"id": ...}` or
/// `{"code": "v2...."}`, NIP-98 signed with a payload tag. Owners and admins
/// can revoke any invite in the community; any other member only one they
/// created. Repeating a revoke returns the first `revoked_at`.
pub async fn revoke_invite(
    State(state): State<Arc<AppState>>,
    slot: AccessSlot,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let (tenant, pubkey) = match admit_invite_caller(
        &state,
        &slot,
        &headers,
        "POST",
        "/api/invites/revoke",
        &body,
        InviteGate::Member,
    )
    .await
    {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };

    let request: RevokeInviteRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => {
            return api_error(StatusCode::BAD_REQUEST, "invalid revoke request").into_response()
        }
    };
    let token_hash;
    let selector = match (request.id, request.code.as_deref()) {
        (Some(id), None) => InviteSelector::Id(id),
        (None, Some(code)) => {
            // v1 codes have no row to revoke.
            if validate_v2_code(code).is_err() {
                return api_error(StatusCode::BAD_REQUEST, "invite_not_revocable").into_response();
            }
            token_hash = hash_v2_code(code);
            InviteSelector::TokenHash(&token_hash)
        }
        _ => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "exactly one of id or code is required",
            )
            .into_response()
        }
    };

    let revoked = state
        .db
        .revoke_relay_invite(tenant.community(), &pubkey.to_hex(), selector)
        .await;
    match revoked {
        Ok(Some(revoked)) => {
            slot.detail(crate::access::Detail {
                invite_id: Some(revoked.id),
                ..Default::default()
            });
            Json(serde_json::json!({
                "id": revoked.id,
                "revoked_at": revoked.revoked_at.timestamp(),
            }))
            .into_response()
        }
        Ok(None) => api_error(StatusCode::NOT_FOUND, "invite_not_found").into_response(),
        // The database made the final role decision; log its refusal.
        Err(buzz_db::DbError::InviteRevokeForbidden) => {
            slot.finish(AuthResult::Denied(AuthReason::NotRelayAdmin));
            api_error(
                StatusCode::FORBIDDEN,
                "only relay owners, admins or the invite's creator can revoke it",
            )
            .into_response()
        }
        Err(buzz_db::DbError::InviteRestricted) => {
            slot.finish(AuthResult::Denied(AuthReason::Restricted));
            api_error(StatusCode::FORBIDDEN, "invite_restricted").into_response()
        }
        Err(buzz_db::DbError::AccessDenied(_)) => api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "community writes are temporarily unavailable",
        )
        .into_response(),
        Err(e) => internal_error(&format!("invite revoke: {e}")).into_response(),
    }
}
