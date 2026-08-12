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
use buzz_core::client_binding_bootstrap::ClientBindingScopeV1;
use buzz_db::authorization_admission::{
    AdmissionApplicationContext, AdmissionApplicationEffect, AdmissionApplicationOutcome,
    AdmissionApplicationResult, AdmissionApplicationResultSchema, AdmissionCommitError,
    AdmissionCommitOutcome, AdmissionCommitRequest, AdmissionObject, CanonicalAdmissionCommitter,
};
use tracing::{debug, info, warn};

use crate::authorization_runtime::outbox::{
    CommittedStatusDeliveryAuthority, DurableStatusContract, DurableStatusSink,
};
use crate::authorization_runtime::{
    ConnectionStatusSession, CurrentStatusAuthorization, CurrentStatusEvidenceSource,
    ProviderFreeRuntimeMode, StatusCadence, StatusSessionError,
};
use crate::connection::{AuthState, ConnectionState};
use crate::protocol::RelayMessage;
use crate::state::AppState;

#[derive(Clone, Copy, PartialEq, Eq)]
struct StatusConnectionBinding {
    request: [u8; 32],
    target: [u8; 32],
    transport: [u8; 32],
}

impl StatusConnectionBinding {
    fn from_coordinates(coordinates: &buzz_auth::Nip42BindingStatusCoordinates) -> Self {
        let (request, target, transport) = coordinates.request_binding();
        Self {
            request: *request,
            target: *target,
            transport: *transport,
        }
    }
}

fn committed_status_receipt_matches(
    authorization_request: &[u8; 32],
    receipt: buzz_db::authorization_admission::AdmissionCommitReceipt,
    expected_result_digest: &[u8; 32],
) -> bool {
    authorization_request == receipt.request_fingerprint()
        && receipt.application_result_digest() == Some(expected_result_digest)
}

#[derive(Clone, Copy)]
struct BindingStatusApplicationBinding {
    object: AdmissionObject,
    actor: nostr::PublicKey,
    event_id: [u8; 32],
    connection: Option<StatusConnectionBinding>,
    intent_digest: [u8; 32],
}

impl BindingStatusApplicationBinding {
    fn new(
        object: AdmissionObject,
        actor: nostr::PublicKey,
        event_id: [u8; 32],
        challenge: &str,
        connection: Option<StatusConnectionBinding>,
    ) -> Self {
        let connection_enabled = [u8::from(connection.is_some())];
        let connection_request = connection.map_or([0; 32], |binding| binding.request);
        let connection_target = connection.map_or([0; 32], |binding| binding.target);
        let connection_transport = connection.map_or([0; 32], |binding| binding.transport);
        Self {
            object,
            actor,
            event_id,
            connection,
            intent_digest: crate::protected_ingress::fingerprint(
                b"buzz:nip-fi:binding-status-admission-intent:v1",
                &[
                    object.key(),
                    actor.as_bytes(),
                    &event_id,
                    challenge.as_bytes(),
                    &connection_enabled,
                    &connection_request,
                    &connection_target,
                    &connection_transport,
                ],
            ),
        }
    }

    fn application_result(self) -> Result<AdmissionApplicationResult, AdmissionCommitError> {
        let mut payload = Vec::with_capacity(130);
        payload.push(1);
        payload.extend_from_slice(&self.event_id);
        payload.push(u8::from(self.connection.is_some()));
        if let Some(connection) = self.connection {
            payload.extend_from_slice(&connection.request);
            payload.extend_from_slice(&connection.target);
            payload.extend_from_slice(&connection.transport);
        }
        AdmissionApplicationResult::new(
            AdmissionApplicationResultSchema::binding_status(),
            1,
            payload,
        )
    }

