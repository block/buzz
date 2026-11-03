use super::{api_error, bridge, internal_error, AppState};
use axum::{
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde_json::Value;
use std::sync::Arc;

/// Verify tenant-scoped NIP-98 and optional NIP-OA provenance without user writes.
pub(super) async fn authenticate(
    state: &Arc<AppState>,
    slot: &crate::access::AccessSlot,
    headers: &HeaderMap,
    path: &'static str,
    body: &[u8],
) -> Result<
    (
        buzz_core::TenantContext,
        nostr::PublicKey,
        Option<nostr::PublicKey>,
    ),
    (StatusCode, Json<Value>),
> {
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&**state, raw_host)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
        })?;
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, path);
    let bridge::VerifiedBridgeAuth {
        pubkey,
        event_id_bytes,
        signed_created_at,
    } = bridge::verify_nip98_exempt_invite_claim(slot, headers, "POST", &url, Some(body))?;
    // Claiming an invite is how a non-member joins: replay is the last
    // admission step, so it decides the request.
    slot.attempt(crate::access::AuthMethod::Nip98, Some(pubkey), true);
    slot.in_community(tenant.community());
    slot.gate(
        bridge::check_nip98_replay(state, &tenant, event_id_bytes).await,
        bridge::ReplayRejection::auth_result,
    )?;
    slot.finish(crate::access::AuthResult::Success);
    let owner = super::super::relay_members::extract_nip_oa_owner(
        pubkey.as_bytes(),
        super::super::relay_members::extract_auth_tag_header(headers),
        signed_created_at,
    );
    Ok((tenant, pubkey, owner))
}

/// The claim write's refusal, recording an active ban or timeout on `slot`.
pub(super) fn map_claim_error(
    slot: &crate::access::AccessSlot,
    error: buzz_db::DbError,
) -> (StatusCode, Json<Value>) {
    match error {
        buzz_db::DbError::InviteRestricted => {
            slot.finish(crate::access::AuthResult::Denied(
                crate::access::AuthReason::Restricted,
            ));
            api_error(StatusCode::FORBIDDEN, "invite_restricted")
        }
        buzz_db::DbError::InviteOwnerMismatch => {
            api_error(StatusCode::FORBIDDEN, "invite_owner_mismatch")
        }
        e => internal_error(&format!("invite claim: {e}")),
    }
}
