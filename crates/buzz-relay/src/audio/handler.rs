//! WebSocket audio handler: NIP-42 auth → room join → frame relay → cleanup.
//!
//! ```text
//! ws_audio_handler
//!   └─ handle_audio_connection
//!        ├─ send challenge, await auth (5s timeout)
//!        ├─ ensure_membership (auto-add for ephemeral channels)
//!        ├─ room.add_peer → broadcast joined
//!        ├─ spawn send_loop + heartbeat_loop
//!        ├─ run recv_loop (blocks until disconnect)
//!        └─ cleanup: remove peer, broadcast left, emit lifecycle events
//! ```

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::{future::poll_fn, pin::Pin};

use axum::extract::ws::{Message as WsMessage, WebSocket};
use axum::http::{HeaderMap, StatusCode};
use axum::{
    extract::{Path, State, WebSocketUpgrade},
    response::IntoResponse,
};
use bytes::Bytes;
use futures_util::{Sink, SinkExt, StreamExt};
use nostr::{EventBuilder, Kind, Tag};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use buzz_auth::{
    generate_challenge, AuthTransport, AuthorizationCapability, VerifiedDelegationOutput,
    VerifiedEvidenceAdapter,
};
use buzz_core::{tenant::TenantContext, CommunityId};
use buzz_db::{authorization_invalidation::AuthorizationSessionTarget, channel::MemberRole};

use buzz_core::StoredEvent;
use buzz_pubsub::EventTopic;

use crate::audio::room::{
    AudioRoomManager, PeerCtrl, ProtectedDeadlineSchedule, ProtectedPeerEffects,
    ProtectedPeerEpoch, Room, RoomOwnerEpoch,
};
use crate::authorization_runtime::transport::{
    authorize_exact_session_if_configured, ProtectedAuthorization,
};
use crate::state::{run_registered_community_connection, AppState};

/// Maximum binary frame size: 4 KB is generous for a single Opus packet.
const MAX_AUDIO_FRAME_BYTES: usize = 4096;

/// Maximum text frame size: 8 KB bounds auth/control JSON parsing.
const MAX_TEXT_FRAME_BYTES: usize = 8192;

/// Parser-level cap for this route. Text auth/control frames are the largest
/// message type audio accepts; binary Opus frames are bounded more tightly
/// after parsing.
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = MAX_TEXT_FRAME_BYTES;

/// Heartbeat interval.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Missed pong limit before disconnect.
const MAX_MISSED_PONGS: u8 = 3;

/// Auth timeout.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);

/// Enforce-only exact room claim. Every return path after room acquisition
/// passes through this drop guard, so cleanup cannot retire a replacement room
/// or release a replacement owner generation.
struct ProtectedRoomRetirement {
    rooms: Arc<AudioRoomManager>,
    community_id: CommunityId,
    channel_id: Uuid,
    room: Arc<Room>,
    owner_epoch: Option<RoomOwnerEpoch>,
    owners: Option<Arc<crate::audio::join::HuddleOwnerRegistry>>,
    local_runtime_id: Option<buzz_relay_mesh::RuntimeId>,
}

impl ProtectedRoomRetirement {
    fn retire_if_empty(&self) -> bool {
        if let Some(epoch) = self.owner_epoch {
            self.rooms.retire_exact_owner_if_empty(
                self.community_id,
                self.channel_id,
                &self.room,
                epoch,
                || {
                    if self.local_runtime_id == Some(epoch.owner_runtime_id) {
                        if let Some(owners) = &self.owners {
                            owners.release(self.channel_id, epoch.generation);
                        }
                    }
                },
            )
        } else {
            false
        }
    }
}

impl Drop for ProtectedRoomRetirement {
    fn drop(&mut self) {
        self.retire_if_empty();
    }
}

/// Exact fallback cleanup for every success, error, cancellation, timeout,
/// and early-return path after a protected peer activates.
struct ProtectedActivePeerGuard {
    room: std::sync::Weak<Room>,
    epoch: ProtectedPeerEpoch,
}

impl Drop for ProtectedActivePeerGuard {
    fn drop(&mut self) {
        if let Some(room) = self.room.upgrade() {
            room.remove_protected_epoch(self.epoch);
        }
    }
}

/// WebSocket upgrade handler for `/huddle/:channel_id/audio`.
pub async fn ws_audio_handler(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Row zero: bind this huddle-audio connection to its community from the
    // request host BEFORE the WebSocket upgrade, identical to the main relay
    // door. An unmapped host or lookup failure fails closed with a generic 404
    // — never a default tenant — so an unauthenticated caller cannot probe
    // which communities exist on this deployment.
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let tenant = match crate::tenant::bind_community(&state.db, raw_host).await {
        Ok(ctx) => ctx,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
                .into_response();
        }
    };
    let permit = match acquire_audio_connection_permit(&state.conn_semaphore) {
        Some(permit) => permit,
        None => {
            warn!("Connection limit reached, rejecting audio WebSocket");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "relay: connection limit reached",
            )
                .into_response();
        }
    };
    let corporate_identity_assertion =
        match crate::corporate_identity::identity_assertion_from_headers(
            &state,
            tenant.community(),
            &headers,
        ) {
            Ok(assertion) => assertion,
            Err(error) => {
                return (error.status_code(), error.public_message()).into_response();
            }
        };

    // Keep the parser boundary at the largest message this route accepts. The
    // checks in the receive loop still distinguish text from binary policy, but
    // they run after tungstenite has assembled a message.
    limit_audio_websocket(ws).on_upgrade(move |socket| {
        handle_audio_connection(
            socket,
            state,
            tenant,
            channel_id,
            permit,
            corporate_identity_assertion,
        )
    })
}

fn acquire_audio_connection_permit(
    conn_semaphore: &Arc<Semaphore>,
) -> Option<OwnedSemaphorePermit> {
    Arc::clone(conn_semaphore).try_acquire_owned().ok()
}

fn limit_audio_websocket<F>(ws: WebSocketUpgrade<F>) -> WebSocketUpgrade<F> {
    ws.max_message_size(MAX_WEBSOCKET_MESSAGE_BYTES)
        .max_frame_size(MAX_WEBSOCKET_MESSAGE_BYTES)
}

/// Highest huddle audio protocol version this relay understands. Clients are
/// allowed to negotiate any version in `1..=CURRENT_PROTOCOL_VERSION`; older
/// versions stay supported indefinitely for staged rollouts.
const CURRENT_PROTOCOL_VERSION: u8 = 2;

#[derive(Deserialize)]
struct AuthMsg {
    #[serde(rename = "type")]
    msg_type: String,
    event: nostr::Event,
    parent_channel_id: Option<Uuid>,
    /// Huddle audio protocol version requested by the client. Defaults to 1
    /// when missing so existing clients keep working without recompile. A
    /// room is pinned to whichever version its first peer requested; later
    /// peers must match or get `upgrade_required`.
    #[serde(default = "default_protocol_version")]
    protocol_version: u8,
}

fn default_protocol_version() -> u8 {
    1
}

async fn handle_audio_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    tenant: TenantContext,
    channel_id: Uuid,
    _permit: OwnedSemaphorePermit,
    corporate_identity_assertion: Option<crate::corporate_identity::IdentityAssertionInput>,
) {
    let cancel = CancellationToken::new();
    let community_id = tenant.community();
    let registry = Arc::clone(&state.community_connections);
    let check_state = Arc::clone(&state);
    let run_state = Arc::clone(&state);
    let session_id = Uuid::new_v4();
    run_registered_community_connection(
        &registry,
        session_id,
        community_id,
        cancel.clone(),
        move || async move { check_state.db.is_community_active(community_id).await },
        move || {
            handle_active_audio_connection(
                socket,
                run_state,
                tenant,
                channel_id,
                session_id,
                cancel,
                corporate_identity_assertion,
            )
        },
    )
    .await;
}

