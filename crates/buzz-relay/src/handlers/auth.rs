//! NIP-42 AUTH handler — verify challenge response, transition auth state.
//!
//! Relay membership enforcement uses the shared
//! [`crate::api::relay_members::enforce_relay_membership`] helper, which supports
//! NIP-OA owner-delegation fallback on closed relays. On open relays, the auth
//! handler calls [`crate::api::relay_members::extract_nip_oa_owner`] directly to
//! extract the owner pubkey for agent→owner backfill (observer frame auth).
//!
//! For WebSocket auth, the NIP-OA `auth` tag is extracted from the signed AUTH
//! event itself (the tag is integrity-protected by the event signature).

use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::connection::{AuthState, ConnectionState};
use crate::protocol::RelayMessage;
use crate::state::AppState;

/// Extract a NIP-OA `auth` tag from a verified AUTH event and serialize it as
/// the JSON-array string that [`buzz_sdk::nip_oa::verify_auth_tag`] expects.
///
/// Returns `None` if no `auth` tag is present (direct-member auth path) or if
/// more than one `auth` tag exists (per NIP-OA spec: >1 auth tag ⇒ no valid tag).
pub fn extract_auth_tag_json(event: &nostr::Event) -> Option<String> {
    let mut iter = event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("auth"));
    let first = iter.next()?;
    if iter.next().is_some() {
        return None; // NIP-OA spec: treat >1 auth tag as no valid auth tag
    }
    serde_json::to_string(first.as_slice()).ok()
}

