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

use axum::extract::ws::{Message as WsMessage, WebSocket};
use axum::http::{HeaderMap, StatusCode};
use axum::{
    extract::{Path, State, WebSocketUpgrade},
    response::IntoResponse,
};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, Kind, Tag};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use buzz_auth::generate_challenge;
use buzz_core::tenant::TenantContext;
use buzz_db::channel::MemberRole;

use buzz_core::StoredEvent;
use buzz_pubsub::EventTopic;

use crate::audio::room::PeerCtrl;
use crate::state::{run_registered_community_connection, AppState};

/// Maximum binary frame size: 4 KB is generous for a single Opus packet.
const MAX_AUDIO_FRAME_BYTES: usize = 4096;

/// Maximum text frame size: 8 KB bounds auth/control JSON parsing.
const MAX_TEXT_FRAME_BYTES: usize = 8192;

/// Heartbeat interval.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Missed pong limit before disconnect.
const MAX_MISSED_PONGS: u8 = 3;

/// Auth timeout.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);

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
    ws.on_upgrade(move |socket| handle_audio_connection(socket, state, tenant, channel_id))
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
) {
    let cancel = CancellationToken::new();
    let community_id = tenant.community();
    let registry = Arc::clone(&state.community_connections);
    let check_state = Arc::clone(&state);
    let run_state = Arc::clone(&state);
    run_registered_community_connection(
        &registry,
        Uuid::new_v4(),
        community_id,
        cancel.clone(),
        move || async move { check_state.db.is_community_active(community_id).await },
        move || handle_active_audio_connection(socket, run_state, tenant, channel_id, cancel),
    )
    .await;
}