    fn application_result_digest(
        self,
        result_binding: buzz_db::authorization_admission::AdmissionApplicationResultBinding,
        result: &AdmissionApplicationResult,
    ) -> Result<[u8; 32], AdmissionCommitError> {
        let authorization_domain = result_binding.authorization_domain();
        let semantic_fingerprint = result_binding.semantic_fingerprint();
        let application_intent_digest = result_binding.application_intent_digest();
        if authorization_domain.as_uuid().is_nil()
            || semantic_fingerprint == &[0; 32]
            || application_intent_digest == &[0; 32]
        {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        let schema = result.schema();
        Ok(crate::protected_ingress::fingerprint(
            b"buzz:canonical-application-result:v1",
            &[
                authorization_domain.as_uuid().as_bytes(),
                &self.object.kind().database_code().to_be_bytes(),
                self.object.key(),
                semantic_fingerprint,
                application_intent_digest,
                schema.type_key(),
                &schema.version().to_be_bytes(),
                &result.code().to_be_bytes(),
                result.payload(),
            ],
        ))
    }

    fn bind_committed(
        self,
        authorization: buzz_auth::FinalizedAuthContext,
        receipt: buzz_db::authorization_admission::AdmissionCommitReceipt,
        application_result: AdmissionApplicationResult,
        result_binding: buzz_db::authorization_admission::AdmissionApplicationResultBinding,
        connection_lease: Option<buzz_auth::BoundedAuthorizationLease>,
    ) -> Result<Option<CommittedStatusActivation>, AdmissionCommitError> {
        let expected_result = self.application_result()?;
        let expected_result_digest =
            self.application_result_digest(result_binding, &expected_result)?;
        let admission_lease = authorization.lease().clone();
        let (_, admission_target, _) = admission_lease.request_binding();
        if authorization.authorization_domain() != receipt.authorization_domain()
            || authorization.authorization_domain() != result_binding.authorization_domain()
            || authorization.authorization_domain() != admission_lease.authorization_domain()
            || authorization.actor_pubkey() != self.actor
            || authorization.capability() != buzz_auth::RouteCapability::BindingStatus
            || authorization.owner_pubkey().is_some()
            || !committed_status_receipt_matches(
                authorization.request_fingerprint(),
                receipt,
                &expected_result_digest,
            )
            || admission_target != self.object.key()
            || receipt.object() != self.object
            || result_binding.object() != self.object
            || receipt.semantic_fingerprint() != result_binding.semantic_fingerprint()
            || result_binding.application_intent_digest() != &self.intent_digest
            || application_result != expected_result
        {
            return Err(AdmissionCommitError::IntentConflict);
        }
        match (self.connection, connection_lease) {
            (None, None) => Ok(None),
            (Some(binding), Some(connection_lease)) => {
                let delivery_authority = CommittedStatusDeliveryAuthority::new(
                    self.object,
                    admission_lease,
                    connection_lease,
                    (binding.request, binding.target, binding.transport),
                )
                .map_err(|_| AdmissionCommitError::IntentConflict)?;
                let authorization = CurrentStatusAuthorization::from_committed_connection(
                    delivery_authority.connection_lease(),
                    delivery_authority.admission_lease(),
                )
                .map_err(|_| AdmissionCommitError::IntentConflict)?;
                Ok(Some(CommittedStatusActivation {
                    authorization,
                    delivery_authority,
                }))
            }
            (None, Some(_)) | (Some(_), None) => Err(AdmissionCommitError::IntentConflict),
        }
    }
}

struct BindingStatusAdmissionEffect {
    binding: BindingStatusApplicationBinding,
}

impl AdmissionApplicationEffect for BindingStatusAdmissionEffect {
    fn intent_digest(&self) -> [u8; 32] {
        self.binding.intent_digest
    }

    fn result_schema(&self) -> AdmissionApplicationResultSchema {
        AdmissionApplicationResultSchema::binding_status()
    }

    fn apply<'a, 'transaction>(
        &'a mut self,
        _transaction: &'a mut sqlx::Transaction<'transaction, sqlx::Postgres>,
        context: &'a AdmissionApplicationContext<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<AdmissionApplicationOutcome, AdmissionCommitError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            if context.object() != self.binding.object
                || context.authorization().actor_pubkey() != self.binding.actor
                || context.authorization().capability() != buzz_auth::RouteCapability::BindingStatus
            {
                return Err(AdmissionCommitError::AuthorizationDenied);
            }
            let result = self.binding.application_result()?;
            let effect_digest = crate::protected_ingress::fingerprint(
                b"buzz:nip-fi:binding-status-admission-effect:v1",
                &[
                    context.authorization_domain().as_uuid().as_bytes(),
                    context.operation_id().as_bytes(),
                    context.request_fingerprint(),
                    &self.binding.intent_digest,
                ],
            );
            AdmissionApplicationOutcome::new(result, effect_digest)
        })
    }
}

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