/// Handle a NIP-42 AUTH message: verify the challenge response and transition
/// the connection to authenticated state.
///
/// Pure crypto verification — no API tokens, no JWT, no DB token lookups.
pub async fn handle_auth(event: nostr::Event, conn: Arc<ConnectionState>, state: Arc<AppState>) {
    let event_id_hex = event.id.to_hex();
    let (challenge, conn_id) = {
        let auth = conn.auth_state.read().await;
        match &*auth {
            AuthState::Pending { challenge } => (challenge.clone(), conn.conn_id),
            AuthState::Authenticated(_) => {
                debug!(conn_id = %conn.conn_id, "AUTH received but already authenticated");
                conn.send(RelayMessage::ok(
                    &event_id_hex,
                    false,
                    "auth-required: already authenticated",
                ));
                return;
            }
            AuthState::Failed => {
                debug!(conn_id = %conn.conn_id, "AUTH received after failed auth");
                conn.send(RelayMessage::ok(
                    &event_id_hex,
                    false,
                    "auth-required: authentication already failed",
                ));
                return;
            }
        }
    };

    // Extract the NIP-OA auth tag before verification consumes the event.
    // The tag is integrity-protected by the event's Schnorr signature — if
    // tampered, NIP-42 verification will fail before we ever inspect it.
    let auth_tag_json = extract_auth_tag_json(&event);

    let relay_url = state.config.relay_url.clone();
    let auth_svc = Arc::clone(&state.auth);

    metrics::counter!("buzz_auth_attempts_total", "method" => "nip42").increment(1);

    // Pure NIP-42 verification — crypto only, no DB lookups.
    match auth_svc
        .verify_auth_event(event, &challenge, &relay_url)
        .await
    {
        Ok(mut auth_ctx) => {
            let pubkey = auth_ctx.pubkey;

            // Pubkey allowlist gate — only for pubkey-only auth.
            if state.config.pubkey_allowlist_enabled
                && auth_ctx.auth_method == buzz_auth::AuthMethod::Nip42
            {
                let allowed = match state.db.is_pubkey_allowed(pubkey.as_bytes()).await {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = %e,
                              "allowlist DB lookup failed, denying (fail-closed)");
                        false
                    }
                };
                if !allowed {
                    warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), "pubkey not in allowlist");
                    metrics::counter!("buzz_auth_failures_total", "reason" => "allowlist_denied")
                        .increment(1);
                    *conn.auth_state.write().await = AuthState::Failed;
                    conn.send(RelayMessage::ok(
                        &event_id_hex,
                        false,
                        "auth-required: verification failed",
                    ));
                    return;
                }
            }

            // Relay membership gate — uses the shared helper with NIP-OA fallback.
            let nip_oa_owner = match crate::api::relay_members::enforce_relay_membership(
                &state,
                pubkey.as_bytes(),
                auth_tag_json.as_deref(),
            )
            .await
            {
                Ok(owner) => owner,
                Err(e) => {
                    warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = ?e, "not a relay member");
                    metrics::counter!("buzz_auth_failures_total", "reason" => "not_relay_member")
                        .increment(1);
                    *conn.auth_state.write().await = AuthState::Failed;
                    conn.send(RelayMessage::ok(
                        &event_id_hex,
                        false,
                        "restricted: not a relay member",
                    ));
                    return;
                }
            };

            // Open relay NIP-OA backfill: extract owner for agent→owner DB mapping
            // (needed for observer frame auth). Only runs on open relays — on closed
            // relays, enforce_relay_membership already handles NIP-OA delegation.
            // No feature flag needed: NIP-OA is cryptographically self-proving.
            let nip_oa_owner = nip_oa_owner.or_else(|| {
                if !state.config.require_relay_membership && auth_tag_json.is_some() {
                    crate::api::relay_members::extract_nip_oa_owner(
                        pubkey.as_bytes(),
                        auth_tag_json.as_deref(),
                    )
                } else {
                    None
                }
            });

            // Stash NIP-OA owner on the auth context (session-scoped) only if
            // the DB confirms this owner relationship (first-write-wins).
            if let Some(owner) = nip_oa_owner {
                // Ensure both agent and owner have users rows (BYO agents may not,
                // and agent_owner_pubkey has a FK constraint to users.pubkey).
                if let Err(e) = state.db.ensure_user(pubkey.as_bytes()).await {
                    warn!(conn_id = %conn_id, error = %e, "ensure_user(agent) failed during NIP-OA backfill");
                }
                if let Err(e) = state.db.ensure_user(owner.as_bytes()).await {
                    warn!(conn_id = %conn_id, error = %e, "ensure_user(owner) failed during NIP-OA backfill");
                }

                // Idempotent backfill: record agent→owner in DB so cross-connection
                // features (observer frames, channel policy) work for BYO agents.
                // Returns Ok(true) if written, Ok(false) if already owned by someone else.
                match state
                    .db
                    .set_agent_owner(pubkey.as_bytes(), owner.as_bytes())
                    .await
                {
                    Ok(true) => {
                        // Successfully materialized — this owner is authoritative.
                        auth_ctx.agent_owner_pubkey = Some(owner);
                        // Pre-warm the observer cache to avoid stale negatives.
                        let cache_key = (pubkey.to_bytes().to_vec(), owner.to_bytes().to_vec());
                        state.observer_owner_cache.insert(cache_key, true);
                    }
                    Ok(false) => {
                        // Agent already owned by someone else. Verify if this
                        // owner matches the existing DB record before trusting it.
                        match state
                            .db
                            .is_agent_owner(pubkey.as_bytes(), owner.as_bytes())
                            .await
                        {
                            Ok(true) => {
                                auth_ctx.agent_owner_pubkey = Some(owner);
                            }
                            Ok(false) => {
                                warn!(
                                    conn_id = %conn_id,
                                    agent = %pubkey.to_hex(),
                                    nip_oa_owner = %owner.to_hex(),
                                    "NIP-OA owner differs from DB owner — session will not get owner fast-path"
                                );
                            }
                            Err(e) => {
                                warn!(
                                    conn_id = %conn_id,
                                    error = %e,
                                    "is_agent_owner check failed after set_agent_owner conflict"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            conn_id = %conn_id,
                            agent = %pubkey.to_hex(),
                            owner = %owner.to_hex(),
                            error = %e,
                            "failed to backfill agent_owner_pubkey"
                        );
                    }
                }
            }

            info!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), "NIP-42 auth successful");
            *conn.auth_state.write().await = AuthState::Authenticated(auth_ctx);
            state
                .conn_manager
                .set_authenticated_pubkey(conn_id, pubkey.to_bytes().to_vec());
            conn.send(RelayMessage::ok(&event_id_hex, true, ""));
        }
        Err(e) => {
            warn!(conn_id = %conn_id, error = %e, "NIP-42 auth failed");
            metrics::counter!("buzz_auth_failures_total", "reason" => "nip42_invalid").increment(1);
            *conn.auth_state.write().await = AuthState::Failed;
            conn.send(RelayMessage::ok(
                &event_id_hex,
                false,
                "auth-required: verification failed",
            ));
        }
    }
}