async fn handle_active_audio_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    tenant: TenantContext,
    channel_id: Uuid,
    session_id: Uuid,
    cancel: CancellationToken,
    corporate_identity_assertion: Option<crate::corporate_identity::IdentityAssertionInput>,
) {
    let Ok(session_target) = AuthorizationSessionTarget::new(session_id, Uuid::new_v4()) else {
        cancel.cancel();
        return;
    };
    let (mut ws_send, mut ws_recv) = socket.split();

    let challenge = generate_challenge();
    let challenge_msg =
        serde_json::json!({"type": "challenge", "challenge": challenge}).to_string();
    if ws_send
        .send(WsMessage::Text(challenge_msg.into()))
        .await
        .is_err()
    {
        return;
    }

    let auth_result = tokio::select! {
        biased;
        _ = cancel.cancelled() => return,
        result = tokio::time::timeout(AUTH_TIMEOUT, async {
            while let Some(Ok(msg)) = ws_recv.next().await {
                if let WsMessage::Text(text) = msg {
                    if text.len() > MAX_TEXT_FRAME_BYTES {
                        warn!("auth text frame too large — dropping");
                        continue;
                    }
                    if let Ok(auth) = serde_json::from_str::<AuthMsg>(&text) {
                        if auth.msg_type == "auth" {
                            return Some(auth);
                        }
                    }
                }
            }
            None
        }) => result,
    };

    let auth_msg = match auth_result {
        Ok(Some(a)) => a,
        _ => {
            debug!("audio auth timeout or disconnect");
            return;
        }
    };

    // Extract NIP-OA auth tag before verify_auth_event consumes the event.
    let auth_tag_json = crate::handlers::auth::extract_auth_tag_json(&auth_msg.event);
    let verified_event = auth_msg.event.clone();

    let relay_url = crate::api::bridge::nip42_expected_relay_url(&state.config.relay_url, &tenant);
    let auth_ctx = match state
        .auth
        .verify_auth_event(auth_msg.event, &challenge, &relay_url)
        .await
    {
        Ok(ctx) => ctx,
        Err(e) => {
            warn!("audio auth failed: {e}");
            let _ = ws_send
                .send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"auth failed"})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };

    let pubkey = auth_ctx.pubkey;
    let pubkey_hex = pubkey.to_hex();
    let pubkey_bytes = pubkey.to_bytes().to_vec();
    let parent_channel_id = auth_msg.parent_channel_id;

    let identity_lane =
        crate::authorization_runtime::transport::legacy_identity_lane(&state, tenant.community());
    let neutral_evidence = corporate_identity_assertion.is_none()
        && crate::authorization_runtime::transport::provider_evidence_resolver_is_installed(
            &state,
            tenant.community(),
        );
    let identity_proof = if neutral_evidence {
        None
    } else {
        match crate::corporate_identity::verify_corporate_identity(
            &state,
            tenant.community(),
            pubkey,
            corporate_identity_assertion.as_ref(),
            auth_tag_json.as_deref(),
        )
        .await
        {
            Ok(proof) => Some(proof),
            Err(e) => {
                warn!(error = ?e, "audio: corporate identity denied");
                if identity_lane
                    == crate::authorization_runtime::transport::LegacyIdentityLane::ObserveOnly
                {
                    None
                } else {
                    let _ = ws_send
                        .send(WsMessage::Text(
                            serde_json::json!({"type": "error", "message": e.public_message()})
                                .to_string()
                                .into(),
                        ))
                        .await;
                    return;
                }
            }
        }
    };

    if crate::api::relay_members::enforce_relay_membership(
        &state,
        tenant.community(),
        pubkey.as_bytes(),
        auth_tag_json.as_deref(),
    )
    .await
    .is_err()
    {
        warn!("audio: relay membership denied");
        let _ = ws_send
            .send(WsMessage::Text(
                serde_json::json!({"type": "error", "message": "restricted: not a relay member"})
                    .to_string()
                    .into(),
            ))
            .await;
        return;
    }

    let transport_delegation = crate::corporate_identity::verify_unconditional_nip_oa_relationship(
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
    let verified_proof = match VerifiedEvidenceAdapter::new().verify_nip42(
        tenant.community(),
        AuthTransport::Audio,
        &verified_event,
        &challenge,
        &relay_url,
        transport_delegation,
    ) {
        Ok(proof) => Arc::new(proof),
        Err(error) => {
            warn!(error = %error, "audio: sealed NIP-42 evidence denied");
            return;
        }
    };
    let proof_fingerprint = verified_proof.operation_binding().fingerprint();
    let mut correlation = [0_u8; 16];
    correlation.copy_from_slice(&proof_fingerprint[..16]);
    correlation[6] = (correlation[6] & 0x0f) | 0x50;
    correlation[8] = (correlation[8] & 0x3f) | 0x80;
    let verified_assertion = match identity_proof.as_ref() {
        Some(proof) => match crate::corporate_identity::current_verified_assertion_for_proof(
            &state,
            proof,
            tenant.community(),
            AuthTransport::Audio,
        ) {
            Ok(assertion) => assertion.map(Arc::new),
            Err(error) => {
                warn!(error = %error, "audio federated evidence denied");
                if identity_lane
                    == crate::authorization_runtime::transport::LegacyIdentityLane::ObserveOnly
                {
                    None
                } else {
                    return;
                }
            }
        },
        None => None,
    };
    let protected_authority = match authorize_exact_session_if_configured(
        &state,
        Arc::clone(&verified_proof),
        verified_assertion,
        AuthorizationCapability::AudioJoin,
        Uuid::from_bytes(correlation),
        "audio.join",
        session_target,
        cancel.clone(),
    )
    .await
    {
        Ok(authority) => Arc::new(authority),
        Err(error) => {
            warn!(error = %error, "audio: protected authorization denied");
            let _ = ws_send
                .send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"audio authorization denied"})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };
    if protected_authority.revalidate().is_err() {
        return;
    }

    // ── Step 3: membership check / auto-add ───────────────────────────────────
    let membership = match ensure_membership(
        &state,
        &tenant,
        channel_id,
        &pubkey_bytes,
        parent_channel_id,
        protected_authority.is_enforcing(),
    )
    .await
    {
        Ok(membership) => membership,
        Err(e) => {
            warn!("audio membership denied: {e}");
            let _ = send_protected_ws(
                &mut ws_send,
                WsMessage::Text(
                    serde_json::json!({"type":"error","message":"not a member"})
                        .to_string()
                        .into(),
                ),
                protected_authority.as_ref(),
            )
            .await;
            return;
        }
    };
    let parent_id_for_event = membership.lifecycle_parent_id();

    if protected_authority.revalidate().is_err() {
        return;
    }
    // Preserve the atomic private-huddle enrollment boundary.
    // Enforce never reaches this legacy auto-add path; observation lanes may
    // preserve legacy membership behavior but cannot mutate identity state.
    let deferred_private_admission = if let AudioMembership::LegacyAutoAdd { added_by, .. } =
        &membership
    {
        Some((
            added_by.clone(),
            if identity_lane == crate::authorization_runtime::transport::LegacyIdentityLane::Legacy
            {
                identity_proof
            } else {
                None
            },
        ))
    } else {
        if identity_lane == crate::authorization_runtime::transport::LegacyIdentityLane::Legacy {
            if let Some(identity_proof) = identity_proof {
                let identity_decision =
                    match crate::corporate_identity::finalize_corporate_identity(
                        &state,
                        tenant.community(),
                        pubkey,
                        identity_proof,
                    )
                    .await
                    {
                        Ok(decision) => decision,
                        Err(e) => {
                            warn!(error = ?e, "audio: corporate identity finalization denied");
                            let _ = ws_send
                                    .send(WsMessage::Text(
                                        serde_json::json!({"type": "error", "message": e.public_message()})
                                            .to_string()
                                            .into(),
                                    ))
                                    .await;
                            return;
                        }
                    };
                crate::corporate_identity::spawn_session_revalidation(
                    Arc::clone(&state),
                    tenant.community(),
                    pubkey,
                    identity_decision,
                    cancel.clone(),
                );
            }
        }
        None
    };
    let protected_admission_id = if protected_authority.is_enforcing() {
        match commit_existing_member_audio_admission(
            &state,
            &tenant,
            channel_id,
            &pubkey,
            &verified_proof,
            session_id,
            protected_authority.as_ref(),
        )
        .await
        {
            Ok(admission_id) => Some(admission_id),
            Err(_) => {
                let _ = send_protected_ws(
                    &mut ws_send,
                    WsMessage::Text(
                        serde_json::json!({
                            "type":"error",
                            "message":"audio authorization denied"
                        })
                        .to_string()
                        .into(),
                    ),
                    protected_authority.as_ref(),
                )
                .await;
                cancel.cancel();
                return;
            }
        }
    } else {
        None
    };
    let mut durable_audio_admission = match protected_admission_id {
        Some(admission_id) => {
            let Some(guard) = DurableAudioAdmissionGuard::new(
                &state,
                tenant.community(),
                admission_id,
                session_id,
            ) else {
                cancel.cancel();
                return;
            };
            Some(guard)
        }
        None => None,
    };
    let protected_deadline_schedule = if protected_authority.is_enforcing() {
        // Anchor monotonic time before consulting the injected authority clock;
        // clock sampling latency must consume, never extend, the lease.
        let monotonic_anchor = tokio::time::Instant::now();
        match (
            protected_authority.expires_at(),
            protected_authority.expiry_delay(),
        ) {
            (Some(deadline), Ok(Some(delay))) => {
                match ProtectedDeadlineSchedule::new_anchored(
                    deadline,
                    monotonic_anchor,
                    Some(delay),
                ) {
                    Ok(schedule) => Some(schedule),
                    Err(error) => {
                        warn!(?error, "audio: protected deadline unavailable");
                        cancel.cancel();
                        return;
                    }
                }
            }
            _ => {
                warn!("audio: protected deadline unavailable");
                cancel.cancel();
                return;
            }
        }
    } else {
        None
    };
    let protected_expiry_task = protected_deadline_schedule.map(|schedule| {
        let expiry_cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = expiry_cancel.cancelled() => {}
                _ = tokio::time::sleep_until(schedule.wake_at()) => {
                    expiry_cancel.cancel();
                }
            }
        })
    });
    let protected_revalidation_task = protected_authority.is_enforcing().then(|| {
        let authority = Arc::clone(&protected_authority);
        let revalidation_cancel = cancel.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = revalidation_cancel.cancelled() => break,
                    _ = interval.tick() => {
                        if authority.revalidate().is_err() {
                            revalidation_cancel.cancel();
                            break;
                        }
                    }
                }
            }
        })
    });
    // Huddle cross-pod routing (mesh) OR single-pod guardrail.
    //
    // When the mesh is live (`state.mesh()` is `Some`), a huddle can span pods:
    // Redis arbitrates ownership and this pod either owns the room locally or
    // forwards the client to the owner over a `HuddleControl` stream. When the
    // mesh is off, we keep today's behavior exactly — including the
    // `huddle_audio_available=false` rejection under a non-mesh horizontal
    // deployment (two peers on different pods would never hear each other).
    //
    // `remote_owner` is `Some` only on the non-owner path; it carries the
    // registration to the owner and, once the client is admitted locally, is
    // opened so its media forwards to the owner instead of fanning out locally.
    let mut pending_remote: Option<crate::audio::join::JoinOutcome> = None;
    // The freshly-acquired owner lease, if this connection won the CAS. Held
    // until `add_peer` succeeds, then installed in the owner registry so the
    // renewer's lifetime matches the room's, not this connection's failure
    // paths (archived channel, version reject, room full) which return early.
    let mut acquired_lease: Option<crate::audio::join::HuddleLease> = None;
    if protected_authority.revalidate().is_err() {
        cancel.cancel();
        return;
    }
    match state.mesh() {
        Some(mesh) => {
            if mesh.owners.is_draining() {
                let _ = ws_send
                    .send(WsMessage::Text(
                        serde_json::json!({
                            "type": "error",
                            "code": "huddle_relay_draining",
                            "message": "relay is draining; reconnect"
                        })
                        .to_string()
                        .into(),
                    ))
                    .await;
                return;
            }
            match crate::audio::join::resolve_join_owner_ready(
                &mesh.directory,
                tenant.community(),
                channel_id,
                mesh.local_runtime_id,
                &mesh.owners,
            )
            .await
            {
                Ok(resolved) => {
                    acquired_lease = resolved.acquired;
                    pending_remote = Some(resolved.outcome);
                }
                Err(e) => {
                    warn!("huddle join rejected by fence: {e}");
                    let _ = ws_send
                        .send(WsMessage::Text(
                            serde_json::json!({
                                "type": "error",
                                "code": "join_rejected",
                                "message": "huddle join rejected"
                            })
                            .to_string()
                            .into(),
                        ))
                        .await;
                    return;
                }
            }
        }
        None => {
            if !state.config.huddle_audio_available {
                debug!("huddle audio unavailable under horizontal scaling — rejecting join");
                let _ = ws_send
                    .send(WsMessage::Text(
                        serde_json::json!({
                            "type": "error",
                            "code": "huddle_audio_unavailable",
                            "message": "huddle audio unavailable in this deployment"
                        })
                        .to_string()
                        .into(),
                    ))
                    .await;
                return;
            }
        }
    }

    let room_owner_epoch = if protected_authority.is_enforcing() {
        state.mesh().zip(pending_remote).map(|(mesh, outcome)| {
            let fenced = outcome.fenced_header(channel_id, mesh.local_runtime_id);
            RoomOwnerEpoch::new(fenced.owner_runtime_id, fenced.generation)
        })
    } else {
        None
    };
    // Declared before the claim so reverse drop order releases the in-flight
    // claim before the exact retirement guard runs on every early return.
    let protected_room_retirement;
    let mut _protected_room_claim = None;
    let room = match room_owner_epoch {
        Some(epoch) => {
            let Some(mesh) = state.mesh() else {
                cancel.cancel();
                return;
            };
            match state.audio_rooms.get_or_create_for_owner(
                tenant.community(),
                channel_id,
                epoch,
                |old_epoch| {
                    if old_epoch.owner_runtime_id == mesh.local_runtime_id {
                        mesh.owners.release(channel_id, old_epoch.generation);
                    }
                },
            ) {
                Ok(claim) => {
                    let room = claim.room();
                    _protected_room_claim = Some(claim);
                    room
                }
                Err(error) => {
                    warn!(?error, "audio: owner room epoch unavailable");
                    release_pending_huddle_lease(&state, &mut acquired_lease).await;
                    cancel.cancel();
                    return;
                }
            }
        }
        None => state
            .audio_rooms
            .get_or_create(tenant.community(), channel_id),
    };
    protected_room_retirement = protected_authority.is_enforcing().then(|| {
        let mesh = state.mesh();
        ProtectedRoomRetirement {
            rooms: Arc::clone(&state.audio_rooms),
            community_id: tenant.community(),
            channel_id,
            room: Arc::clone(&room),
            owner_epoch: room_owner_epoch,
            owners: mesh.map(|mesh| Arc::clone(&mesh.owners)),
            local_runtime_id: mesh.map(|mesh| mesh.local_runtime_id),
        }
    });
    let cleanup_room_if_empty = || {
        protected_room_retirement.as_ref().map_or_else(
            || {
                state
                    .audio_rooms
                    .cleanup_if_empty(tenant.community(), channel_id)
            },
            ProtectedRoomRetirement::retire_if_empty,
        )
    };

    // Re-check archived status after obtaining the room. This closes the
    // cross-boundary race: a joiner that passed ensure_membership before
    // the last peer archived the channel could get a fresh room via
    // get_or_create (the old room was already cleaned up). This DB check
    // catches that case. The room-level ended flag (checked inside add_peer)
    // handles the same-room case.
    match state.db.get_channel(tenant.community(), channel_id).await {
        Ok(ch) if ch.archived_at.is_some() => {
            debug!("channel archived before room join");
            let _ = ws_send
                .send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"huddle has ended"})
                        .to_string()
                        .into(),
                ))
                .await;
            cleanup_room_if_empty();
            release_pending_huddle_lease(&state, &mut acquired_lease).await;
            return;
        }
        Err(e) => {
            warn!("pre-join channel check failed (fail-closed): {e}");
            cleanup_room_if_empty();
            release_pending_huddle_lease(&state, &mut acquired_lease).await;
            return;
        }
        Ok(_) => {} // Channel exists and is not archived — proceed.
    }

    // Reject unsupported future versions up-front so we don't accidentally
    // pin a room to a version we can't speak. Versions 1..=CURRENT are OK.
    let requested_version = auth_msg.protocol_version;
    if requested_version == 0 || requested_version > CURRENT_PROTOCOL_VERSION {
        warn!(
            requested_version,
            current = CURRENT_PROTOCOL_VERSION,
            "audio: client requested unsupported protocol version"
        );
        let _ = ws_send
            .send(WsMessage::Text(
                serde_json::json!({
                    "type": "error",
                    "code": "unsupported_version",
                    "message": format!(
                        "huddle audio protocol v{requested_version} not supported; relay max is v{CURRENT_PROTOCOL_VERSION}"
                    ),
                    "current_version": CURRENT_PROTOCOL_VERSION,
                })
                .to_string()
                .into(),
            ))
            .await;
        release_pending_huddle_lease(&state, &mut acquired_lease).await;
        return;
    }

    // Remote registration happens before ingress admission. The owner-assigned
    // index is therefore the only index this client ever has; no frame or
    // `joined` message can escape with an ingress-local placeholder.
    let mut pending_remote_session: Option<crate::audio::join::PendingRemoteHuddleSession> = None;
    let mut remote_session: Option<crate::audio::join::RemoteHuddleSession> = None;
    let mut remote_stream: Option<buzz_relay_mesh::MeshStream> = None;
    let mut remote_fence: Option<Arc<crate::audio::mesh::GenerationFloor>> = None;
    if let (Some(mesh), Some(crate::audio::join::JoinOutcome::RemoteOwner { .. })) =
        (state.mesh(), pending_remote)
    {
        let outcome = pending_remote.expect("RemoteOwner matched above");
        let fenced = outcome.fenced_header(channel_id, mesh.local_runtime_id);
        let crate::audio::join::JoinOutcome::RemoteOwner {
            owner_runtime_id, ..
        } = outcome
        else {
            unreachable!("matched RemoteOwner above");
        };
        let dial = {
            let dial_future = async {
                if let Some(admission_id) = protected_admission_id {
                    crate::audio::join::reserve_remote_owner(
                        Arc::clone(&mesh.transport),
                        mesh.local_runtime_id,
                        owner_runtime_id,
                        fenced,
                        crate::audio::join::RemoteReservationRequest {
                            community_id: tenant.community(),
                            admission_id,
                            pubkey: pubkey_hex.clone(),
                            protocol_version: requested_version,
                        },
                    )
                    .await
                    .map(|(pending, stream)| (Some(pending), None, stream))
                } else {
                    crate::audio::join::dial_remote_owner(
                        Arc::clone(&mesh.transport),
                        mesh.local_runtime_id,
                        owner_runtime_id,
                        fenced,
                        tenant.community(),
                        pubkey_hex.clone(),
                        requested_version,
                    )
                    .await
                    .map(|(session, stream)| (None, Some(session), stream))
                }
            };
            tokio::pin!(dial_future);
            tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                result = &mut dial_future => Some(result),
            }
        };
        let Some(dial) = dial else {
            cleanup_room_if_empty();
            release_pending_huddle_lease(&state, &mut acquired_lease).await;
            return;
        };
        match dial {
            Ok((pending, session, stream)) => {
                pending_remote_session = pending;
                remote_session = session;
                remote_stream = Some(stream);
                remote_fence = Some(Arc::clone(&mesh.audio_fence));
            }
            Err(crate::audio::join::DialError::Rejected(reason)) => {
                warn!("huddle owner rejected registration: {reason:?}");
                let _ = ws_send
                    .send(WsMessage::Text(
                        remote_rejection_ws_error(&reason).to_string().into(),
                    ))
                    .await;
                cleanup_room_if_empty();
                release_pending_huddle_lease(&state, &mut acquired_lease).await;
                return;
            }
            Err(crate::audio::join::DialError::Mesh(e)) => {
                warn!("huddle owner registration failed: {e}");
                let _ = ws_send
                    .send(WsMessage::Text(
                        serde_json::json!({
                            "type": "error", "code": "huddle_owner_unreachable",
                            "message": "could not reach the huddle owner"
                        })
                        .to_string()
                        .into(),
                    ))
                    .await;
                cleanup_room_if_empty();
                release_pending_huddle_lease(&state, &mut acquired_lease).await;
                return;
            }
        }
    }

    if protected_authority.revalidate().is_err() {
        if let (Some(pending), Some(stream)) =
            (pending_remote_session.as_ref(), remote_stream.as_mut())
        {
            crate::audio::join::abort_remote_owner(
                stream,
                pending.fenced(),
                pending.admission_id(),
            )
            .await;
        }
        release_pending_huddle_lease(&state, &mut acquired_lease).await;
        cancel.cancel();
        return;
    }
    if protected_admission_id.is_none() {
        if let (Some(mesh), Some(outcome)) = (state.mesh(), pending_remote) {
            if crate::audio::join::validate_join_before_visibility(
                &mesh.directory,
                tenant.community(),
                channel_id,
                mesh.local_runtime_id,
                outcome,
            )
            .await
            .is_err()
            {
                if let (Some(session), Some(stream)) =
                    (remote_session.as_ref(), remote_stream.as_mut())
                {
                    crate::audio::join::send_remote_close(stream, session).await;
                }
                release_pending_huddle_lease(&state, &mut acquired_lease).await;
                cancel.cancel();
                return;
            }
        }
    }
    let protected_effects =
        protected_admission_id.map(|_| ProtectedPeerEffects::new(cancel.clone()));
    let admission = if let Some(admission_id) = protected_admission_id {
        let local_reservation = if let Some(pending) = pending_remote_session.as_ref() {
            room.reserve_peer_at_index(
                admission_id,
                pubkey_hex.clone(),
                requested_version,
                pending.peer_index(),
            )
        } else {
            room.reserve_peer(admission_id, pubkey_hex.clone(), requested_version)
        };
        match local_reservation {
            Ok(local_reservation) => {
                if protected_authority.revalidate().is_err() || cancel.is_cancelled() {
                    if let (Some(pending), Some(stream)) =
                        (pending_remote_session.as_ref(), remote_stream.as_mut())
                    {
                        crate::audio::join::abort_remote_owner(
                            stream,
                            pending.fenced(),
                            pending.admission_id(),
                        )
                        .await;
                    }
                    drop(local_reservation);
                    release_pending_huddle_lease(&state, &mut acquired_lease).await;
                    cancel.cancel();
                    return;
                }
                // The PostgreSQL activation is itself a fresh transaction-owned
                // authorization commit. The durable visibility transition also
                // precedes every remote-owner or local room effect. It is an
                // authorization for the attachment attempt, not evidence that a
                // peer was published; every later failure compensates it.
                if let Some(receipt) = durable_audio_admission.as_mut() {
                    if receipt
                        .activate(
                            &state,
                            protected_authority.as_ref(),
                            channel_id,
                            &pubkey.to_bytes(),
                        )
                        .await
                        .is_err()
                    {
                        if let (Some(pending), Some(stream)) =
                            (pending_remote_session.as_ref(), remote_stream.as_mut())
                        {
                            crate::audio::join::abort_remote_owner(
                                stream,
                                pending.fenced(),
                                pending.admission_id(),
                            )
                            .await;
                        }
                        drop(local_reservation);
                        release_pending_huddle_lease(&state, &mut acquired_lease).await;
                        cancel.cancel();
                        return;
                    }
                    if let Err(error) = receipt.mark_visible().await {
                        warn!(%error, "audio: durable peer visibility could not be witnessed");
                        if let (Some(pending), Some(stream)) =
                            (pending_remote_session.as_ref(), remote_stream.as_mut())
                        {
                            crate::audio::join::abort_remote_owner(
                                stream,
                                pending.fenced(),
                                pending.admission_id(),
                            )
                            .await;
                        }
                        drop(local_reservation);
                        release_pending_huddle_lease(&state, &mut acquired_lease).await;
                        cancel.cancel();
                        return;
                    }
                }
                if let Some(pending) = pending_remote_session.take() {
                    let fenced = pending.fenced();
                    let remote_admission_id = pending.admission_id();
                    let stream = remote_stream
                        .as_mut()
                        .expect("remote reservation owns its control stream");
                    let activation = {
                        let activation_future =
                            crate::audio::join::activate_remote_owner(&pending, stream);
                        tokio::pin!(activation_future);
                        tokio::select! {
                            biased;
                            _ = cancel.cancelled() => None,
                            result = &mut activation_future => Some(result),
                        }
                    };
                    let Some(activation) = activation else {
                        crate::audio::join::abort_remote_owner(stream, fenced, remote_admission_id)
                            .await;
                        drop(local_reservation);
                        release_pending_huddle_lease(&state, &mut acquired_lease).await;
                        return;
                    };
                    if let Err(error) = activation {
                        crate::audio::join::abort_remote_owner(stream, fenced, remote_admission_id)
                            .await;
                        drop(local_reservation);
                        release_pending_huddle_lease(&state, &mut acquired_lease).await;
                        warn!(?error, "huddle owner activation failed");
                        cancel.cancel();
                        return;
                    }
                    if protected_authority.revalidate().is_err() || cancel.is_cancelled() {
                        crate::audio::join::abort_remote_owner(stream, fenced, remote_admission_id)
                            .await;
                        drop(local_reservation);
                        release_pending_huddle_lease(&state, &mut acquired_lease).await;
                        cancel.cancel();
                        return;
                    }
                    let attachment_context = crate::audio::join::protected_audio_attachment_context(
                        tenant.community(),
                        channel_id,
                        remote_admission_id,
                        &pubkey_hex,
                    );
                    let authority_token =
                        match crate::authorization_runtime::ephemeral::seal_context(
                            &state,
                            protected_authority.as_ref(),
                            attachment_context,
                        ) {
                            Ok(token) => token,
                            Err(error) => {
                                crate::audio::join::abort_remote_owner(
                                    stream,
                                    fenced,
                                    remote_admission_id,
                                )
                                .await;
                                drop(local_reservation);
                                release_pending_huddle_lease(&state, &mut acquired_lease).await;
                                warn!(?error, "huddle authority sealing failed");
                                cancel.cancel();
                                return;
                            }
                        };
                    let confirmation = {
                        let confirmation_future = crate::audio::join::confirm_remote_owner(
                            pending,
                            stream,
                            authority_token,
                        );
                        tokio::pin!(confirmation_future);
                        tokio::select! {
                            biased;
                            _ = cancel.cancelled() => None,
                            result = &mut confirmation_future => Some(result),
                        }
                    };
                    let Some(confirmation) = confirmation else {
                        crate::audio::join::abort_remote_owner(stream, fenced, remote_admission_id)
                            .await;
                        drop(local_reservation);
                        release_pending_huddle_lease(&state, &mut acquired_lease).await;
                        return;
                    };
                    match confirmation {
                        Ok(session) => {
                            remote_session = Some(session);
                            if let Some(receipt) = durable_audio_admission.as_mut() {
                                receipt.mark_published();
                            }
                        }
                        Err(error) => {
                            crate::audio::join::abort_remote_owner(
                                stream,
                                fenced,
                                remote_admission_id,
                            )
                            .await;
                            drop(local_reservation);
                            release_pending_huddle_lease(&state, &mut acquired_lease).await;
                            warn!(?error, "huddle owner confirmation failed");
                            cancel.cancel();
                            return;
                        }
                    }
                }
                if protected_authority.revalidate().is_err() || cancel.is_cancelled() {
                    if let (Some(session), Some(stream)) =
                        (remote_session.as_ref(), remote_stream.as_mut())
                    {
                        crate::audio::join::send_remote_close(stream, session).await;
                    }
                    drop(local_reservation);
                    release_pending_huddle_lease(&state, &mut acquired_lease).await;
                    cancel.cancel();
                    return;
                }
                if let (Some(mesh), Some(outcome)) = (state.mesh(), pending_remote) {
                    if crate::audio::join::validate_join_before_visibility(
                        &mesh.directory,
                        tenant.community(),
                        channel_id,
                        mesh.local_runtime_id,
                        outcome,
                    )
                    .await
                    .is_err()
                    {
                        if let (Some(session), Some(stream)) =
                            (remote_session.as_ref(), remote_stream.as_mut())
                        {
                            crate::audio::join::send_remote_close(stream, session).await;
                        }
                        drop(local_reservation);
                        release_pending_huddle_lease(&state, &mut acquired_lease).await;
                        cancel.cancel();
                        return;
                    }
                }
                // Redis validation above is asynchronous. Re-check the exact
                // PostgreSQL-authorized attempt after it completes and before
                // the synchronous visibility transition.
                if protected_authority.revalidate().is_err() || cancel.is_cancelled() {
                    if let (Some(session), Some(stream)) =
                        (remote_session.as_ref(), remote_stream.as_mut())
                    {
                        crate::audio::join::send_remote_close(stream, session).await;
                    }
                    drop(local_reservation);
                    release_pending_huddle_lease(&state, &mut acquired_lease).await;
                    cancel.cancel();
                    return;
                }
                // The PostgreSQL check above follows an awaited Redis fence;
                // validate the exact owner fence once more, then finish with
                // a synchronous PostgreSQL/expiry check before activation.
                if let (Some(mesh), Some(outcome)) = (state.mesh(), pending_remote) {
                    if crate::audio::join::validate_join_before_visibility(
                        &mesh.directory,
                        tenant.community(),
                        channel_id,
                        mesh.local_runtime_id,
                        outcome,
                    )
                    .await
                    .is_err()
                    {
                        if let (Some(session), Some(stream)) =
                            (remote_session.as_ref(), remote_stream.as_mut())
                        {
                            crate::audio::join::send_remote_close(stream, session).await;
                        }
                        drop(local_reservation);
                        release_pending_huddle_lease(&state, &mut acquired_lease).await;
                        cancel.cancel();
                        return;
                    }
                }
                if protected_authority.revalidate().is_err() || cancel.is_cancelled() {
                    if let (Some(session), Some(stream)) =
                        (remote_session.as_ref(), remote_stream.as_mut())
                    {
                        crate::audio::join::send_remote_close(stream, session).await;
                    }
                    drop(local_reservation);
                    release_pending_huddle_lease(&state, &mut acquired_lease).await;
                    cancel.cancel();
                    return;
                }
                if let Some(receipt) = durable_audio_admission.as_ref() {
                    if !receipt
                        .is_current(&state, channel_id, &pubkey.to_bytes())
                        .await
                    {
                        if let (Some(session), Some(stream)) =
                            (remote_session.as_ref(), remote_stream.as_mut())
                        {
                            crate::audio::join::send_remote_close(stream, session).await;
                        }
                        drop(local_reservation);
                        release_pending_huddle_lease(&state, &mut acquired_lease).await;
                        cancel.cancel();
                        return;
                    }
                }
                let activated = match protected_deadline_schedule {
                    Some(schedule) => local_reservation.activate_protected_with_effects_if(
                        schedule,
                        protected_effects
                            .clone()
                            .expect("protected admission has exact effects"),
                        || protected_authority.revalidate().is_ok() && !cancel.is_cancelled(),
                    ),
                    // A protected V1 admission without a finite deadline cannot
                    // be scheduled for proactive closure and therefore fails
                    // closed instead of falling back to legacy activation.
                    None => Ok(None),
                };
                match activated {
                    Ok(Some((activated, epoch))) => {
                        if remote_session.is_none() {
                            if let Some(receipt) = durable_audio_admission.as_mut() {
                                receipt.mark_published();
                            }
                        }
                        if protected_authority.revalidate().is_err() || cancel.is_cancelled() {
                            room.remove_protected_epoch(epoch);
                            if let (Some(session), Some(stream)) =
                                (remote_session.as_ref(), remote_stream.as_mut())
                            {
                                crate::audio::join::send_remote_close(stream, session).await;
                            }
                            release_pending_huddle_lease(&state, &mut acquired_lease).await;
                            cancel.cancel();
                            return;
                        }
                        Ok((activated, Some(epoch)))
                    }
                    Ok(None) => {
                        if let (Some(session), Some(stream)) =
                            (remote_session.as_ref(), remote_stream.as_mut())
                        {
                            crate::audio::join::send_remote_close(stream, session).await;
                        }
                        release_pending_huddle_lease(&state, &mut acquired_lease).await;
                        cancel.cancel();
                        return;
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    } else if let Some(session) = remote_session.as_ref() {
        room.add_peer_at_index(pubkey_hex.clone(), requested_version, session.peer_index())
            .map(|(id, audio, ctrl)| ((id, session.peer_index(), audio, ctrl), None))
    } else {
        room.add_peer(pubkey_hex.clone(), requested_version)
            .map(|activated| (activated, None))
    };
    let ((peer_id, peer_index, audio_rx, peer_ctrl_rx), protected_peer_epoch) = match admission {
        Ok(v) => v,
        Err(crate::audio::room::AdmissionError::Full) => {
            warn!("audio room full (255 peers exhausted)");
            let _ = ws_send.send(WsMessage::Text(serde_json::json!({"type":"error","code":"room_full","message":"peer index space exhausted"}).to_string().into())).await;
            if let (Some(session), Some(stream)) = (remote_session.as_ref(), remote_stream.as_mut())
            {
                crate::audio::join::send_remote_close(stream, session).await;
            } else if let (Some(pending), Some(stream)) =
                (pending_remote_session.as_ref(), remote_stream.as_mut())
            {
                crate::audio::join::abort_remote_owner(
                    stream,
                    pending.fenced(),
                    pending.admission_id(),
                )
                .await;
            }
            release_pending_huddle_lease(&state, &mut acquired_lease).await;
            return;
        }
        Err(crate::audio::room::AdmissionError::Ended) => {
            debug!("room ended before admission");
            let _ = ws_send.send(WsMessage::Text(serde_json::json!({"type":"error","code":"room_ended","message":"huddle has ended"}).to_string().into())).await;
            if let (Some(session), Some(stream)) = (remote_session.as_ref(), remote_stream.as_mut())
            {
                crate::audio::join::send_remote_close(stream, session).await;
            } else if let (Some(pending), Some(stream)) =
                (pending_remote_session.as_ref(), remote_stream.as_mut())
            {
                crate::audio::join::abort_remote_owner(
                    stream,
                    pending.fenced(),
                    pending.admission_id(),
                )
                .await;
            }
            release_pending_huddle_lease(&state, &mut acquired_lease).await;
            return;
        }
        Err(crate::audio::room::AdmissionError::VersionMismatch { pinned, requested }) => {
            info!(
                pinned,
                requested, "audio: protocol version mismatch — upgrade required"
            );
            let _ = ws_send.send(WsMessage::Text(serde_json::json!({
                "type": "error", "code": "upgrade_required",
                "message": format!("this huddle is using audio protocol v{pinned}; your client requested v{requested}"),
                "pinned_version": pinned, "requested_version": requested,
            }).to_string().into())).await;
            if let (Some(session), Some(stream)) = (remote_session.as_ref(), remote_stream.as_mut())
            {
                crate::audio::join::send_remote_close(stream, session).await;
            } else if let (Some(pending), Some(stream)) =
                (pending_remote_session.as_ref(), remote_stream.as_mut())
            {
                crate::audio::join::abort_remote_owner(
                    stream,
                    pending.fenced(),
                    pending.admission_id(),
                )
                .await;
            }
            release_pending_huddle_lease(&state, &mut acquired_lease).await;
            return;
        }
    };

    if let Some((added_by, identity_proof)) = deferred_private_admission {
        debug_assert!(!protected_authority.is_enforcing());
        debug_assert!(protected_peer_epoch.is_none());
        let identity_input = identity_proof
            .as_ref()
            .and_then(|proof| crate::corporate_identity::binding_input_for_proof(proof, &pubkey));
        let outcome = state
            .db
            .add_member_with_identity(
                tenant.community(),
                channel_id,
                &pubkey_bytes,
                MemberRole::Member,
                Some(&added_by),
                identity_input.as_ref(),
            )
            .await;
        let committed_binding = match outcome {
            Ok(buzz_db::channel::ChannelAdmissionOutcome::Joined {
                identity_binding, ..
            }) => identity_binding,
            Ok(buzz_db::channel::ChannelAdmissionOutcome::IdentityConflict(conflict)) => Some(
                buzz_db::identity_binding::BindIdentityResult::Conflict(conflict),
            ),
            Ok(buzz_db::channel::ChannelAdmissionOutcome::IdentityRevoked) => {
                Some(buzz_db::identity_binding::BindIdentityResult::Revoked)
            }
            Ok(buzz_db::channel::ChannelAdmissionOutcome::IdentityBindingRequired) => {
                Some(buzz_db::identity_binding::BindIdentityResult::BindingRequired)
            }
            Err(e) => {
                warn!("audio membership auto-add failed: {e}");
                let _ = ws_send
                    .send(WsMessage::Text(
                        serde_json::json!({"type":"error","message":"not a member"})
                            .to_string()
                            .into(),
                    ))
                    .await;
                if let (Some(session), Some(stream)) =
                    (remote_session.as_ref(), remote_stream.as_mut())
                {
                    crate::audio::join::send_remote_close(stream, session).await;
                }
                room.remove_peer(peer_id);
                cleanup_room_if_empty();
                release_pending_huddle_lease(&state, &mut acquired_lease).await;
                cancel.cancel();
                return;
            }
        };
        if let Some(identity_proof) = identity_proof {
            let identity_decision =
                match crate::corporate_identity::finalize_atomic_corporate_identity_result(
                    &state,
                    tenant.community(),
                    pubkey,
                    identity_proof,
                    committed_binding,
                )
                .await
                {
                    Ok(decision) => decision,
                    Err(e) => {
                        warn!(error = ?e, "audio: corporate identity finalization denied");
                        let _ = ws_send
                            .send(WsMessage::Text(
                                serde_json::json!({"type": "error", "message": e.public_message()})
                                    .to_string()
                                    .into(),
                            ))
                            .await;
                        if let (Some(session), Some(stream)) =
                            (remote_session.as_ref(), remote_stream.as_mut())
                        {
                            crate::audio::join::send_remote_close(stream, session).await;
                        }
                        room.remove_peer(peer_id);
                        cleanup_room_if_empty();
                        release_pending_huddle_lease(&state, &mut acquired_lease).await;
                        cancel.cancel();
                        return;
                    }
                };
            crate::corporate_identity::spawn_session_revalidation(
                Arc::clone(&state),
                tenant.community(),
                pubkey,
                identity_decision,
                cancel.clone(),
            );
        }
        state.invalidate_membership(&tenant, channel_id, &pubkey_bytes);
    }

    let _protected_active_peer = protected_peer_epoch.map(|epoch| ProtectedActivePeerGuard {
        room: Arc::downgrade(&room),
        epoch,
    });
    if let (Some(session), Some(epoch)) = (remote_session.as_mut(), protected_peer_epoch) {
        session.bind_local_epoch(epoch);
    }

    // A non-owner pod accepts realtime fan-out only from the authenticated
    // owner and generation established by this reliable control attachment.
    let remote_media_attachment = remote_session.as_ref().and_then(|session| {
        state.mesh().map(|mesh| {
            mesh.audio_attachments.register_owner_fanout(
                session.fenced(),
                session.admission_id().unwrap_or(peer_id),
                protected_authority.expires_at().unwrap_or(u64::MAX),
            )
        })
    });
    let mut _legacy_remote_media_attachment = None;
    if let Some(media_attachment) = remote_media_attachment {
        if let Some(effects) = protected_effects.as_ref() {
            if !effects.install_revoker(move || drop(media_attachment)) {
                if let Some(epoch) = protected_peer_epoch {
                    room.remove_protected_epoch(epoch);
                } else {
                    room.remove_peer(peer_id);
                }
                if let (Some(session), Some(stream)) =
                    (remote_session.as_ref(), remote_stream.as_mut())
                {
                    crate::audio::join::send_remote_close(stream, session).await;
                }
                cancel.cancel();
                return;
            }
        } else {
            _legacy_remote_media_attachment = Some(media_attachment);
        }
    }

    info!(peer_index, "audio peer joined");

    // Owner path: install (or reuse) this room's single lease renewer now that
    // a peer is admitted, and capture its owner-loss signal. The connection
    // that won the CAS holds `acquired_lease`; it installs the renewer. A
    // steady-state owner (an earlier joiner installed it) reuses the room's
    // existing signal. `owner_lost` drives this connection's own teardown
    // below; `owner_generation` fences the release on room-empty so a stale
    // teardown cannot release a newer epoch a re-acquire installed.
    //
    // The reuse arm's live entry is guaranteed by `resolve_join_owner_ready`:
    // it re-resolves until the CAS winner has installed (reuse) or a fresh CAS
    // wins (acquire), never returning a `LocalOwner` snapshot with a missing
    // registry entry. So a local owner peer here always gets a real `lost`
    // watcher — the ownerless split-brain (an owner peer fanning stale media
    // with no way to observe lease loss, since local WS peers have no per-frame
    // fence) cannot occur. A `None` on the reuse arm is therefore an invariant
    // violation, not a benign race; log it loudly rather than proceed silently.
    let mut owner_lost: Option<CancellationToken> = None;
    let mut owner_draining: Option<CancellationToken> = None;
    let mut owner_generation: Option<u64> = None;
    if let Some(mesh) = state.mesh() {
        match (pending_remote, acquired_lease.take()) {
            (Some(crate::audio::join::JoinOutcome::LocalOwner { generation }), Some(lease)) => {
                let signals =
                    mesh.owners
                        .attach_signals(channel_id, Arc::new(mesh.directory.clone()), lease);
                owner_lost = Some(signals.lost);
                owner_draining = Some(signals.draining);
                owner_generation = Some(generation);
            }
            (Some(crate::audio::join::JoinOutcome::LocalOwner { generation }), None) => {
                owner_lost = mesh.owners.lost_for(channel_id);
                owner_draining = mesh.owners.drain_for(channel_id);
                owner_generation = Some(generation);
                if owner_lost.is_none() {
                    error!(
                        "huddle owner-ready invariant violated: LocalOwner reuse with no live \
                         registry entry after resolve_join_owner_ready — owner peer has no \
                         lease-loss watcher"
                    );
                }
            }
            _ => {}
        }
    }

    // Remote registration and owner-assigned ingress admission completed above.

    let peers_snapshot: Vec<serde_json::Value> = if let Some(session) = remote_session.as_ref() {
        session
            .roster()
            .peers
            .iter()
            .map(|peer| serde_json::json!({"pubkey": peer.pubkey, "peer_index": peer.peer_index}))
            .collect()
    } else {
        room.peer_pubkeys()
            .into_iter()
            .map(|(pk, idx)| serde_json::json!({"pubkey": pk, "peer_index": idx}))
            .collect()
    };

    let joined_msg = serde_json::json!({
        "type": "joined",
        "pubkey": pubkey_hex,
        "peer_index": peer_index,
        "peers": peers_snapshot,
    })
    .to_string();

    if protected_authority.revalidate().is_err() {
        cancel.cancel();
        if let Some(epoch) = protected_peer_epoch {
            room.remove_protected_epoch(epoch);
        } else {
            room.remove_peer(peer_id);
        }
        if let (Some(session), Some(stream)) = (remote_session.as_ref(), remote_stream.as_mut()) {
            crate::audio::join::send_remote_close(stream, session).await;
        }
        if cleanup_room_if_empty() {
            if let (Some(mesh), Some(generation)) = (state.mesh(), owner_generation) {
                mesh.owners.release(channel_id, generation);
            }
        }
        return;
    }
    if remote_session.is_some() {
        let joined_sent = if let Some(epoch) = protected_peer_epoch {
            let ready = poll_fn(|context| Pin::new(&mut ws_send).poll_ready(context)).await;
            if ready.is_err() {
                false
            } else {
                let publication = room.publish_protected_join_if_current(
                    epoch,
                    &pubkey_hex,
                    peer_index,
                    |_pubkey, _peer_index, _snapshot| {
                        Pin::new(&mut ws_send)
                            .start_send(WsMessage::Text(joined_msg.clone().into()))
                            .ok()
                    },
                );
                publication.is_some() && ws_send.flush().await.is_ok()
            }
        } else {
            send_protected_ws(
                &mut ws_send,
                WsMessage::Text(joined_msg.clone().into()),
                protected_authority.as_ref(),
            )
            .await
        };
        if !joined_sent {
            cancel.cancel();
            if let Some(epoch) = protected_peer_epoch {
                room.remove_protected_epoch(epoch);
            } else {
                room.remove_peer(peer_id);
            }
            if let (Some(session), Some(stream)) = (remote_session.as_ref(), remote_stream.as_mut())
            {
                crate::audio::join::send_remote_close(stream, session).await;
            }
            let room_emptied = cleanup_room_if_empty();
            if room_emptied {
                if let (Some(mesh), Some(generation)) = (state.mesh(), owner_generation) {
                    mesh.owners.release(channel_id, generation);
                }
            }
            return;
        }
    } else if let Some(epoch) = protected_peer_epoch {
        if room
            .broadcast_protected_join_if_current(epoch, &pubkey_hex, peer_index)
            .is_none()
        {
            cancel.cancel();
            room.remove_protected_epoch(epoch);
            cleanup_room_if_empty();
            return;
        }
    } else {
        room.broadcast_control(joined_msg);
    }

    // ── Step 6: emit kind:48101 (PARTICIPANT_JOINED) ──────────────────────────
    if !protected_authority.is_enforcing() {
        emit_participant_event(
            &state,
            &tenant,
            Kind::Custom(48101),
            channel_id,
            parent_id_for_event,
            &pubkey_hex,
        )
        .await;
    }

    let missed_pongs = Arc::new(AtomicU8::new(0));

    // Dual-channel pattern (matches connection.rs): data channel for audio,
    // control channel for Ping/Pong/Close/control JSON with priority drain.
    let (data_tx, data_rx) = mpsc::channel::<WsMessage>(16);
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<WsMessage>(8);

    let send_cancel = cancel.child_token();
    let send_task = tokio::spawn(send_loop(
        ws_send,
        data_rx,
        ctrl_rx,
        send_cancel,
        Arc::clone(&protected_authority),
    ));

    let hb_cancel = cancel.clone();
    let hb_missed = Arc::clone(&missed_pongs);
    let heartbeat_task = tokio::spawn(heartbeat_loop(ctrl_tx.clone(), hb_missed, hb_cancel));

    let fwd_cancel = cancel.child_token();
    let forward_task = tokio::spawn(audio_forward_loop(
        audio_rx,
        peer_ctrl_rx,
        data_tx,
        ctrl_tx.clone(),
        fwd_cancel,
        Arc::clone(&protected_authority),
    ));

    // Non-owner path: own the owner's `HuddleControl` stream in a reader task.
    // It races the owner's teardown signal against our own cancellation:
    //   * owner speaks first (`Goodbye` / stream close) → tear the client down
    //     and close its WS so it rejoins (against a fresh owner/generation),
    //     and forget the local generation floor so the rejoin isn't fenced by
    //     the dead session. Redis remains the ownership arbiter; forgetting the
    //     floor only clears local stale-frame suppression.
    //   * we cancel first (client left / heartbeat death) → send the clean
    //     `UnregisterPeer` + `Goodbye(SessionEnded)` so the owner drops us.
    let reader_task = remote_stream.map(|mut stream| {
        let reader_cancel = cancel.clone();
        let fence = remote_fence.expect("remote_fence set whenever remote_stream is");
        let fenced = remote_session
            .as_ref()
            .expect("remote_session set whenever remote_stream is")
            .fenced();
        let pubkey = remote_session
            .as_ref()
            .expect("remote_session set whenever remote_stream is")
            .pubkey()
            .to_string();
        let roster_revision = remote_session
            .as_ref()
            .expect("remote_session set whenever remote_stream is")
            .roster()
            .revision;
        let admission_id = remote_session
            .as_ref()
            .expect("remote_session set whenever remote_stream is")
            .admission_id();
        let roster_ctrl_tx = ctrl_tx.clone();
        tokio::spawn(async move {
            tokio::select! {
                cause = crate::audio::join::read_owner_control(
                    &mut stream,
                    fenced,
                    roster_revision,
                    &roster_ctrl_tx,
                ) => {
                    teardown_remote_huddle(cause, channel_id, &reader_cancel, &fence);
                }
                _ = reader_cancel.cancelled() => {
                    if let Some(admission_id) = admission_id {
                        crate::audio::join::abort_remote_owner(
                            &mut stream,
                            fenced,
                            admission_id,
                        ).await;
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_millis(250),
                            stream.send_frame(buzz_relay_mesh::MeshStreamFrame::Goodbye {
                                fenced,
                                reason: crate::audio::join::HUDDLE_SESSION_ENDED,
                            }),
                        ).await;
                        let _ = stream.finish();
                    } else {
                        crate::audio::join::send_clean_close(&mut stream, fenced, &pubkey).await;
                    }
                }
            }
        })
    });

    // Owner path: watch the room's owner-loss / owner-drain signals. Fenced loss
    // and intentional drain both close local owner clients for rejoin and forget
    // the local generation floor so the fresh generation is accepted. The cause
    // distinction is carried on the remote control streams; locally the action
    // is the same WS teardown. Silent on ordinary client leave.
    let owner_teardown_task = if owner_lost.is_some() || owner_draining.is_some() {
        let fence = Arc::clone(
            &state
                .mesh()
                .expect("owner teardown watcher only exists when mesh owner state exists")
                .audio_fence,
        );
        let owner_cancel = cancel.clone();
        Some(tokio::spawn(async move {
            let lost_fired = async {
                match &owner_lost {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            };
            let drain_fired = async {
                match &owner_draining {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = drain_fired => {
                    info!(
                        "huddle owner is draining — closing local client for rejoin"
                    );
                    owner_cancel.cancel();
                    fence.forget(channel_id);
                }
                _ = lost_fired => {
                    info!(
                        "huddle owner lost its lease — closing local client for rejoin"
                    );
                    owner_cancel.cancel();
                    fence.forget(channel_id);
                }
                _ = owner_cancel.cancelled() => {}
            }
        }))
    } else {
        None
    };

    recv_loop(
        ws_recv,
        Arc::clone(&room),
        peer_id,
        requested_version,
        ctrl_tx,
        Arc::clone(&missed_pongs),
        cancel.clone(),
        Arc::clone(&protected_authority),
        remote_session.as_mut(),
    )
    .await;

    cancel.cancel();
    let _ = send_task.await;
    let _ = heartbeat_task.await;
    let _ = forward_task.await;
    if let Some(expiry_task) = protected_expiry_task {
        let _ = expiry_task.await;
    }
    if let Some(revalidation_task) = protected_revalidation_task {
        let _ = revalidation_task.await;
    }
    // The reader task owns the owner control stream; joining it here guarantees
    // its clean-close (or teardown) completes before connection cleanup returns.
    if let Some(reader_task) = reader_task {
        let _ = reader_task.await;
    }
    // The owner teardown watcher is cancelled by `cancel.cancel()` above (or has
    // already fired); join it so it settles before cleanup.
    if let Some(owner_teardown_task) = owner_teardown_task {
        let _ = owner_teardown_task.await;
    }

    // Atomic owner remove + end check: remove_peer_and_check_ended holds the
    // AdmissionGuard lock across index recycling AND the is_empty + ended=true
    // check. Ingress mirrors never archive authoritative huddle state; they
    // remove locally and let the owner decide room lifetime.
    let (removed_peer, should_auto_end) = if let Some(epoch) = protected_peer_epoch {
        (room.remove_protected_epoch(epoch), false)
    } else if remote_session.is_some() {
        (room.remove_peer(peer_id), false)
    } else {
        match room.remove_peer_and_check_ended(peer_id) {
            Some((_, ended)) => (true, ended),
            None => (false, false),
        }
    };

    let left_msg = serde_json::json!({
        "type": "left",
        "pubkey": pubkey_hex,
        "peer_index": peer_index,
    })
    .to_string();
    if remote_session.is_none() && protected_peer_epoch.is_none() && removed_peer {
        room.broadcast_control(left_msg);
    }

    if !protected_authority.is_enforcing() {
        emit_participant_event(
            &state,
            &tenant,
            Kind::Custom(48102),
            channel_id,
            parent_id_for_event,
            &pubkey_hex,
        )
        .await;
    }

    let room_emptied;
    let mut owner_release_coupled = false;
    if protected_authority.is_enforcing() {
        // Automatic protected-state mutation requires a separately reviewed
        // system-authority model. Keep the room reusable and leave durable
        // channel state unchanged.
        room.clear_ended();
        room_emptied = cleanup_room_if_empty();
        owner_release_coupled = room_emptied;
    } else if should_auto_end {
        info!("audio room empty — auto-ending huddle");

        match state
            .db
            .archive_channel(tenant.community(), channel_id)
            .await
        {
            Err(e) => {
                warn!("auto-archive failed, huddle stays alive: {e}");
                room.clear_ended();
                room_emptied = false;
            }
            Ok(()) => {
                room_emptied = cleanup_room_if_empty();

                emit_participant_event(
                    &state,
                    &tenant,
                    Kind::Custom(48103),
                    channel_id,
                    parent_id_for_event,
                    &pubkey_hex,
                )
                .await;
            }
        }
    } else {
        room_emptied = cleanup_room_if_empty();
    }

    // Owner path: release this room's lease when the room empties, so a new
    // owner can acquire and the renewer stops cleanly (silent, not owner-loss).
    // Fenced on the generation this connection saw as owner: if the room
    // emptied and a re-acquire installed a newer epoch in the gap, `release`
    // is a no-op for the stale generation and leaves the live renewer running.
    // Only the last leaver empties the room, so exactly one release fires.
    if room_emptied && !owner_release_coupled {
        if let (Some(mesh), Some(generation)) = (state.mesh(), owner_generation) {
            mesh.owners.release(channel_id, generation);
        }
    }

    // Local/mesh effects and roster visibility are gone before PostgreSQL
    // cleanup can wait or retry. A database stall cannot retain an expired
    // peer in media, control, or roster state.
    if let Some(admission) = durable_audio_admission.as_mut() {
        if let Err(error) = admission.request_cleanup().await {
            warn!(%error, "audio: durable cleanup intent failed; drop retry scheduled");
        }
    }
    if let Some(admission) = durable_audio_admission.take() {
        admission.finish().await;
    }

    info!("audio peer left");
}

async fn commit_existing_member_audio_admission(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
    pubkey: &nostr::PublicKey,
    proof: &buzz_auth::VerifiedNostrProof,
    claimant_id: Uuid,
    authority: &ProtectedAuthorization,
) -> Result<Uuid, crate::authorization_runtime::executor::AuthorizationExecutionError> {
    use crate::authorization_runtime::executor::{
        begin_authorized_operation, AuthorizedOperationStart, ProtectedOperationId,
    };

    let mut stable = Sha256::new();
    stable.update(b"buzz-audio-admission-operation-v1");
    stable.update(tenant.community().as_uuid().as_bytes());
    stable.update(channel_id.as_bytes());
    stable.update(pubkey.to_bytes());
    stable.update(proof.operation_binding().fingerprint());
    let stable: [u8; 32] = stable.finalize().into();
    let mut admission_bytes = [0_u8; 16];
    admission_bytes.copy_from_slice(&stable[..16]);
    admission_bytes[6] = (admission_bytes[6] & 0x0f) | 0x50;
    admission_bytes[8] = (admission_bytes[8] & 0x3f) | 0x80;
    let admission_id = Uuid::from_bytes(admission_bytes);
    let operation_id =
        ProtectedOperationId::derive(tenant.community(), "audio.admission.v1", &stable)?;
    let mut request = Sha256::new();
    request.update(b"buzz-audio-admission-request-v1");
    request.update(channel_id.as_bytes());
    request.update(pubkey.to_bytes());
    request.update(claimant_id.as_bytes());
    let request: [u8; 32] = request.finalize().into();
    let permit = authority
        .seal_postgres_mutation(operation_id, "audio.admission.v1", request)
        .map_err(|_| {
            crate::authorization_runtime::executor::AuthorizationExecutionError::InvalidCommitFence
        })?
        .ok_or(
            crate::authorization_runtime::executor::AuthorizationExecutionError::InvalidCommitFence,
        )?;
    match begin_authorized_operation(state, permit).await? {
        AuthorizedOperationStart::Replay(payload) => {
            let bytes: [u8; 16] = payload.try_into().map_err(|_| {
                crate::authorization_runtime::executor::AuthorizationExecutionError::ConflictingRetry
            })?;
            Ok(Uuid::from_bytes(bytes))
        }
        AuthorizedOperationStart::Execute(mut operation) => {
            let expires_at = authority.expires_at().ok_or(
                crate::authorization_runtime::executor::AuthorizationExecutionError::Expired,
            )?;
            buzz_db::audio_admission::admit_existing_audio_member_tx(
                operation.transaction(),
                tenant.community(),
                admission_id,
                channel_id,
                &pubkey.to_bytes(),
                claimant_id,
                expires_at,
            )
            .await
            .map_err(crate::authorization_runtime::executor::AuthorizationExecutionError::Db)?;
            operation.commit(admission_id.as_bytes()).await?;
            Ok(admission_id)
        }
    }
}

async fn activate_existing_member_audio_admission(
    state: &AppState,
    community_id: CommunityId,
    admission_id: Uuid,
    channel_id: Uuid,
    pubkey: &[u8; 32],
    claimant_id: Uuid,
    authority: &ProtectedAuthorization,
) -> Result<(), crate::authorization_runtime::executor::AuthorizationExecutionError> {
    use crate::authorization_runtime::executor::{
        begin_authorized_operation, AuthorizedOperationStart, ProtectedOperationId,
    };

    let operation_id = ProtectedOperationId::derive(
        community_id,
        "audio.admission.activate.v1",
        admission_id.as_bytes(),
    )?;
    let mut request = Sha256::new();
    request.update(b"buzz-audio-admission-activation-v1");
    request.update(admission_id.as_bytes());
    request.update(channel_id.as_bytes());
    request.update(pubkey);
    request.update(claimant_id.as_bytes());
    let request: [u8; 32] = request.finalize().into();
    let permit = authority
        .seal_postgres_mutation(operation_id, "audio.admission.activate.v1", request)
        .map_err(|_| {
            crate::authorization_runtime::executor::AuthorizationExecutionError::InvalidCommitFence
        })?
        .ok_or(
            crate::authorization_runtime::executor::AuthorizationExecutionError::InvalidCommitFence,
        )?;
    match begin_authorized_operation(state, permit).await? {
        AuthorizedOperationStart::Replay(payload) => {
            if payload.as_slice() != admission_id.as_bytes()
                || !buzz_db::audio_admission::audio_admission_is_active(
                    &state.db,
                    community_id,
                    admission_id,
                    channel_id,
                    pubkey,
                    claimant_id,
                )
                .await?
            {
                return Err(
                    crate::authorization_runtime::executor::AuthorizationExecutionError::ConflictingRetry,
                );
            }
            Ok(())
        }
        AuthorizedOperationStart::Execute(mut operation) => {
            let expires_at = authority.expires_at().ok_or(
                crate::authorization_runtime::executor::AuthorizationExecutionError::Expired,
            )?;
            buzz_db::audio_admission::activate_audio_admission_tx(
                operation.transaction(),
                community_id,
                admission_id,
                channel_id,
                pubkey,
                claimant_id,
                expires_at,
            )
            .await?;
            operation.commit(admission_id.as_bytes()).await?;
            Ok(())
        }
    }
}

/// Owns durable compensation for every return path after reserve. A process
/// crash is reconciled from PostgreSQL; ordinary cancellation and errors are
/// compensated by Drop, while a durably observed attachment records completion.
struct DurableAudioAdmissionGuard {
    db: buzz_db::Db,
    restore: Arc<crate::authorization_runtime::restore::RestoreProtectionRuntime>,
    community_id: CommunityId,
    admission_id: Uuid,
    claimant_id: Uuid,
    visibility_committed: bool,
    effect_published: bool,
    settled: bool,
}

impl DurableAudioAdmissionGuard {
    fn new(
        state: &AppState,
        community_id: CommunityId,
        admission_id: Uuid,
        claimant_id: Uuid,
    ) -> Option<Self> {
        Some(Self {
            db: state.db.clone(),
            restore: Arc::clone(state.restore_protection()?),
            community_id,
            admission_id,
            claimant_id,
            visibility_committed: false,
            effect_published: false,
            settled: false,
        })
    }

    async fn activate(
        &mut self,
        state: &AppState,
        authority: &ProtectedAuthorization,
        channel_id: Uuid,
        pubkey: &[u8; 32],
    ) -> Result<(), crate::authorization_runtime::executor::AuthorizationExecutionError> {
        activate_existing_member_audio_admission(
            state,
            self.community_id,
            self.admission_id,
            channel_id,
            pubkey,
            self.claimant_id,
            authority,
        )
        .await
    }

    async fn mark_visible(
        &mut self,
    ) -> Result<(), crate::authorization_runtime::executor::AuthorizationExecutionError> {
        mark_durable_audio_admission_visible(
            &self.db,
            &self.restore,
            self.community_id,
            self.admission_id,
            self.claimant_id,
        )
        .await?;
        self.visibility_committed = true;
        Ok(())
    }

    fn mark_published(&mut self) {
        debug_assert!(
            self.visibility_committed,
            "protected audio cannot publish before durable visibility"
        );
        self.effect_published = self.visibility_committed;
    }

    async fn request_cleanup(
        &mut self,
    ) -> Result<(), crate::authorization_runtime::executor::AuthorizationExecutionError> {
        request_durable_audio_admission_cleanup(
            &self.db,
            &self.restore,
            self.community_id,
            self.admission_id,
            self.claimant_id,
        )
        .await
    }

    async fn finish(mut self) {
        match finalize_durable_audio_admission(
            &self.db,
            &self.restore,
            self.community_id,
            self.admission_id,
            self.claimant_id,
            self.effect_published,
        )
        .await
        {
            Ok(()) => self.settled = true,
            Err(error) => {
                warn!(%error, "audio: awaited durable admission cleanup failed; retry scheduled")
            }
        }
    }

    async fn is_current(&self, state: &AppState, channel_id: Uuid, pubkey: &[u8; 32]) -> bool {
        buzz_db::audio_admission::audio_admission_is_active(
            &state.db,
            self.community_id,
            self.admission_id,
            channel_id,
            pubkey,
            self.claimant_id,
        )
        .await
        .unwrap_or(false)
    }
}

impl Drop for DurableAudioAdmissionGuard {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let db = self.db.clone();
        let restore = Arc::clone(&self.restore);
        let community_id = self.community_id;
        let admission_id = self.admission_id;
        let claimant_id = self.claimant_id;
        let effect_published = self.effect_published;
        tokio::spawn(async move {
            if let Err(error) = finalize_durable_audio_admission(
                &db,
                &restore,
                community_id,
                admission_id,
                claimant_id,
                effect_published,
            )
            .await
            {
                warn!(%error, "audio: durable admission cleanup retries exhausted");
            }
        });
    }
}

async fn mark_durable_audio_admission_visible(
    db: &buzz_db::Db,
    restore: &Arc<crate::authorization_runtime::restore::RestoreProtectionRuntime>,
    community_id: CommunityId,
    admission_id: Uuid,
    claimant_id: Uuid,
) -> Result<(), crate::authorization_runtime::executor::AuthorizationExecutionError> {
    use crate::authorization_runtime::executor::{
        AuthorizationExecutionError, ProtectedOperationId,
    };

    let mut stable = Sha256::new();
    stable.update(b"buzz-audio-admission-visibility-v1");
    stable.update(admission_id.as_bytes());
    stable.update(claimant_id.as_bytes());
    let stable: [u8; 32] = stable.finalize().into();
    let operation =
        ProtectedOperationId::derive(community_id, "audio.admission.visible.v1", &stable)?;
    let mut request = Sha256::new();
    request.update(b"buzz-audio-admission-visibility-request-v1");
    request.update(stable);
    let request: [u8; 32] = request.finalize().into();
    let witness = restore
        .begin(community_id, operation.as_uuid(), request)
        .await?;
    match buzz_db::audio_admission::mark_audio_admission_visible_with_receipt(
        db,
        community_id,
        admission_id,
        claimant_id,
        operation.as_uuid(),
        request,
    )
    .await
    {
        Ok(()) => witness.commit().await?,
        Err(error) => {
            if db
                .authorization_operation_receipt_fingerprint(community_id, operation.as_uuid())
                .await?
                == Some(request)
            {
                witness.commit().await?;
            } else {
                witness.abort().await?;
                return Err(AuthorizationExecutionError::Db(error));
            }
        }
    }
    Ok(())
}

async fn finalize_durable_audio_admission(
    db: &buzz_db::Db,
    restore: &Arc<crate::authorization_runtime::restore::RestoreProtectionRuntime>,
    community_id: CommunityId,
    admission_id: Uuid,
    claimant_id: Uuid,
    visible: bool,
) -> Result<(), crate::authorization_runtime::executor::AuthorizationExecutionError> {
    let mut last_error = None;
    for attempt in 0_u32..8 {
        match finalize_durable_audio_admission_once(
            db,
            restore,
            community_id,
            admission_id,
            claimant_id,
            visible,
        )
        .await
        {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        tokio::time::sleep(std::time::Duration::from_millis(
            25_u64.saturating_mul(1_u64 << attempt.min(6)),
        ))
        .await;
    }
    Err(last_error.expect("at least one durable cleanup attempt"))
}

async fn finalize_durable_audio_admission_once(
    db: &buzz_db::Db,
    restore: &Arc<crate::authorization_runtime::restore::RestoreProtectionRuntime>,
    community_id: CommunityId,
    admission_id: Uuid,
    claimant_id: Uuid,
    visible: bool,
) -> Result<(), crate::authorization_runtime::executor::AuthorizationExecutionError> {
    use crate::authorization_runtime::executor::{
        AuthorizationExecutionError, ProtectedOperationId,
    };

    request_durable_audio_admission_cleanup(db, restore, community_id, admission_id, claimant_id)
        .await?;

    let terminal = if visible {
        b"finished".as_slice()
    } else {
        b"aborted".as_slice()
    };
    let mut completion_stable = Sha256::new();
    completion_stable.update(b"buzz-audio-admission-completion-v1");
    completion_stable.update(admission_id.as_bytes());
    completion_stable.update(claimant_id.as_bytes());
    completion_stable.update(terminal);
    let completion_stable: [u8; 32] = completion_stable.finalize().into();
    let completion_operation = ProtectedOperationId::derive(
        community_id,
        "audio.admission.complete.v1",
        &completion_stable,
    )?;
    let mut completion_request = Sha256::new();
    completion_request.update(b"buzz-audio-admission-completion-request-v1");
    completion_request.update(completion_stable);
    let completion_request: [u8; 32] = completion_request.finalize().into();
    let completion_witness = restore
        .begin(
            community_id,
            completion_operation.as_uuid(),
            completion_request,
        )
        .await?;
    match buzz_db::audio_admission::complete_claimed_audio_admission_with_receipt(
        db,
        community_id,
        admission_id,
        claimant_id,
        visible,
        (!visible).then_some("attachment_aborted"),
        completion_operation.as_uuid(),
        completion_request,
    )
    .await
    {
        Ok(()) => completion_witness.commit().await?,
        Err(error) => {
            if db
                .authorization_operation_receipt_fingerprint(
                    community_id,
                    completion_operation.as_uuid(),
                )
                .await?
                == Some(completion_request)
            {
                completion_witness.commit().await?;
            } else {
                completion_witness.abort().await?;
                return Err(AuthorizationExecutionError::Db(error));
            }
        }
    }
    Ok(())
}

async fn request_durable_audio_admission_cleanup(
    db: &buzz_db::Db,
    restore: &Arc<crate::authorization_runtime::restore::RestoreProtectionRuntime>,
    community_id: CommunityId,
    admission_id: Uuid,
    claimant_id: Uuid,
) -> Result<(), crate::authorization_runtime::executor::AuthorizationExecutionError> {
    use crate::authorization_runtime::executor::{
        AuthorizationExecutionError, ProtectedOperationId,
    };

    let mut cleanup_stable = Sha256::new();
    cleanup_stable.update(b"buzz-audio-admission-cleanup-request-v1");
    cleanup_stable.update(admission_id.as_bytes());
    cleanup_stable.update(claimant_id.as_bytes());
    let cleanup_stable: [u8; 32] = cleanup_stable.finalize().into();
    let cleanup_operation = ProtectedOperationId::derive(
        community_id,
        "audio.admission.cleanup-request.v1",
        &cleanup_stable,
    )?;
    let mut cleanup_request = Sha256::new();
    cleanup_request.update(b"buzz-audio-admission-cleanup-request-receipt-v1");
    cleanup_request.update(cleanup_stable);
    let cleanup_request: [u8; 32] = cleanup_request.finalize().into();
    let cleanup_witness = restore
        .begin(community_id, cleanup_operation.as_uuid(), cleanup_request)
        .await?;
    match buzz_db::audio_admission::request_audio_admission_cleanup_with_receipt(
        db,
        community_id,
        admission_id,
        claimant_id,
        cleanup_operation.as_uuid(),
        cleanup_request,
    )
    .await
    {
        Ok(()) => cleanup_witness.commit().await?,
        Err(error) => {
            if db
                .authorization_operation_receipt_fingerprint(
                    community_id,
                    cleanup_operation.as_uuid(),
                )
                .await?
                == Some(cleanup_request)
            {
                cleanup_witness.commit().await?;
            } else {
                cleanup_witness.abort().await?;
                return Err(AuthorizationExecutionError::Db(error));
            }
        }
    }

    Ok(())
}

async fn release_pending_huddle_lease(
    state: &AppState,
    lease: &mut Option<crate::audio::join::HuddleLease>,
) {
    let (Some(mesh), Some(owned_lease)) = (state.mesh(), lease.take()) else {
        return;
    };
    let mut last_error = None;
    for attempt in 0_u32..5 {
        match crate::audio::join::HuddleDirectory::release(&mesh.directory, &owned_lease).await {
            Ok(_) => return,
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(std::time::Duration::from_millis(
                    25_u64.saturating_mul(1_u64 << attempt),
                ))
                .await;
            }
        }
    }
    // The lease itself remains bounded by Redis TTL, but a failed explicit
    // compensation must never disappear silently.
    warn!(error = ?last_error, "audio: failed to release pending owner lease after retries");
}

/// React to a non-owner huddle teardown signal read off the owner's control
/// stream: cancel the connection (which drives the client's WS to close so it
/// rejoins) and forget the local generation floor for this session.
///
/// The `cause` is logged for observability but does not change behaviour —
/// every cause is recoverable by a rejoin, whether against a fresh owner
/// (`OwnerLost`/`StreamClosed`), a draining owner (`OwnerDraining`), or a room
/// that simply ended (`SessionEnded`). `forget` clears local stale-frame
/// suppression so the rejoin's fresh generation is accepted; it never
/// authorizes ownership — Redis fenced CAS remains the arbiter.
fn teardown_remote_huddle(
    cause: crate::audio::join::HuddleTeardownCause,
    channel_id: Uuid,
    cancel: &CancellationToken,
    fence: &crate::audio::mesh::GenerationFloor,
) {
    info!(
        ?cause,
        "owner tore down cross-pod huddle session — closing client for rejoin"
    );
    cancel.cancel();
    fence.forget(channel_id);
}

/// Map an owner's registration rejection to the client-facing WS error, using
/// the same `code`s a same-pod join produces so a cross-pod client handles them
/// identically. Fence rejections carry their taxonomy code for observability.
fn remote_rejection_ws_error(reason: &crate::audio::join::RegisterRejection) -> serde_json::Value {
    use crate::audio::join::RegisterRejection;
    match reason {
        RegisterRejection::RoomFull => serde_json::json!({
            "type": "error", "code": "room_full",
            "message": "peer index space exhausted"
        }),
        RegisterRejection::RoomEnded => serde_json::json!({
            "type": "error", "code": "room_ended", "message": "huddle has ended"
        }),
        RegisterRejection::VersionMismatch { pinned, requested } => serde_json::json!({
            "type": "error", "code": "upgrade_required",
            "message": format!(
                "this huddle is using audio protocol v{pinned}; your client requested v{requested}"
            ),
            "pinned_version": pinned,
            "requested_version": requested,
        }),
        RegisterRejection::Fenced(f) => serde_json::json!({
            "type": "error", "code": "join_rejected",
            "message": "huddle join rejected",
            "fence_reason": f.code(),
        }),
    }
}

/// Receive loop: reads client frames and routes them. Local/owner joins fan
/// out through the local room; a non-owner join forwards to the huddle owner
/// via `remote_session`. Argument count reflects the pre-existing connection
/// wiring plus the one mesh session; a param struct would obscure more than it
/// clarifies at this single call site.
#[allow(clippy::too_many_arguments)]
async fn recv_loop(
    mut ws_recv: futures_util::stream::SplitStream<WebSocket>,
    room: Arc<crate::audio::room::Room>,
    peer_id: Uuid,
    protocol_version: u8,
    ctrl_tx: mpsc::Sender<WsMessage>,
    missed_pongs: Arc<AtomicU8>,
    cancel: CancellationToken,
    protected_authority: Arc<ProtectedAuthorization>,
    mut remote_session: Option<&mut crate::audio::join::RemoteHuddleSession>,
) {
    use crate::audio::wire::{FrameHeader, V2_HEADER_LEN};

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            msg = ws_recv.next() => {
                if protected_authority.revalidate().is_err() {
                    cancel.cancel();
                    break;
                }
                match msg {
                    Some(Ok(WsMessage::Binary(data))) => {
                        if data.len() > MAX_AUDIO_FRAME_BYTES {
                            warn!(peer_id = %peer_id, bytes = data.len(), "audio frame too large — dropping");
                            continue;
                        }

                        // Protocol v2 sanity-parse: validate the header is
                        // present and well-shaped, then forward opaquely.
                        // We never strip, rewrite, or re-encode bytes — the
                        // header is sender-authored telemetry only — but we
                        // do refuse to broadcast frames that are clearly
                        // malformed for the room's pinned protocol so we
                        // don't help v2 peers feed garbage to other v2 peers.
                        if protocol_version >= 2 {
                            // Frame must carry at least the 8-byte header
                            // plus a non-empty Opus payload.
                            if data.len() <= V2_HEADER_LEN {
                                warn!(
                                    peer_id = %peer_id,
                                    bytes = data.len(),
                                    "v2 frame missing header or payload — dropping"
                                );
                                continue;
                            }
                            match FrameHeader::parse(&data) {
                                Some((header, payload)) if !payload.is_empty() => {
                                    // Header is well-formed. `level_dbov` is
                                    // already clamped by `parse` — bad values
                                    // do not drop the frame, they just lose
                                    // the metric (which the relay does not
                                    // trust for anything anyway).
                                    tracing::trace!(
                                        peer_id = %peer_id,
                                        seq = header.seq,
                                        ts_48k = header.ts_48k,
                                        level_dbov = header.level_dbov,
                                        is_dtx = header.is_dtx(),
                                        "v2 audio frame"
                                    );
                                }
                                _ => {
                                    warn!(
                                        peer_id = %peer_id,
                                        bytes = data.len(),
                                        "v2 frame failed header parse — dropping"
                                    );
                                    continue;
                                }
                            }
                        }

                        // Non-owner path forwards the client's Opus to the
                        // huddle owner as a datagram (the owner is the sole
                        // fan-out authority); the owner-side room fans it back
                        // to every participant, including our co-located peers.
                        // Owner/local path fans out through the local room.
                        match remote_session.as_deref_mut() {
                            Some(session) => session.forward_media(&room, &data),
                            None => room.broadcast_frame(peer_id, data),
                        }
                    }
                    Some(Ok(WsMessage::Text(text))) => {
                        if text.len() > MAX_TEXT_FRAME_BYTES {
                            warn!(peer_id = %peer_id, bytes = text.len(), "control text frame too large — dropping");
                            continue;
                        }
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                            if v.get("type").and_then(|t| t.as_str()) == Some("leave") {
                                break;
                            }
                        }
                    }
                    Some(Ok(WsMessage::Pong(_))) => {
                        missed_pongs.store(0, Ordering::Relaxed);
                    }
                    Some(Ok(WsMessage::Ping(data))) => {
                        // Pong goes through the control channel — priority delivery.
                        let _ = ctrl_tx.try_send(WsMessage::Pong(data));
                    }
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Err(e)) => {
                        debug!(peer_id = %peer_id, "ws error: {e}");
                        break;
                    }
                }
            }
        }
    }
}