fn status_activation_scope_matches(
    verified_auth_event: &nostr::Event,
    authorization: &CurrentStatusAuthorization,
    tenant_domain: buzz_core::CommunityId,
) -> bool {
    let (authorization_domain, authorization_author) = authorization.domain_author();
    authorization_domain == tenant_domain && authorization_author == verified_auth_event.pubkey
}

struct CommittedStatusActivation {
    authorization: CurrentStatusAuthorization,
    delivery_authority: CommittedStatusDeliveryAuthority,
}

/// Activate current-only status after the caller has completed canonical
/// scoped NIP-42 finalization and obtained a status authorization.
///
/// Only the Enforce branch of [`handle_auth`] can call this function because it
/// receives the finalized `BindingStatus` lease and verifier-authenticated peer
/// from the canonical connection admission. The connection manager cancels
/// and joins any prior AUTH scope before this function delivers the replacement
/// bootstrap, then becomes sole owner of the new task. `Ok(None)` means the
/// optional presentation was withheld by authenticated-peer admission without
/// changing the completed AUTH decision.
async fn activate_client_binding_status<E>(
    verified_auth_event: &nostr::Event,
    activation: CommittedStatusActivation,
    authenticated_peer: buzz_auth::AuthenticatedClientPeer,
    evidence: Arc<E>,
    conn: Arc<ConnectionState>,
    state: Arc<AppState>,
) -> Result<Option<tokio_util::sync::CancellationToken>, StatusSessionError>
where
    E: CurrentStatusEvidenceSource + ?Sized + 'static,
{
    let CommittedStatusActivation {
        authorization,
        delivery_authority,
    } = activation;
    if state.authorization_runtime.mode() != ProviderFreeRuntimeMode::Enforce
        || !state.authorization_runtime.is_ready()
    {
        conn.cancel.cancel();
        return Err(StatusSessionError::EvidenceUnavailable);
    }
    if !status_activation_scope_matches(
        verified_auth_event,
        &authorization,
        conn.tenant.community(),
    ) {
        conn.cancel.cancel();
        return Err(StatusSessionError::EvidenceChanged);
    }
    let scope =
        ClientBindingScopeV1::from_verified_auth_event(verified_auth_event).map_err(|_| {
            conn.cancel.cancel();
            StatusSessionError::ContractUnavailable
        })?;
    if !conn.clear_client_binding_status_task().await {
        conn.cancel.cancel();
        return Err(StatusSessionError::DeliveryFailed);
    }
    let admission_policy = state
        .config
        .nip_fi
        .enforce()
        .ok_or_else(|| {
            conn.cancel.cancel();
            StatusSessionError::EvidenceUnavailable
        })?
        .client_status_admission();
    if crate::admission::check_client_status_presentation(
        state.admission_rate_limiter.as_ref(),
        &conn.tenant,
        &verified_auth_event.pubkey,
        &authenticated_peer,
        admission_policy,
    )
    .await
    .is_err()
    {
        return Ok(None);
    }
    *conn.status_scope.write().await = Some(scope);
    let activation_now = state
        .db
        .status_delivery_authoritative_now()
        .await
        .map_err(|_| StatusSessionError::EvidenceUnavailable)?;
    let (sink, bootstrap) = DurableStatusSink::activate(
        state.config.nip_fi_mode,
        state.db.clone(),
        Arc::clone(&evidence),
        Arc::clone(&conn),
        delivery_authority,
        &state.relay_keypair,
        activation_now,
    )
    .await
    .map_err(|_| StatusSessionError::DeliveryFailed)?;
    while sink
        .recover_one()
        .await
        .map_err(|_| StatusSessionError::DeliveryFailed)?
    {}
    let contract = DurableStatusContract::new(
        state.relay_keypair.clone(),
        state.relay_keypair.public_key(),
    )
    .inspect_err(|_| {
        conn.cancel.cancel();
    })?;
    let cancel = conn.cancel.child_token();
    let task_cancel = cancel.clone();
    let mut session = ConnectionStatusSession::new(
        evidence,
        contract,
        sink,
        StatusCadence::production(),
        bootstrap,
    );
    // The start gate makes task ownership structural: no 24244 work begins
    // until the connection manager has installed this exact task. Replacement
    // likewise joins the old task (and its withdrawal) before opening the gate.
    let (start, started) = tokio::sync::oneshot::channel();
    let (initial_presentation, presented) = tokio::sync::oneshot::channel();
    let join = tokio::spawn(async move {
        let gate_cancel = task_cancel.clone();
        tokio::select! {
            _ = gate_cancel.cancelled() => {}
            started = started => {
                if started.is_ok() {
                    match session.present(&authorization).await {
                        Ok(_) => {
                            if initial_presentation.send(Ok(())).is_err() {
                                let _ = session.withdraw(chrono::Utc::now()).await;
                                return;
                            }
                            if let Err(error) = session
                                .run_until_cancelled(authorization, task_cancel)
                                .await
                            {
                                tracing::warn!(error = %error, "current-binding status session closed fail-closed");
                            }
                        }
                        Err(error) => {
                            let _ = initial_presentation.send(Err(error));
                        }
                    }
                }
            }
        }
    });
    if !conn
        .replace_client_binding_status_task(cancel.clone(), join)
        .await
    {
        conn.cancel.cancel();
        return Err(StatusSessionError::DeliveryFailed);
    }
    if start.send(()).is_err() {
        let _ = conn.clear_client_binding_status_task().await;
        conn.cancel.cancel();
        return Err(StatusSessionError::DeliveryFailed);
    }
    match presented.await {
        Ok(Ok(())) if !cancel.is_cancelled() => Ok(Some(cancel)),
        Ok(Err(error)) => {
            let _ = conn.clear_client_binding_status_task().await;
            conn.cancel.cancel();
            Err(error)
        }
        Ok(Ok(())) | Err(_) => {
            let _ = conn.clear_client_binding_status_task().await;
            conn.cancel.cancel();
            Err(StatusSessionError::DeliveryFailed)
        }
    }
}

