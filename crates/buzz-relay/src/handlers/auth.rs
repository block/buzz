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

use axum::extract::ws::Message as WsMessage;
use buzz_auth::{
    AuthTransport, VerifiedDelegationOutput, VerifiedEvidenceAdapter, VerifiedNostrProof,
};
use buzz_core::client_binding_bootstrap::{
    ClientBindingBootstrapInputV1, ClientBindingScopeV1, CLIENT_BINDING_BOOTSTRAP_SUB_ID,
};
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
#[tracing::instrument(skip_all, fields(event_id, conn_id))]
pub async fn handle_auth(event: nostr::Event, conn: Arc<ConnectionState>, state: Arc<AppState>) {
    let event_id_hex = event.id.to_hex();
    let verified_event = event.clone();
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

    // Record the declared span fields now that we have the values.
    tracing::Span::current()
        .record("event_id", event_id_hex.as_str())
        .record("conn_id", conn_id.to_string().as_str());

    // Extract the NIP-OA auth tag before verification consumes the event.
    // The tag is integrity-protected by the event's Schnorr signature — if
    // tampered, NIP-42 verification will fail before we ever inspect it.
    let auth_tag_json = extract_auth_tag_json(&event);

    let relay_url =
        crate::api::bridge::nip42_expected_relay_url(&state.config.relay_url, &conn.tenant);
    let auth_svc = Arc::clone(&state.auth);

    metrics::counter!("buzz_auth_attempts_total", "method" => "nip42").increment(1);

    // Pure NIP-42 verification — crypto only, no DB lookups.
    match auth_svc
        .verify_auth_event(event, &challenge, &relay_url)
        .await
    {
        Ok(mut auth_ctx) => {
            let pubkey = auth_ctx.pubkey;

            if state
                .protected_transport()
                .and_then(|runtime| runtime.mode_for_domain(conn.tenant.community()))
                == Some(
                    crate::authorization_runtime::finalization::AuthorizationMode::DenyProtected,
                )
            {
                metrics::counter!(
                    "buzz_auth_failures_total",
                    "reason" => "deny_protected"
                )
                .increment(1);
                *conn.auth_state.write().await = AuthState::Failed;
                conn.send(RelayMessage::ok(
                    &event_id_hex,
                    false,
                    "auth-required: protected authorization unavailable",
                ));
                conn.cancel.cancel();
                return;
            }

            // Community ban gate (NIP-42 seam). Runs immediately after auth
            // verification succeeds and before the allowlist and relay-membership
            // gates, per COMMUNITY_MODERATION_PLAN.md §0 decision 4 and the
            // MOD-7/M20 invariant (a ban must block connection auth even for open
            // channels — enforcement is structural, not filtered later). A banned
            // principal gets the standard protocol denial and the connection is
            // dropped with zero further processing.
            //
            // NIP-OA cascade: a ban on the authenticated pubkey blocks it directly;
            // a ban on its cryptographically-proven owner cascades to the agent
            // (owner ban ⇒ agents banned; agent ban is agent-only). The owner is
            // extracted from the self-proving auth tag with no DB round-trip.
            {
                // Fail closed on a DB error, but distinguish it from a real ban:
                // a transient blip must deny (never let a banned principal
                // through) without telling an innocent user they are banned and
                // pinning `Failed` for the connection's life on a false premise.
                // `Banned` claims the ban; `DbError` denies with `error: internal`
                // (mirrors the ingest write-path gate).
                enum BanOutcome {
                    Clear,
                    Banned,
                    DbError,
                }

                let mut outcome = match state
                    .db
                    .moderation_restriction_state(conn.tenant.community(), pubkey.as_bytes())
                    .await
                {
                    Ok(state) if state.banned => BanOutcome::Banned,
                    Ok(_) => BanOutcome::Clear,
                    Err(e) => {
                        warn!(conn_id = %conn_id, error = %e,
                              "ban-state DB lookup failed, denying (fail-closed)");
                        BanOutcome::DbError
                    }
                };

                // Cascade: check the proven NIP-OA owner only if the agent itself
                // is clear (a DB error already denies; a direct ban already blocks
                // — both skip the needless second DB read).
                if matches!(outcome, BanOutcome::Clear) {
                    if let Some(owner) = crate::api::relay_members::extract_nip_oa_owner(
                        pubkey.as_bytes(),
                        auth_tag_json.as_deref(),
                    ) {
                        outcome = match state
                            .db
                            .moderation_restriction_state(conn.tenant.community(), owner.as_bytes())
                            .await
                        {
                            Ok(state) if state.banned => BanOutcome::Banned,
                            Ok(_) => BanOutcome::Clear,
                            Err(e) => {
                                warn!(conn_id = %conn_id, error = %e,
                                      "owner ban-state DB lookup failed, denying (fail-closed)");
                                BanOutcome::DbError
                            }
                        };
                    }
                }

                let denial: Option<(&str, &str)> = match outcome {
                    BanOutcome::Clear => None,
                    BanOutcome::Banned => {
                        Some(("banned", "blocked: you are banned from this community"))
                    }
                    BanOutcome::DbError => Some((
                        "ban_check_error",
                        "error: internal error checking restriction state",
                    )),
                };

                if let Some((metric_reason, deny_reason)) = denial {
                    warn!(conn_id = %conn_id, reason = deny_reason, "principal denied at ban seam");
                    metrics::counter!("buzz_auth_failures_total", "reason" => metric_reason)
                        .increment(1);
                    *conn.auth_state.write().await = AuthState::Failed;
                    // Decision 4: banned ⇒ OK false + immediate WebSocket close.
                    // Route the reason frame on the control channel (not `send`,
                    // which uses the data channel and would race the cancel), so
                    // the send loop drains it ahead of the Close it emits on
                    // cancel. Then cancel to close the socket immediately.
                    let _ = conn.ctrl_tx.try_send(WsMessage::Text(
                        RelayMessage::ok(&event_id_hex, false, deny_reason).into(),
                    ));
                    conn.cancel.cancel();
                    return;
                }
            }

            let identity_lane = crate::authorization_runtime::transport::legacy_identity_lane(
                &state,
                conn.tenant.community(),
            );
            let neutral_evidence = conn.corporate_identity_assertion.is_none()
                && crate::authorization_runtime::transport::provider_evidence_resolver_is_installed(
                    &state,
                    conn.tenant.community(),
                );
            let identity_proof = if neutral_evidence {
                None
            } else {
                match crate::corporate_identity::verify_corporate_identity(
                    &state,
                    conn.tenant.community(),
                    pubkey,
                    conn.corporate_identity_assertion.as_ref(),
                    auth_tag_json.as_deref(),
                )
                .await
                {
                    Ok(proof) => Some(proof),
                    Err(e) => {
                        warn!(conn_id = %conn_id, error = ?e, "corporate identity denied");
                        if identity_lane
                        == crate::authorization_runtime::transport::LegacyIdentityLane::ObserveOnly
                    {
                        None
                    } else {
                        *conn.auth_state.write().await = AuthState::Failed;
                        conn.send(RelayMessage::ok(
                            &event_id_hex,
                            false,
                            &format!("restricted: {}", e.public_message()),
                        ));
                        return;
                    }
                    }
                }
            };

            let verified_assertion = match identity_proof.as_ref() {
                Some(proof) => {
                    match crate::corporate_identity::current_verified_assertion_for_proof(
                        &state,
                        proof,
                        conn.tenant.community(),
                        AuthTransport::RelayWebSocket,
                    ) {
                        Ok(assertion) => assertion.map(Arc::new),
                        Err(error) => {
                            warn!(conn_id = %conn_id, error = %error, "federated evidence sealing failed");
                            if identity_lane
                                == crate::authorization_runtime::transport::LegacyIdentityLane::ObserveOnly
                            {
                                None
                            } else {
                            *conn.auth_state.write().await = AuthState::Failed;
                            return;
                            }
                        }
                    }
                }
                None => None,
            };

            // Pubkey allowlist gate — only for pubkey-only auth.
            if state.config.pubkey_allowlist_enabled
                && auth_ctx.auth_method == buzz_auth::AuthMethod::Nip42
            {
                let allowed = match state
                    .db
                    .is_pubkey_allowed(conn.tenant.community(), pubkey.as_bytes())
                    .await
                {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(conn_id = %conn_id, error = %e,
                              "allowlist DB lookup failed, denying (fail-closed)");
                        false
                    }
                };
                if !allowed {
                    warn!(conn_id = %conn_id, "pubkey not in allowlist");
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
                conn.tenant.community(),
                pubkey.as_bytes(),
                auth_tag_json.as_deref(),
            )
            .await
            {
                Ok(owner) => owner,
                Err(e) => {
                    warn!(conn_id = %conn_id, error = ?e, "not a relay member");
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

            let identity_decision = if identity_lane
                == crate::authorization_runtime::transport::LegacyIdentityLane::Legacy
            {
                if let Some(identity_proof) = identity_proof.clone() {
                    match crate::corporate_identity::finalize_corporate_identity(
                        &state,
                        conn.tenant.community(),
                        pubkey,
                        identity_proof,
                    )
                    .await
                    {
                        Ok(decision) => Some(decision),
                        Err(e) => {
                            warn!(conn_id = %conn_id, error = ?e, "corporate identity finalization denied");
                            *conn.auth_state.write().await = AuthState::Failed;
                            conn.send(RelayMessage::ok(
                                &event_id_hex,
                                false,
                                &format!("restricted: {}", e.public_message()),
                            ));
                            return;
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(crate::corporate_identity::CorporateIdentityDecision::Delegated {
                owner_pubkey,
                ..
            }) = &identity_decision
            {
                auth_ctx.agent_owner_pubkey = Some(*owner_pubkey);
            }

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

            // Stash NIP-OA owner on the auth context only after the shared
            // backfill confirms the first-write-wins relationship.
            if let Some(owner) = nip_oa_owner {
                let owner_is_current = identity_lane
                    != crate::authorization_runtime::transport::LegacyIdentityLane::Legacy
                    || crate::api::relay_members::materialize_nip_oa_owner(
                        &state,
                        &conn.tenant,
                        &pubkey,
                        &owner,
                    )
                    .await;
                if owner_is_current {
                    auth_ctx.agent_owner_pubkey = Some(owner);
                } else {
                    warn!(
                        conn_id = %conn_id,
                        "NIP-OA owner could not be materialized"
                    );
                }
            }

            info!(conn_id = %conn_id, "NIP-42 auth successful");
            let transport_delegation =
                crate::corporate_identity::verify_unconditional_nip_oa_relationship(
                    pubkey,
                    auth_tag_json.as_deref(),
                )
                .map(|relationship| {
                    VerifiedDelegationOutput::from_workspace_verifier(
                        relationship.owner_pubkey(),
                        pubkey,
                        relationship.relationship_id(),
                        relationship.relationship_revision(),
                        None,
                        true,
                    )
                });
            let verified_proof: Arc<VerifiedNostrProof> = match VerifiedEvidenceAdapter::new()
                .verify_nip42(
                    conn.tenant.community(),
                    AuthTransport::RelayWebSocket,
                    &verified_event,
                    &challenge,
                    &relay_url,
                    transport_delegation,
                ) {
                Ok(proof) => Arc::new(proof),
                Err(error) => {
                    warn!(conn_id = %conn_id, error = %error, "sealed NIP-42 evidence creation failed");
                    *conn.auth_state.write().await = AuthState::Failed;
                    conn.send(RelayMessage::ok(
                        &event_id_hex,
                        false,
                        "auth-required: verification failed",
                    ));
                    return;
                }
            };
            let client_binding_scope =
                ClientBindingScopeV1::from_verified_auth_event(&verified_event)
                    .ok()
                    .filter(|scope| scope.relay_signer() == state.relay_keypair.public_key());
            *conn.auth_state.write().await = AuthState::Authenticated(auth_ctx);
            state.conn_manager.set_authenticated_authority(
                conn_id,
                Arc::clone(&verified_proof),
                verified_assertion.clone(),
            );
            if let (Some(runtime), Some(assertion), Some(scope)) = (
                state.client_status_runtime().cloned(),
                verified_assertion,
                client_binding_scope,
            ) {
                let bootstrap_queued = ClientBindingBootstrapInputV1::new(
                    conn.tenant.community(),
                    pubkey,
                    scope.connection_epoch().clone(),
                    nostr::Timestamp::now().as_secs(),
                )
                .ok()
                .and_then(|input| input.sign_with_relay_keys(&state.relay_keypair).ok())
                .is_some_and(|event| {
                    conn.send(RelayMessage::event(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &event))
                });
                let presentation = if bootstrap_queued {
                    runtime
                        .present_after_auth(
                            Arc::clone(&state),
                            verified_proof,
                            assertion,
                            conn_id,
                            conn.cancel.clone(),
                        )
                        .await
                } else {
                    Err(crate::authorization_runtime::status::ClientStatusRuntimeError::DeliveryUnavailable)
                };
                if let Err(error) = presentation {
                    // Presentation failure never widens or narrows access. The
                    // client receives no current indicator and clears any old
                    // status on its existing freshness/disconnect boundary.
                    metrics::counter!("buzz_client_status_degradation_total").increment(1);
                    warn!(
                        conn_id = %conn_id,
                        reason = "client_status_unavailable",
                        "client binding status withheld"
                    );
                    tracing::debug!(error = %error, "client binding status detail");
                }
            }
            if let Some(identity_decision) = identity_decision {
                crate::corporate_identity::spawn_session_revalidation(
                    Arc::clone(&state),
                    conn.tenant.community(),
                    pubkey,
                    identity_decision,
                    conn.cancel.clone(),
                );
            }
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

#[cfg(test)]
mod tests {
    use super::extract_auth_tag_json;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    #[test]
    fn observational_auth_cannot_enter_mutating_identity_lane() {
        use crate::authorization_runtime::{
            finalization::AuthorizationMode,
            transport::{legacy_identity_lane_for_mode, LegacyIdentityLane},
        };

        let mut binding_writes = 0;
        let mut membership_writes = 0;
        let mut public_projection_writes = 0;
        for mode in [
            AuthorizationMode::Shadow,
            AuthorizationMode::VerifyOnly,
            AuthorizationMode::Enforce,
        ] {
            if legacy_identity_lane_for_mode(Some(mode)) == LegacyIdentityLane::Legacy {
                binding_writes += 1;
                membership_writes += 1;
                public_projection_writes += 1;
            }
        }
        assert_eq!(binding_writes, 0);
        assert_eq!(membership_writes, 0);
        assert_eq!(public_projection_writes, 0);
        assert_eq!(
            legacy_identity_lane_for_mode(Some(AuthorizationMode::Off)),
            LegacyIdentityLane::Legacy
        );
        assert_eq!(
            legacy_identity_lane_for_mode(Some(AuthorizationMode::Shadow)),
            LegacyIdentityLane::ObserveOnly
        );
        assert_eq!(
            legacy_identity_lane_for_mode(Some(AuthorizationMode::VerifyOnly)),
            LegacyIdentityLane::ObserveOnly
        );
        assert_eq!(
            legacy_identity_lane_for_mode(Some(AuthorizationMode::Enforce)),
            LegacyIdentityLane::ProtectedEnforce
        );
    }

    /// Build a signed NIP-98 (kind 27235) event carrying the given tags. The
    /// `auth` tag lives inside the signed event exactly as the git and
    /// WebSocket auth paths receive it.
    fn signed_event_with_tags(tags: Vec<Tag>) -> nostr::Event {
        EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign auth event")
    }

    /// A single `auth` tag is extracted verbatim as its JSON-array string —
    /// this is the exact value fed to `verify_auth_tag` on the git path.
    #[test]
    fn single_auth_tag_extracted_verbatim() {
        let owner = Keys::generate().public_key().to_hex();
        let sig = "00".repeat(64);
        let event = signed_event_with_tags(vec![
            Tag::parse(["u", "https://relay/git/x/y"]).unwrap(),
            Tag::parse(["auth", owner.as_str(), "", sig.as_str()]).unwrap(),
        ]);

        let extracted = extract_auth_tag_json(&event).expect("auth tag present");
        let expected = serde_json::to_string(&["auth", owner.as_str(), "", sig.as_str()]).unwrap();
        assert_eq!(extracted, expected);
    }

    /// No `auth` tag → `None` (the direct-member path, tag absent).
    #[test]
    fn no_auth_tag_returns_none() {
        let event =
            signed_event_with_tags(vec![Tag::parse(["u", "https://relay/git/x/y"]).unwrap()]);
        assert_eq!(extract_auth_tag_json(&event), None);
    }

    /// More than one `auth` tag → `None`. Per NIP-OA, an ambiguous set of
    /// attestations is treated as no valid attestation (fail-closed), so a
    /// second forged tag cannot smuggle an alternate delegation past the gate.
    #[test]
    fn duplicate_auth_tags_return_none() {
        let a = Keys::generate().public_key().to_hex();
        let b = Keys::generate().public_key().to_hex();
        let sig = "00".repeat(64);
        let event = signed_event_with_tags(vec![
            Tag::parse(["auth", a.as_str(), "", sig.as_str()]).unwrap(),
            Tag::parse(["auth", b.as_str(), "", sig.as_str()]).unwrap(),
        ]);
        assert_eq!(extract_auth_tag_json(&event), None);
    }
}