/// Wait for sink readiness, then revalidate immediately before `start_send`.
async fn send_protected_ws<S>(
    sink: &mut S,
    message: WsMessage,
    protected_authority: &dyn crate::connection::OutboundReleaseFence,
) -> bool
where
    S: futures_util::Sink<WsMessage> + Unpin,
{
    if std::future::poll_fn(|cx| std::pin::Pin::new(&mut *sink).poll_ready(cx))
        .await
        .is_err()
    {
        return false;
    }
    if !protected_authority.release() {
        return false;
    }
    if std::pin::Pin::new(&mut *sink).start_send(message).is_err() {
        return false;
    }
    futures_util::SinkExt::flush(sink).await.is_ok()
}

/// Outbound send loop with control-frame priority (matches connection.rs pattern).
///
/// Control frames (Ping, Pong, Close, control JSON) are drained first on every
/// iteration, so heartbeat pings are never starved by audio backpressure.
async fn send_loop(
    mut ws_send: futures_util::stream::SplitSink<WebSocket, WsMessage>,
    mut data_rx: mpsc::Receiver<WsMessage>,
    mut ctrl_rx: mpsc::Receiver<WsMessage>,
    cancel: CancellationToken,
    protected_authority: Arc<ProtectedAuthorization>,
) {
    loop {
        // Priority: drain all pending control frames before data.
        while let Ok(ctrl_msg) = ctrl_rx.try_recv() {
            if !send_protected_ws(&mut ws_send, ctrl_msg, protected_authority.as_ref()).await {
                cancel.cancel();
                return;
            }
        }

        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let _ = ws_send.send(WsMessage::Close(None)).await;
                break;
            }
            Some(ctrl_msg) = ctrl_rx.recv() => {
                if !send_protected_ws(&mut ws_send, ctrl_msg, protected_authority.as_ref()).await {
                    cancel.cancel();
                    break;
                }
            }
            Some(msg) = data_rx.recv() => {
                if !send_protected_ws(&mut ws_send, msg, protected_authority.as_ref()).await {
                    cancel.cancel();
                    break;
                }
            }
        }
    }
}

