//! Restriction list and lifts for an explicit community.
//!
//! The caller names the community (`communityHost`, validated natively) and
//! the relay and signer its list loaded under. A lift is refused before any
//! request (`notSent`) when either changed, and every attempt — including the
//! 401 retry — is signed with the one key snapshot that was checked.

use super::error::AdminMutationError;
use super::{helpers, origin, routes, SUCCESS_JSON_CAP};

const RELAY_SCOPE_CHANGED: &str =
    "active community changed since restrictions loaded; nothing was sent. Reload to continue.";
const SIGNER_CHANGED: &str =
    "active identity changed since restrictions loaded; nothing was sent. Reload to continue.";

/// Build a restrictions-route URL for `community_host`, refusing when the
/// active relay no longer matches `expected_relay`.
fn restrictions_url(
    origin: &str,
    route: &routes::AdminRoute,
    community_host: &str,
    cursor: Option<String>,
    expected_relay: &str,
    relay_base: &str,
) -> Result<String, String> {
    let origin = origin::AdminOrigin::parse(origin)?;
    if expected_relay.trim().is_empty()
        || crate::relay::assert_expected_relay_scope(Some(expected_relay), relay_base).is_err()
    {
        return Err(RELAY_SCOPE_CHANGED.to_string());
    }
    let host = buzz_core_pkg::tenant::validate_community_host(community_host.trim())
        .map_err(|e| format!("invalid community host: {e}"))?;
    let q = routes::AdminQuery {
        community_host: Some(host),
        cursor,
        ..Default::default()
    };
    Ok(origin.route_url(route, &q))
}

/// List active bans and timeouts —
/// GET /api/admin/v1/members/restrictions?communityHost={host}[&cursor={token}].
///
/// Returns `{ items: [...], nextCursor: string|null }` (relay page size 200).
#[tauri::command]
pub async fn admin_list_restrictions(
    origin: String,
    community_host: String,
    cursor: Option<String>,
    expected_relay: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    let relay_base = crate::relay::relay_api_base_url_with_override(&state);
    let url = restrictions_url(
        &origin,
        &routes::AdminRoute::MemberRestrictionsList,
        &community_host,
        cursor,
        &expected_relay,
        &relay_base,
    )?;
    let bytes = helpers::fetch_admin_json(&url, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}"))
}

/// A lift as the restriction row froze it.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminLiftIntent {
    pub origin: String,
    pub community_host: String,
    pub expected_relay: String,
    pub expected_signer: String,
    pub kind: LiftKind,
    pub pubkey: String,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LiftKind {
    Ban,
    Timeout,
}

/// Validate `intent` against the relay and signer snapshot; every refusal
/// here happens before any request.
pub(super) fn lift_url(
    intent: &AdminLiftIntent,
    relay_base: &str,
    signer_hex: &str,
) -> Result<String, String> {
    if intent.expected_signer.trim().is_empty()
        || crate::relay::assert_expected_signer(Some(&intent.expected_signer), signer_hex).is_err()
    {
        return Err(SIGNER_CHANGED.to_string());
    }
    let pubkey =
        routes::Hex64::parse(&intent.pubkey).map_err(|e| format!("invalid member pubkey: {e}"))?;
    let route = match intent.kind {
        LiftKind::Ban => routes::AdminRoute::MemberBanDelete { pubkey },
        LiftKind::Timeout => routes::AdminRoute::MemberTimeoutDelete { pubkey },
    };
    restrictions_url(
        &intent.origin,
        &route,
        &intent.community_host,
        None,
        &intent.expected_relay,
        relay_base,
    )
}

/// Lift an active ban or timeout —
/// DELETE /api/admin/v1/members/{pubkey}/{ban|timeout}?communityHost={host}.
///
/// 204 on success; 409 when nothing is active, surfaced as `AdminMutationError`.
#[tauri::command]
pub async fn admin_lift_restriction(
    intent: AdminLiftIntent,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<(), AdminMutationError> {
    let keys = state.signing_keys().map_err(AdminMutationError::not_sent)?;
    let relay_base = crate::relay::relay_api_base_url_with_override(&state);
    send_lift(&intent, &keys, &relay_base).await
}

/// Send a validated lift, signing the first attempt and any retry with `keys`.
pub(super) async fn send_lift(
    intent: &AdminLiftIntent,
    keys: &nostr::Keys,
    relay_base: &str,
) -> Result<(), AdminMutationError> {
    let url = lift_url(intent, relay_base, &keys.public_key().to_hex())
        .map_err(AdminMutationError::not_sent)?;
    helpers::send_admin_mutation(keys, reqwest::Method::DELETE, &url, None, SUCCESS_JSON_CAP)
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "restrictions_tests.rs"]
mod tests;