/// Handle a NIP-42 AUTH message: verify the challenge response and transition
/// the connection to authenticated state.
///
/// Pure crypto verification — no API tokens, no JWT, no DB token lookups.
#[tracing::instrument(skip_all, fields(event_id, conn_id))]
pub async fn handle_auth(event: nostr::Event, conn: Arc<ConnectionState>, state: Arc<AppState>) {
    let event_id_hex = event.id.to_hex();
    let canonical_event = event.clone();
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
            if state.config.nip_fi_mode == buzz_auth::NipFiMode::DenyProtected {
                *conn.auth_state.write().await = AuthState::Failed;
                conn.send(RelayMessage::ok(
                    &event_id_hex,
                    false,
                    "auth-required: protected route unavailable",
                ));
                return;
            }

            let mut prepared_canonical = if state.config.nip_fi_mode
                == buzz_auth::NipFiMode::Enforce
            {
                match prepare_canonical_websocket(
                    &state,
                    &conn,
                    &canonical_event,
                    &challenge,
                    &relay_url,
                )
                .await
                {
                    Ok(admission) => Some(admission),
                    Err(error) => {
                        warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = ?error, "canonical WebSocket admission denied");
                        *conn.auth_state.write().await = AuthState::Failed;
                        let reason = match error {
                            crate::protected_ingress::ProtectedIngressError::Denied => {
                                "restricted: authorization denied"
                            }
                            crate::protected_ingress::ProtectedIngressError::Expired => {
                                "nip_fi_auth_expired"
                            }
                            crate::protected_ingress::ProtectedIngressError::Unavailable => {
                                "error: authorization unavailable"
                            }
                        };
                        conn.send(RelayMessage::ok(&event_id_hex, false, reason));
                        return;
                    }
                }
            } else {
                None
            };

            // Community ban gate (NIP-42 seam). In Enforce mode, canonical
            // preflight has already finalized read authority and prepared the
            // status mutation; the mutation is committed only after this gate.
            // The ban check still precedes allowlist and relay-membership gates,
            // per COMMUNITY_MODERATION_PLAN.md §0 decision 4 and the
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
                        warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = %e,
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
                                warn!(conn_id = %conn_id, owner = %owner.to_hex(), error = %e,
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
                    warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), reason = deny_reason, "principal denied at ban seam");
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

            let identity_proof = if state.config.nip_fi_mode == buzz_auth::NipFiMode::Off {
                match crate::corporate_identity::verify_corporate_identity(
                    &state,
                    conn.tenant.community(),
                    pubkey,
                    conn.corporate_identity_jwt.as_deref(),
                    auth_tag_json.as_deref(),
                )
                .await
                {
                    Ok(proof) => Some(proof),
                    Err(e) => {
                        warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = %e, "corporate identity denied");
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
                conn.tenant.community(),
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

            let mut canonical_status = None;
            let identity_decision = match state.config.nip_fi_mode {
                buzz_auth::NipFiMode::Off => {
                    let Some(identity_proof) = identity_proof else {
                        *conn.auth_state.write().await = AuthState::Failed;
                        conn.send(RelayMessage::ok(
                            &event_id_hex,
                            false,
                            "error: authorization unavailable",
                        ));
                        return;
                    };
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
                            warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = %e, "corporate identity finalization denied");
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
                buzz_auth::NipFiMode::Enforce => {
                    let Some(prepared) = prepared_canonical.take() else {
                        *conn.auth_state.write().await = AuthState::Failed;
                        conn.send(RelayMessage::ok(
                            &event_id_hex,
                            false,
                            "error: authorization unavailable",
                        ));
                        return;
                    };
                    let admission = match commit_canonical_websocket(&state, prepared).await {
                        Ok(admission) => admission,
                        Err(error) => {
                            warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = ?error, "canonical WebSocket admission denied");
                            *conn.auth_state.write().await = AuthState::Failed;
                            let reason = match error {
                                crate::protected_ingress::ProtectedIngressError::Denied => {
                                    "restricted: authorization denied"
                                }
                                crate::protected_ingress::ProtectedIngressError::Expired => {
                                    "nip_fi_auth_expired"
                                }
                                crate::protected_ingress::ProtectedIngressError::Unavailable => {
                                    "error: authorization unavailable"
                                }
                            };
                            conn.send(RelayMessage::ok(&event_id_hex, false, reason));
                            return;
                        }
                    };
                    canonical_status = Some(admission);
                    None
                }
                buzz_auth::NipFiMode::DenyProtected => None,
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
                if crate::api::relay_members::materialize_nip_oa_owner(
                    &state,
                    &conn.tenant,
                    &pubkey,
                    &owner,
                )
                .await
                {
                    auth_ctx.agent_owner_pubkey = Some(owner);
                } else {
                    warn!(
                        conn_id = %conn_id,
                        agent = %pubkey.to_hex(),
                        nip_oa_owner = %owner.to_hex(),
                        "NIP-OA owner could not be materialized"
                    );
                }
            }

            let mut authenticated_peer = None;
            let mut canonical_session = None;
            if let Some(admission) = canonical_status {
                let CanonicalWebsocketAdmission {
                    status_activation,
                    authenticated_peer: canonical_peer,
                    read_authorization,
                    event_assertion,
                } = admission;
                if let Some(status_activation) = status_activation {
                    let evidence: Arc<
                        dyn crate::authorization_runtime::CurrentStatusEvidenceSource,
                    > = {
                        #[cfg(test)]
                        if let Some(override_source) =
                            state.client_status_evidence_override.read().await.clone()
                        {
                            override_source
                        } else {
                            Arc::new(
                                crate::authorization_runtime::LocalBindingStatusEvidenceSource::new(
                                    Arc::new(state.db.clone()),
                                ),
                            )
                        }
                        #[cfg(not(test))]
                        Arc::new(
                            crate::authorization_runtime::LocalBindingStatusEvidenceSource::new(
                                Arc::new(state.db.clone()),
                            ),
                        )
                    };
                    if let Err(error) = activate_client_binding_status(
                        &canonical_event,
                        status_activation,
                        canonical_peer,
                        evidence,
                        Arc::clone(&conn),
                        Arc::clone(&state),
                    )
                    .await
                    {
                        warn!(conn_id = %conn_id, error = %error, "current-binding status activation failed closed");
                        *conn.auth_state.write().await = AuthState::Failed;
                        let _ = conn.ctrl_tx.try_send(WsMessage::Text(
                            RelayMessage::ok(
                                &event_id_hex,
                                false,
                                "error: current-binding status unavailable",
                            )
                            .into(),
                        ));
                        conn.cancel.cancel();
                        return;
                    }
                }
                authenticated_peer = Some(canonical_peer);
                canonical_session = Some(crate::connection::CanonicalWebsocketSession::new(
                    read_authorization,
                    event_assertion,
                ));
            }
            if let Some(canonical_session) = canonical_session {
                *conn.canonical_authorization.write().await = Some(canonical_session);
            }
            *conn.auth_state.write().await =
                AuthState::Authenticated(crate::connection::AuthenticatedConnectionContext::new(
                    auth_ctx,
                    authenticated_peer,
                ));
            info!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), "NIP-42 auth successful");
            state
                .conn_manager
                .set_authenticated_pubkey(conn_id, pubkey.to_bytes().to_vec());
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