// Bridges the room's mpsc channel to the WS send channel.

/// Bridges room per-peer channels → WS send channels.
/// Audio frames (from room audio_rx) go to data_tx.
/// Control messages (from room ctrl_rx) go to ws ctrl_tx (priority path).
/// Two separate room channels ensure control is never starved by audio backpressure.
async fn audio_forward_loop(
    mut audio_rx: mpsc::Receiver<Bytes>,
    mut peer_ctrl_rx: mpsc::Receiver<PeerCtrl>,
    data_tx: mpsc::Sender<WsMessage>,
    ctrl_tx: mpsc::Sender<WsMessage>,
    cancel: CancellationToken,
    protected_authority: Arc<ProtectedAuthorization>,
) {
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            // Control messages get priority over audio in the select.
            msg = peer_ctrl_rx.recv() => {
                if protected_authority.revalidate().is_err() {
                    cancel.cancel();
                    break;
                }
                match msg {
                    Some(PeerCtrl::Json(json)) => {
                        let _ = ctrl_tx.try_send(WsMessage::Text(json.into()));
                    }
                    Some(PeerCtrl::Close) | None => {
                        cancel.cancel();
                        break;
                    }
                }
            }
            frame = audio_rx.recv() => {
                if protected_authority.revalidate().is_err() {
                    cancel.cancel();
                    break;
                }
                match frame {
                    Some(bytes) => {
                        let _ = data_tx.try_send(WsMessage::Binary(bytes));
                    }
                    None => {
                        cancel.cancel();
                        break;
                    }
                }
            }
        }
    }
}