async fn handle_active_audio_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    tenant: TenantContext,
    channel_id: Uuid,
    cancel: CancellationToken,
) {
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
                        warn!(channel_id = %channel_id, "auth text frame too large — dropping");
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
            debug!(channel_id = %channel_id, "audio auth timeout or disconnect");
            return;
        }
    };

    // Extract NIP-OA auth tag before verify_auth_event consumes the event.
    let auth_tag_json = crate::handlers::auth::extract_auth_tag_json(&auth_msg.event);

    let relay_url = crate::api::bridge::nip42_expected_relay_url(&state.config.relay_url, &tenant);
    let auth_ctx = match state
        .auth
        .verify_auth_event(auth_msg.event, &challenge, &relay_url)
        .await
    {
        Ok(ctx) => ctx,
        Err(e) => {
            warn!(channel_id = %channel_id, "audio auth failed: {e}");
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

    if crate::api::relay_members::enforce_relay_membership(
        &state,
        tenant.community(),
        pubkey.as_bytes(),
        auth_tag_json.as_deref(),
    )
    .await
    .is_err()
    {
        warn!(channel_id = %channel_id, pubkey = %pubkey_hex, "audio: relay membership denied");
        let _ = ws_send
            .send(WsMessage::Text(
                serde_json::json!({"type": "error", "message": "restricted: not a relay member"})
                    .to_string()
                    .into(),
            ))
            .await;
        return;
    }

    // ── Step 3: membership check / auto-add ───────────────────────────────────
    let parent_id_for_event = match ensure_membership(
        &state,
        &tenant,
        channel_id,
        &pubkey_bytes,
        parent_channel_id,
    )
    .await
    {
        Ok(parent_id) => parent_id,
        Err(e) => {
            warn!(channel_id = %channel_id, pubkey = %pubkey_hex, "audio membership denied: {e}");
            let _ = ws_send
                .send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"not a member"})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };

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
                    warn!(
                        channel_id = %channel_id,
                        pubkey = %pubkey_hex,
                        "huddle join rejected by fence: {e}"
                    );
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
                debug!(
                    channel_id = %channel_id,
                    pubkey = %pubkey_hex,
                    "huddle audio unavailable under horizontal scaling — rejecting join"
                );
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

    let room = state
        .audio_rooms
        .get_or_create(tenant.community(), channel_id);

    // Re-check archived status after obtaining the room. This closes the
    // cross-boundary race: a joiner that passed ensure_membership before
    // the last peer archived the channel could get a fresh room via
    // get_or_create (the old room was already cleaned up). This DB check
    // catches that case. The room-level ended flag (checked inside add_peer)
    // handles the same-room case.
    match state.db.get_channel(tenant.community(), channel_id).await {
        Ok(ch) if ch.archived_at.is_some() => {
            debug!(channel_id = %channel_id, "channel archived before room join");
            let _ = ws_send
                .send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"huddle has ended"})
                        .to_string()
                        .into(),
                ))
                .await;
            state
                .audio_rooms
                .cleanup_if_empty(tenant.community(), channel_id);
            return;
        }
        Err(e) => {
            warn!(channel_id = %channel_id, "pre-join channel check failed (fail-closed): {e}");
            state
                .audio_rooms
                .cleanup_if_empty(tenant.community(), channel_id);
            return;
        }
        Ok(_) => {} // Channel exists and is not archived — proceed.
    }

    // Reject unsupported future versions up-front so we don't accidentally
    // pin a room to a version we can't speak. Versions 1..=CURRENT are OK.
    let requested_version = auth_msg.protocol_version;
    if requested_version == 0 || requested_version > CURRENT_PROTOCOL_VERSION {
        warn!(
            channel_id = %channel_id,
            pubkey = %pubkey_hex,
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
        return;
    }

    // Remote registration happens before ingress admission. The owner-assigned
    // index is therefore the only index this client ever has; no frame or
    // `joined` message can escape with an ingress-local placeholder.
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
        match crate::audio::join::dial_remote_owner(
            Arc::clone(&mesh.transport),
            mesh.local_runtime_id,
            owner_runtime_id,
            fenced,
            tenant.community(),
            pubkey_hex.clone(),
            requested_version,
        )
        .await
        {
            Ok((session, stream)) => {
                remote_session = Some(session);
                remote_stream = Some(stream);
                remote_fence = Some(Arc::clone(&mesh.audio_fence));
            }
            Err(crate::audio::join::DialError::Rejected(reason)) => {
                warn!(channel_id = %channel_id, pubkey = %pubkey_hex, "huddle owner rejected registration: {reason:?}");
                let _ = ws_send
                    .send(WsMessage::Text(
                        remote_rejection_ws_error(&reason).to_string().into(),
                    ))
                    .await;
                state
                    .audio_rooms
                    .cleanup_if_empty(tenant.community(), channel_id);
                return;
            }
            Err(crate::audio::join::DialError::Mesh(e)) => {
                warn!(channel_id = %channel_id, pubkey = %pubkey_hex, "huddle owner registration failed: {e}");
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
                state
                    .audio_rooms
                    .cleanup_if_empty(tenant.community(), channel_id);
                return;
            }
        }
    }

    let admission = if let Some(session) = remote_session.as_ref() {
        room.add_peer_at_index(pubkey_hex.clone(), requested_version, session.peer_index())
            .map(|(id, audio, ctrl)| (id, session.peer_index(), audio, ctrl))
    } else {
        room.add_peer(pubkey_hex.clone(), requested_version)
    };
    let (peer_id, peer_index, audio_rx, peer_ctrl_rx) = match admission {
        Ok(v) => v,
        Err(crate::audio::room::AdmissionError::Full) => {
            warn!(channel_id = %channel_id, "audio room full (255 peers exhausted)");
            let _ = ws_send.send(WsMessage::Text(serde_json::json!({"type":"error","code":"room_full","message":"peer index space exhausted"}).to_string().into())).await;
            if let (Some(session), Some(stream)) = (remote_session.as_ref(), remote_stream.as_mut())
            {
                crate::audio::join::send_clean_close(stream, session.fenced(), session.pubkey())
                    .await;
            }
            return;
        }
        Err(crate::audio::room::AdmissionError::Ended) => {
            debug!(channel_id = %channel_id, "room ended before admission");
            let _ = ws_send.send(WsMessage::Text(serde_json::json!({"type":"error","code":"room_ended","message":"huddle has ended"}).to_string().into())).await;
            if let (Some(session), Some(stream)) = (remote_session.as_ref(), remote_stream.as_mut())
            {
                crate::audio::join::send_clean_close(stream, session.fenced(), session.pubkey())
                    .await;
            }
            return;
        }
        Err(crate::audio::room::AdmissionError::VersionMismatch { pinned, requested }) => {
            info!(channel_id = %channel_id, pubkey = %pubkey_hex, pinned, requested, "audio: protocol version mismatch — upgrade required");
            let _ = ws_send.send(WsMessage::Text(serde_json::json!({
                "type": "error", "code": "upgrade_required",
                "message": format!("this huddle is using audio protocol v{pinned}; your client requested v{requested}"),
                "pinned_version": pinned, "requested_version": requested,
            }).to_string().into())).await;
            if let (Some(session), Some(stream)) = (remote_session.as_ref(), remote_stream.as_mut())
            {
                crate::audio::join::send_clean_close(stream, session.fenced(), session.pubkey())
                    .await;
            }
            return;
        }
    };

    info!(
        channel_id = %channel_id,
        pubkey = %pubkey_hex,
        peer_index,
        "audio peer joined"
    );

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
                        channel_id = %channel_id,
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

    if remote_session.is_some() {
        if ws_send
            .send(WsMessage::Text(joined_msg.into()))
            .await
            .is_err()
        {
            room.remove_peer(peer_id);
            state
                .audio_rooms
                .cleanup_if_empty(tenant.community(), channel_id);
            return;
        }
    } else {
        room.broadcast_control(joined_msg);
    }

    // ── Step 6: emit kind:48101 (PARTICIPANT_JOINED) ──────────────────────────
    emit_participant_event(
        &state,
        &tenant,
        Kind::Custom(48101),
        channel_id,
        parent_id_for_event,
        &pubkey_hex,
    )
    .await;

    let missed_pongs = Arc::new(AtomicU8::new(0));

    // Dual-channel pattern (matches connection.rs): data channel for audio,
    // control channel for Ping/Pong/Close/control JSON with priority drain.
    let (data_tx, data_rx) = mpsc::channel::<WsMessage>(16);
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<WsMessage>(8);

    let send_cancel = cancel.child_token();
    let send_task = tokio::spawn(send_loop(ws_send, data_rx, ctrl_rx, send_cancel));

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
                    crate::audio::join::send_clean_close(&mut stream, fenced, &pubkey).await;
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
                        channel_id = %channel_id,
                        "huddle owner is draining — closing local client for rejoin"
                    );
                    owner_cancel.cancel();
                    fence.forget(channel_id);
                }
                _ = lost_fired => {
                    info!(
                        channel_id = %channel_id,
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
        remote_session.as_mut(),
    )
    .await;

    cancel.cancel();
    let _ = send_task.await;
    let _ = heartbeat_task.await;
    let _ = forward_task.await;
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
    let should_auto_end = if remote_session.is_some() {
        room.remove_peer(peer_id);
        false
    } else {
        room.remove_peer_and_check_ended(peer_id)
            .map(|(_, ended)| ended)
            .unwrap_or(false)
    };

    let left_msg = serde_json::json!({
        "type": "left",
        "pubkey": pubkey_hex,
        "peer_index": peer_index,
    })
    .to_string();
    if remote_session.is_none() {
        room.broadcast_control(left_msg);
    }

    emit_participant_event(
        &state,
        &tenant,
        Kind::Custom(48102),
        channel_id,
        parent_id_for_event,
        &pubkey_hex,
    )
    .await;

    let room_emptied;
    if should_auto_end {
        info!(channel_id = %channel_id, "audio room empty — auto-ending huddle");

        match state
            .db
            .archive_channel(tenant.community(), channel_id)
            .await
        {
            Err(e) => {
                warn!(channel_id = %channel_id, "auto-archive failed, huddle stays alive: {e}");
                room.clear_ended();
                room_emptied = false;
            }
            Ok(()) => {
                room_emptied = state
                    .audio_rooms
                    .cleanup_if_empty(tenant.community(), channel_id);

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
        room_emptied = state
            .audio_rooms
            .cleanup_if_empty(tenant.community(), channel_id);
    }

    // Owner path: release this room's lease when the room empties, so a new
    // owner can acquire and the renewer stops cleanly (silent, not owner-loss).
    // Fenced on the generation this connection saw as owner: if the room
    // emptied and a re-acquire installed a newer epoch in the gap, `release`
    // is a no-op for the stale generation and leaves the live renewer running.
    // Only the last leaver empties the room, so exactly one release fires.
    if room_emptied {
        if let (Some(mesh), Some(generation)) = (state.mesh(), owner_generation) {
            mesh.owners.release(channel_id, generation);
        }
    }

    info!(
        channel_id = %channel_id,
        pubkey = %pubkey_hex,
        "audio peer left"
    );
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
        channel_id = %channel_id,
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
    mut remote_session: Option<&mut crate::audio::join::RemoteHuddleSession>,
) {
    use crate::audio::wire::{FrameHeader, V2_HEADER_LEN};

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            msg = ws_recv.next() => {
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
                            Some(session) => session.forward_media(&data),
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

/// Outbound send loop with control-frame priority (matches connection.rs pattern).
///
/// Control frames (Ping, Pong, Close, control JSON) are drained first on every
/// iteration, so heartbeat pings are never starved by audio backpressure.
async fn send_loop(
    mut ws_send: futures_util::stream::SplitSink<WebSocket, WsMessage>,
    mut data_rx: mpsc::Receiver<WsMessage>,
    mut ctrl_rx: mpsc::Receiver<WsMessage>,
    cancel: CancellationToken,
) {
    loop {
        // Priority: drain all pending control frames before data.
        while let Ok(ctrl_msg) = ctrl_rx.try_recv() {
            if ws_send.send(ctrl_msg).await.is_err() {
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
                if ws_send.send(ctrl_msg).await.is_err() { break; }
            }
            Some(msg) = data_rx.recv() => {
                if ws_send.send(msg).await.is_err() { break; }
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
) {
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            // Control messages get priority over audio in the select.
            msg = peer_ctrl_rx.recv() => {
                match msg {
                    Some(PeerCtrl::Json(json)) => {
                        let _ = ctrl_tx.try_send(WsMessage::Text(json.into()));
                    }
                    Some(PeerCtrl::Close) | None => break,
                }
            }
            frame = audio_rx.recv() => {
                match frame {
                    Some(bytes) => {
                        let _ = data_tx.try_send(WsMessage::Binary(bytes));
                    }
                    None => break,
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
) -> Result<Uuid, String> {
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
    let is_member = state
        .is_member_cached(tenant.community(), channel_id, pubkey_bytes)
        .await
        .map_err(|e| format!("db error: {e}"))?;

    if is_member {
        return Ok(lifecycle_parent_id);
    }

    if channel.visibility == "open" {
        return Ok(lifecycle_parent_id);
    }

    // Auto-add path: private ephemeral channel + caller is member of parent.
    if channel.ttl_seconds.is_some() {
        let parent_member = state
            .is_member_cached(tenant.community(), lifecycle_parent_id, pubkey_bytes)
            .await
            .map_err(|e| format!("db error: {e}"))?;

        if parent_member {
            state
                .db
                .add_member(
                    tenant.community(),
                    channel_id,
                    pubkey_bytes,
                    MemberRole::Member,
                    Some(&channel.created_by),
                )
                .await
                .map_err(|e| format!("auto-add failed: {e}"))?;
            state.invalidate_membership(tenant, channel_id, pubkey_bytes);

            return Ok(lifecycle_parent_id);
        }
    }

    Err("not a member".into())
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

    let event_id_hex = event.id.to_hex();

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
            debug!(
                event_id = %event_id_hex,
                channel_id = %parent_channel_id,
                "audio lifecycle event already persisted — skipping fan-out"
            );
            return;
        }
        Err(e) => {
            // DB failure during disconnect cleanup. Still broadcast so live
            // subscribers see the leave/end event immediately — suppressing it
            // would leave connected clients stale. Late joiners will have an
            // inconsistent view until the next huddle lifecycle event lands.
            warn!(
                event_id = %event_id_hex,
                channel_id = %parent_channel_id,
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
        warn!(
            event_id = %event_id_hex,
            channel_id = %parent_channel_id,
            "audio: failed to publish lifecycle event: {e}"
        );
    }
}