struct CanonicalWebsocketAdmission {
    status_activation: Option<CommittedStatusActivation>,
    authenticated_peer: buzz_auth::AuthenticatedClientPeer,
    read_authorization: buzz_auth::FinalizedAuthContext,
    event_assertion: buzz_auth::VerifiedFederatedAssertion,
}

struct PreparedCanonicalWebsocketAdmission {
    status_request: AdmissionCommitRequest,
    status_binding: BindingStatusApplicationBinding,
    connection_status_lease: Option<buzz_auth::BoundedAuthorizationLease>,
    authenticated_peer: buzz_auth::AuthenticatedClientPeer,
    read_authorization: buzz_auth::FinalizedAuthContext,
    event_assertion: buzz_auth::VerifiedFederatedAssertion,
}

async fn prepare_canonical_websocket(
    state: &AppState,
    conn: &ConnectionState,
    event: &nostr::Event,
    challenge: &str,
    relay_url: &str,
) -> Result<PreparedCanonicalWebsocketAdmission, crate::protected_ingress::ProtectedIngressError> {
    let domain = conn.tenant.community();
    let assertion_object = crate::protected_ingress::domain_object(domain)?;
    let object = AdmissionObject::binding_status_auth_event(domain, event.id.to_bytes())
        .ok_or(crate::protected_ingress::ProtectedIngressError::Denied)?;
    let event_id = event.id.to_bytes();
    let evidence = conn
        .canonical_transport_evidence
        .lock()
        .await
        .take()
        .ok_or(crate::protected_ingress::ProtectedIngressError::Unavailable)?;
    if evidence.authorization_domain() != domain
        || evidence.transport() != buzz_auth::ProofTransport::Nip42
    {
        return Err(crate::protected_ingress::ProtectedIngressError::Denied);
    }
    let authenticated_peer = evidence
        .authenticated_client_peer()
        .copied()
        .ok_or(crate::protected_ingress::ProtectedIngressError::Denied)?;
    let request_fingerprint = *evidence.request_fingerprint();
    let transport_context_fingerprint = *evidence.transport_context_fingerprint();
    let assertion_coordinates = crate::protected_ingress::ProtectedRequestCoordinates::new(
        crate::authorization_runtime::ProtectedIngress::WebSocketAuthenticate,
        domain,
        buzz_auth::RouteCapability::BindingStatus,
        object,
        buzz_auth::ProofTransport::Nip42,
        request_fingerprint,
        transport_context_fingerprint,
    )?;
    let status_assertion = crate::protected_ingress::verify_assertion(
        state,
        evidence.confidential_assertion(),
        assertion_coordinates,
    )
    .await?;
    let status_opted_in = event.tags.iter().any(|tag| {
        tag.as_slice().first().map(String::as_str)
            == Some(buzz_core::client_binding_bootstrap::CLIENT_BINDING_SCOPE_TAG)
    });
    let (connection_binding, connection_status_lease) = if status_opted_in {
        let authoritative_now = state
            .db
            .status_delivery_authoritative_now()
            .await
            .map_err(|_| crate::protected_ingress::ProtectedIngressError::Unavailable)?;
        let coordinates = buzz_auth::Nip42BindingStatusCoordinates::new(
            domain,
            relay_url,
            conn.conn_id,
            state.relay_keypair.public_key(),
            event,
        )
        .map_err(|_| crate::protected_ingress::ProtectedIngressError::Denied)?;
        let binding = StatusConnectionBinding::from_coordinates(&coordinates);
        let proof = buzz_auth::verify_nip42_binding_status_proof(
            event,
            challenge,
            &coordinates,
            authoritative_now,
        )
        .map_err(|_| crate::protected_ingress::ProtectedIngressError::Denied)?;
        let lease = state
            .db
            .finalize_binding_status_authorization(proof)
            .await
            .map_err(|error| match error {
                buzz_db::DbError::InvalidData(_) => {
                    crate::protected_ingress::ProtectedIngressError::Denied
                }
                _ => crate::protected_ingress::ProtectedIngressError::Unavailable,
            })?
            .lease()
            .clone();
        (Some(binding), Some(lease))
    } else {
        (None, None)
    };
    let session_assertion_coordinates = crate::protected_ingress::ProtectedRequestCoordinates::new(
        crate::authorization_runtime::ProtectedIngress::WebSocketQuery,
        domain,
        buzz_auth::RouteCapability::MessagesRead,
        assertion_object,
        buzz_auth::ProofTransport::Nip42,
        request_fingerprint,
        transport_context_fingerprint,
    )?;
    let session_assertion = crate::protected_ingress::verify_assertion(
        state,
        evidence.confidential_assertion(),
        session_assertion_coordinates,
    )
    .await?;
    let (_, status_assertion_expires_at) = status_assertion.time_bounds();
    let (_, session_assertion_expires_at) = session_assertion.time_bounds();
    let coordinates = crate::protected_ingress::ProtectedRequestCoordinates::new(
        crate::authorization_runtime::ProtectedIngress::WebSocketAuthenticate,
        domain,
        buzz_auth::RouteCapability::BindingStatus,
        object,
        buzz_auth::ProofTransport::Nip42,
        request_fingerprint,
        transport_context_fingerprint,
    )?;
    let proof = buzz_auth::verify_nip42_authorization_proof(
        event,
        challenge,
        relay_url,
        domain,
        request_fingerprint,
        *object.key(),
        transport_context_fingerprint,
        Some(*status_assertion.assertion_fingerprint()),
        None,
        status_assertion_expires_at.min(evidence.proxy_expires_at()),
    )
    .map_err(|_| crate::protected_ingress::ProtectedIngressError::Denied)?;
    let request =
        crate::protected_ingress::prepare_mutation(state, coordinates, status_assertion, proof)
            .await?;
    let status_binding = BindingStatusApplicationBinding::new(
        object,
        event.pubkey,
        event_id,
        challenge,
        connection_binding,
    );
    let request = request
        .with_application_effect(Box::new(BindingStatusAdmissionEffect {
            binding: status_binding,
        }))
        .map_err(|_| crate::protected_ingress::ProtectedIngressError::Denied)?;
    let request = crate::api::media::ProtectedTransportInstaller::install(request, evidence)
        .map_err(|_| crate::protected_ingress::ProtectedIngressError::Denied)?;
    let event_assertion = session_assertion.clone();
    let read_coordinates = crate::protected_ingress::ProtectedRequestCoordinates::new(
        crate::authorization_runtime::ProtectedIngress::WebSocketQuery,
        domain,
        buzz_auth::RouteCapability::MessagesRead,
        assertion_object,
        buzz_auth::ProofTransport::Nip42,
        request_fingerprint,
        transport_context_fingerprint,
    )?;
    let read_proof = buzz_auth::verify_nip42_authorization_proof(
        event,
        challenge,
        relay_url,
        domain,
        request_fingerprint,
        *assertion_object.key(),
        transport_context_fingerprint,
        Some(*session_assertion.assertion_fingerprint()),
        None,
        session_assertion_expires_at,
    )
    .map_err(|_| crate::protected_ingress::ProtectedIngressError::Denied)?;
    let read_authorization = crate::protected_ingress::authorize_read(
        state,
        read_coordinates,
        session_assertion.clone(),
        read_proof,
    )
    .await?;

    Ok(PreparedCanonicalWebsocketAdmission {
        status_request: request,
        status_binding,
        connection_status_lease,
        authenticated_peer,
        read_authorization,
        event_assertion,
    })
}