async fn heartbeat_loop(
    ws_tx: mpsc::Sender<WsMessage>,
    missed_pongs: Arc<AtomicU8>,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                // fetch_add returns the previous value; +1 gives the current count.
                let missed = missed_pongs.fetch_add(1, Ordering::Relaxed) + 1;
                if missed >= MAX_MISSED_PONGS {
                    warn!("audio: {missed} missed pongs — closing connection");
                    cancel.cancel();
                    break;
                }
                if ws_tx.try_send(WsMessage::Ping(axum::body::Bytes::new())).is_err() {
                    cancel.cancel();
                    break;
                }
            }
            _ = cancel.cancelled() => break,
        }
    }
}

async fn ensure_membership(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
    pubkey_bytes: &[u8],
    parent_channel_id: Option<Uuid>,
    enforcing: bool,
) -> Result<AudioMembership, String> {
    // Load channel first — reject archived channels before any membership check.
    // This ensures auto-ended huddles can't be rejoined by existing members.
    let channel = state
        .db
        .get_channel(tenant.community(), channel_id)
        .await
        .map_err(|e| format!("db error: {e}"))?;

    if channel.archived_at.is_some() {
        return Err("channel is archived".into());
    }

    // Lifecycle events for an ephemeral huddle belong in its parent channel.
    // Resolve that parent from a creator-signed kind:48100 event instead of
    // trusting the UUID supplied by the client during audio auth.
    let lifecycle_parent_id = if channel.ttl_seconds.is_some() {
        let parent_id = parent_channel_id.ok_or("ephemeral channel requires parent linkage")?;
        let linked = state
            .db
            .huddle_started_link_exists(
                tenant.community(),
                parent_id,
                channel_id,
                &channel.created_by,
            )
            .await
            .map_err(|e| format!("db error: {e}"))?;
        if !linked {
            return Err("ephemeral channel is not linked to claimed parent".into());
        }
        parent_id
    } else {
        channel_id
    };

    // Fast path: already a member.
    let is_member = if enforcing {
        state
            .db
            .is_member(tenant.community(), channel_id, pubkey_bytes)
            .await
    } else {
        state
            .is_member_cached(tenant.community(), channel_id, pubkey_bytes)
            .await
    }
    .map_err(|e| format!("db error: {e}"))?;

    if is_member {
        return Ok(AudioMembership::ExistingMember {
            lifecycle_parent_id,
        });
    }

    if enforcing {
        return Err("not a member".into());
    }

    if channel.visibility == "open" {
        return Ok(AudioMembership::LegacyOpenGuest {
            lifecycle_parent_id,
        });
    }

    // Auto-add path: private ephemeral channel + caller is member of parent.
    if channel.ttl_seconds.is_some() {
        let parent_member = state
            .is_member_cached(tenant.community(), lifecycle_parent_id, pubkey_bytes)
            .await
            .map_err(|e| format!("db error: {e}"))?;

        if parent_member {
            return Ok(AudioMembership::LegacyAutoAdd {
                lifecycle_parent_id,
                added_by: channel.created_by,
            });
        }
    }

    Err("not a member".into())
}

