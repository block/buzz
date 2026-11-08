//! Operator-mediated invite minting for a community owner or admin.
//!
//! A trusted operator vouches only for which identity its signed-in caller is
//! bound to. The relay decides that identity's authority from its own
//! membership data at mint time, exactly as `POST /api/invites` does.

use super::*;
use crate::api::invites::{
    map_mint_error, minted_invite_json, validate_mint_request, MintInviteRequest,
};

const PATH: &str = "/operator/communities/invites";

type Error = (StatusCode, Json<Value>);

/// Adds a stable `code` beside the `/api/invites` error text, so the operator
/// can tell its own misconfiguration from the actor's refusal without
/// matching on prose.
fn with_code((status, Json(mut body)): Error, code: &'static str) -> Error {
    body["code"] = code.into();
    (status, Json(body))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    community_id: Uuid,
    actor_pubkey: String,
    #[serde(default)]
    ttl_secs: Option<u64>,
    #[serde(default)]
    max_uses: Option<i32>,
}

/// Mint an invite as the operator-attested `actor_pubkey`, who must be an
/// owner or admin of the active community. The limits, defaults, storage and
/// response match `POST /api/invites`; errors add a stable `code`:
/// `not_operator`, `invalid_request`, `community_not_found`, `not_owner`,
/// `restricted` or `unavailable`.
pub async fn mint_community_invite(
    State(state): State<Arc<AppState>>,
    slot: AccessSlot,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, Error> {
    authorize_operator_request(&state, &slot, &headers, "POST", PATH, None, Some(&body))
        .await
        .map_err(|error| match error.0 {
            StatusCode::FORBIDDEN => with_code(error, "not_operator"),
            _ => error,
        })?;
    let request: Request = serde_json::from_slice(&body).map_err(|e| {
        with_code(
            api_error(
                StatusCode::BAD_REQUEST,
                &format!("invalid community-invite JSON: {e}"),
            ),
            "invalid_request",
        )
    })?;
    let actor = validate_pubkey_hex(&request.actor_pubkey).ok_or_else(|| {
        with_code(
            api_error(
                StatusCode::BAD_REQUEST,
                "invalid actor_pubkey: expected 64-char hex pubkey",
            ),
            "invalid_request",
        )
    })?;
    let (ttl, max_uses) = validate_mint_request(&MintInviteRequest {
        ttl_secs: request.ttl_secs,
        max_uses: request.max_uses,
    })
    .map_err(|error| with_code(error, "invalid_request"))?;
    let community = CommunityId::from_uuid(request.community_id);
    // Archived, deleted and deleting communities have no host here.
    let host = state
        .db
        .lookup_community_host(community)
        .await
        .map_err(|e| internal_error(&format!("community invite host lookup: {e}")))?
        .ok_or_else(|| {
            with_code(
                api_error(StatusCode::NOT_FOUND, "community not found"),
                "community_not_found",
            )
        })?;
    let role = state
        .db
        .get_relay_member(community, &actor)
        .await
        .map_err(|e| internal_error(&format!("community invite role lookup: {e}")))?
        .map(|member| member.role);
    if !matches!(role.as_deref(), Some("owner" | "admin")) {
        return Err(with_code(
            api_error(
                StatusCode::FORBIDDEN,
                "only relay owners and admins can create invites",
            ),
            "not_owner",
        ));
    }
    // The mint re-checks lifecycle and the actor's restriction atomically.
    let invite = state
        .db
        .mint_relay_invite(community, &actor, ttl, max_uses)
        .await
        .map_err(|error| {
            let error = map_mint_error(error);
            match error.0 {
                StatusCode::FORBIDDEN => with_code(error, "restricted"),
                StatusCode::SERVICE_UNAVAILABLE => with_code(error, "unavailable"),
                StatusCode::BAD_REQUEST => with_code(error, "invalid_request"),
                _ => error,
            }
        })?;
    slot.detail(crate::access::Detail {
        invite_id: Some(invite.invite_id),
        expires_at: Some(invite.expires_at.timestamp()),
        max_uses: invite.max_uses,
        ..Default::default()
    });
    Ok(Json(minted_invite_json(&state, &host, &invite)))
}