async fn commit_canonical_websocket(
    state: &AppState,
    prepared: PreparedCanonicalWebsocketAdmission,
) -> Result<CanonicalWebsocketAdmission, crate::protected_ingress::ProtectedIngressError> {
    let PreparedCanonicalWebsocketAdmission {
        status_request,
        status_binding,
        connection_status_lease,
        authenticated_peer,
        read_authorization,
        event_assertion,
    } = prepared;
    let committer = crate::protected_ingress::mutation_committer(state)?;
    let status_activation = match committer.commit(status_request).await {
        Ok(AdmissionCommitOutcome::Committed {
            authorization,
            receipt,
            application_result,
            application_result_binding,
        }) => {
            let application_result = application_result
                .ok_or(crate::protected_ingress::ProtectedIngressError::Denied)?;
            let application_result_binding = application_result_binding
                .ok_or(crate::protected_ingress::ProtectedIngressError::Denied)?;
            status_binding
                .bind_committed(
                    *authorization,
                    receipt,
                    application_result,
                    application_result_binding,
                    connection_status_lease,
                )
                .map_err(|_| crate::protected_ingress::ProtectedIngressError::Denied)?
        }
        Ok(AdmissionCommitOutcome::ExactReplay { .. }) => {
            return Err(crate::protected_ingress::ProtectedIngressError::Denied);
        }
        Err(
            AdmissionCommitError::DependencyUnavailable
            | AdmissionCommitError::AuditUnavailable
            | AdmissionCommitError::RecordedAuditUnavailable,
        ) => return Err(crate::protected_ingress::ProtectedIngressError::Unavailable),
        Err(_) => return Err(crate::protected_ingress::ProtectedIngressError::Denied),
    };
    Ok(CanonicalWebsocketAdmission {
        status_activation,
        authenticated_peer,
        read_authorization,
        event_assertion,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        committed_status_receipt_matches, extract_auth_tag_json, status_activation_scope_matches,
    };
    use crate::authorization_runtime::CurrentStatusAuthorization;
    use buzz_core::{AuthorizationLeaseFence, CanonicalCurrentBindingEvidence, CommunityId};
    use buzz_db::authorization_admission::{
        AdmissionCommitDigests, AdmissionCommitReceipt, AdmissionObject,
    };
    use chrono::Utc;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use uuid::Uuid;

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

    #[test]
    fn status_receipt_requires_exact_request_and_typed_result_digest() {
        let domain = CommunityId::from_uuid(Uuid::from_u128(1));
        let object = AdmissionObject::binding_status(domain, Keys::generate().public_key())
            .expect("build binding-status object");
        let receipt = |request_fingerprint, application_result_digest| {
            AdmissionCommitReceipt::from_storage(
                domain,
                object,
                Uuid::new_v4(),
                request_fingerprint,
                [8; 32],
                AdmissionCommitDigests::new([9; 32], Some(application_result_digest))
                    .expect("build commit digests"),
                Uuid::new_v4(),
            )
            .expect("build commit receipt")
        };
        let request_fingerprint = [7; 32];
        let application_result_digest = [10; 32];

        assert!(committed_status_receipt_matches(
            &request_fingerprint,
            receipt(request_fingerprint, application_result_digest),
            &application_result_digest,
        ));
        assert!(!committed_status_receipt_matches(
            &request_fingerprint,
            receipt([6; 32], application_result_digest),
            &application_result_digest,
        ));
        assert!(!committed_status_receipt_matches(
            &request_fingerprint,
            receipt(request_fingerprint, [11; 32]),
            &application_result_digest,
        ));
    }

    #[test]
    fn status_activation_requires_exact_auth_author_and_tenant_domain() {
        let author = Keys::generate();
        let domain = CommunityId::from_uuid(Uuid::from_u128(1));
        let now = Utc::now();
        let evidence = CanonicalCurrentBindingEvidence::new(
            domain,
            author.public_key(),
            Uuid::from_u128(2),
            3,
            4,
            5,
            6,
            AuthorizationLeaseFence::from_bytes([7; 32]).unwrap(),
            now,
            now + chrono::Duration::seconds(120),
        )
        .unwrap();
        let authorization = CurrentStatusAuthorization::from_test_parts(
            &evidence,
            now + chrono::Duration::seconds(120),
        );
        let auth_event = EventBuilder::new(Kind::Custom(22242), "")
            .sign_with_keys(&author)
            .unwrap();

        assert!(status_activation_scope_matches(
            &auth_event,
            &authorization,
            domain,
        ));
        assert!(!status_activation_scope_matches(
            &auth_event,
            &authorization,
            CommunityId::from_uuid(Uuid::from_u128(9)),
        ));
        let other_author = EventBuilder::new(Kind::Custom(22242), "")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert!(!status_activation_scope_matches(
            &other_author,
            &authorization,
            domain,
        ));
    }
}