enum AudioMembership {
    ExistingMember {
        lifecycle_parent_id: Uuid,
    },
    LegacyOpenGuest {
        lifecycle_parent_id: Uuid,
    },
    LegacyAutoAdd {
        lifecycle_parent_id: Uuid,
        added_by: Vec<u8>,
    },
}

impl AudioMembership {
    const fn lifecycle_parent_id(&self) -> Uuid {
        match self {
            Self::ExistingMember {
                lifecycle_parent_id,
            }
            | Self::LegacyOpenGuest {
                lifecycle_parent_id,
            }
            | Self::LegacyAutoAdd {
                lifecycle_parent_id,
                ..
            } => *lifecycle_parent_id,
        }
    }
}

async fn emit_participant_event(
    state: &AppState,
    tenant: &TenantContext,
    kind: Kind,
    channel_id: Uuid,
    parent_channel_id: Uuid,
    participant_pubkey: &str,
) {
    let content = serde_json::json!({"ephemeral_channel_id": channel_id.to_string()}).to_string();

    let h_tag = match Tag::parse(["h", &parent_channel_id.to_string()]) {
        Ok(t) => t,
        Err(e) => {
            warn!("audio: failed to parse h tag: {e}");
            return;
        }
    };
    let p_tag = match Tag::parse(["p", participant_pubkey]) {
        Ok(t) => t,
        Err(e) => {
            warn!("audio: failed to parse p tag: {e}");
            return;
        }
    };
    let tags = vec![h_tag, p_tag];

    let event = match EventBuilder::new(kind, content)
        .tags(tags)
        .sign_with_keys(&state.relay_keypair)
    {
        Ok(e) => e,
        Err(e) => {
            warn!("audio: failed to sign lifecycle event: {e}");
            return;
        }
    };

    // 1. Persist to DB so late-joining clients can reconstruct huddle state
    //    from historical queries. Without this, lifecycle events only exist
    //    for the duration of the Redis pub/sub delivery and are lost forever.
    let stored = match state
        .db
        .insert_event(tenant.community(), &event, Some(parent_channel_id))
        .await
    {
        Ok((stored, true)) => stored,
        Ok((_, false)) => {
            // Duplicate — already persisted (e.g. concurrent emit). Skip fan-out
            // to avoid double-delivery, matching the side_effects.rs pattern.
            debug!("audio lifecycle event already persisted — skipping fan-out");
            return;
        }
        Err(e) => {
            // DB failure during disconnect cleanup. Still broadcast so live
            // subscribers see the leave/end event immediately — suppressing it
            // would leave connected clients stale. Late joiners will have an
            // inconsistent view until the next huddle lifecycle event lands.
            warn!(
                kind = %event.kind.as_u16(),
                "audio: failed to persist lifecycle event: {e}"
            );
            StoredEvent::new(event.clone(), Some(parent_channel_id))
        }
    };

    // 2. Mark as locally-published before Redis broadcast to prevent
    //    double-delivery when the event echoes back through the subscriber loop.
    state.mark_local_event(tenant.community(), &event.id);

    // 3. Local fan-out to WS subscribers on this node, through the guarded send
    //    path so a stale subscription on a removed/non-member connection cannot
    //    receive this channel's audio lifecycle event (same gate as
    //    dispatch_persistent_event in the ingest handler).
    crate::handlers::event::fan_out_event_to_local_subscribers(state, tenant.community(), &stored)
        .await;

    // 4. Cross-node broadcast via Redis pub/sub.
    if let Err(e) = state
        .pubsub
        .publish_event(tenant, EventTopic::Channel(parent_channel_id), &event)
        .await
    {
        state
            .local_event_ids
            .invalidate(&(tenant.community(), event.id.to_bytes()));
        warn!("audio: failed to publish lifecycle event: {e}");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    use axum::{routing::get, Router};
    use futures_util::SinkExt;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    use super::*;

    struct ScriptedAudioFence(AtomicBool);

    impl crate::connection::OutboundReleaseFence for ScriptedAudioFence {
        fn release(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }

    struct AudioReadinessBarrierSink {
        ready: Arc<AtomicBool>,
        polled: Arc<tokio::sync::Notify>,
        waker: Arc<Mutex<Option<std::task::Waker>>>,
        sent: Arc<AtomicBool>,
    }

    impl futures_util::Sink<WsMessage> for AudioReadinessBarrierSink {
        type Error = std::io::Error;

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            if self.ready.load(Ordering::SeqCst) {
                std::task::Poll::Ready(Ok(()))
            } else {
                *self.waker.lock().expect("audio barrier waker poisoned") =
                    Some(cx.waker().clone());
                self.polled.notify_one();
                std::task::Poll::Pending
            }
        }

        fn start_send(self: std::pin::Pin<&mut Self>, _item: WsMessage) -> Result<(), Self::Error> {
            self.sent.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            self.poll_flush(cx)
        }
    }

    #[tokio::test]
    async fn audio_frame_revalidates_after_sink_readiness() {
        let ready = Arc::new(AtomicBool::new(false));
        let polled = Arc::new(tokio::sync::Notify::new());
        let waker = Arc::new(Mutex::new(None));
        let sent = Arc::new(AtomicBool::new(false));
        let fence = Arc::new(ScriptedAudioFence(AtomicBool::new(true)));
        let sink = AudioReadinessBarrierSink {
            ready: Arc::clone(&ready),
            polled: Arc::clone(&polled),
            waker: Arc::clone(&waker),
            sent: Arc::clone(&sent),
        };
        let task_fence = Arc::clone(&fence);
        let task = tokio::spawn(async move {
            let mut sink = sink;
            send_protected_ws(
                &mut sink,
                WsMessage::Text("protected".into()),
                task_fence.as_ref(),
            )
            .await
        });

        polled.notified().await;
        fence.0.store(false, Ordering::SeqCst);
        ready.store(true, Ordering::SeqCst);
        waker
            .lock()
            .expect("audio barrier waker poisoned")
            .take()
            .expect("poll_ready registered a waker")
            .wake();

        assert!(!task.await.expect("audio send task joins"));
        assert!(
            !sent.load(Ordering::SeqCst),
            "authority loss while readiness is pending must prevent start_send"
        );
    }

    #[test]
    fn audio_membership_dispositions_retain_the_server_resolved_parent() {
        let parent = Uuid::new_v4();
        for membership in [
            AudioMembership::ExistingMember {
                lifecycle_parent_id: parent,
            },
            AudioMembership::LegacyOpenGuest {
                lifecycle_parent_id: parent,
            },
            AudioMembership::LegacyAutoAdd {
                lifecycle_parent_id: parent,
                added_by: vec![7; 32],
            },
        ] {
            assert_eq!(membership.lifecycle_parent_id(), parent);
        }
    }

    #[test]
    fn durable_visibility_precedes_remote_and_local_publication() {
        let source = include_str!("handler.rs");
        let protected_path = source
            .split_once("let admission = if let Some(admission_id) = protected_admission_id")
            .expect("protected admission path")
            .1;
        let visibility = protected_path
            .find("receipt.mark_visible().await")
            .expect("durable visibility transition");
        for effect in [
            "crate::audio::join::activate_remote_owner",
            "crate::audio::join::confirm_remote_owner",
            "local_reservation.activate_protected_if",
        ] {
            let effect = protected_path
                .find(effect)
                .expect("protected effect boundary");
            assert!(
                visibility < effect,
                "durable visibility must commit before {effect}"
            );
        }
    }

    #[test]
    fn audio_connection_permits_share_the_global_websocket_budget() {
        let semaphore = Arc::new(Semaphore::new(1));
        let first = acquire_audio_connection_permit(&semaphore).expect("first permit");

        assert!(
            acquire_audio_connection_permit(&semaphore).is_none(),
            "audio connections must stop when the global WebSocket budget is exhausted"
        );

        drop(first);
        assert!(
            acquire_audio_connection_permit(&semaphore).is_some(),
            "dropping an audio connection must return its global permit"
        );
    }

    async fn handler_receives_message_of_size(size: usize) -> bool {
        let (received_tx, received_rx) = oneshot::channel();
        let received_tx = Arc::new(Mutex::new(Some(received_tx)));
        let app = Router::new().route(
            "/",
            get({
                let received_tx = Arc::clone(&received_tx);
                move |ws: WebSocketUpgrade| {
                    let received_tx = Arc::clone(&received_tx);
                    async move {
                        limit_audio_websocket(ws).on_upgrade(move |mut socket| async move {
                            let received = matches!(socket.recv().await, Some(Ok(_)));
                            if let Some(tx) =
                                received_tx.lock().expect("result lock poisoned").take()
                            {
                                let _ = tx.send(received);
                            }
                        })
                    }
                }
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test WebSocket listener");
        let addr = listener.local_addr().expect("test listener address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test WebSocket server");
        });

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("connect test WebSocket client");
        client
            .send(Message::Text("x".repeat(size).into()))
            .await
            .expect("send test WebSocket message");

        let received = tokio::time::timeout(Duration::from_secs(2), received_rx)
            .await
            .expect("server should process the test message")
            .expect("server should report whether it received the message");

        server.abort();
        let _ = server.await;

        received
    }

    #[tokio::test]
    async fn audio_websocket_parser_rejects_oversized_messages_before_handler_reads_them() {
        assert!(
            handler_receives_message_of_size(MAX_WEBSOCKET_MESSAGE_BYTES).await,
            "messages at the audio route limit should still reach the handler"
        );
        assert!(
            !handler_receives_message_of_size(MAX_WEBSOCKET_MESSAGE_BYTES + 1).await,
            "oversized messages must be rejected by the WebSocket parser before the handler sees them"
        );
    }
}
