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
use tracing::{debug, info, warn};
use uuid::Uuid;

use buzz_auth::generate_challenge;
use buzz_db::channel::MemberRole;

use buzz_core::StoredEvent;

use crate::audio::room::PeerCtrl;
use crate::state::AppState;

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

// ── Route handler ─────────────────────────────────────────────────────────────

/// WebSocket upgrade handler for `/huddle/:channel_id/audio`.
pub async fn ws_audio_handler(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<Uuid>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_audio_connection(socket, state, channel_id))
}

// ── Auth message shape ────────────────────────────────────────────────────────

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

// ── Core connection lifecycle ─────────────────────────────────────────────────

async fn handle_audio_connection(socket: WebSocket, state: Arc<AppState>, channel_id: Uuid) {
    let (mut ws_send, mut ws_recv) = socket.split();

    // ── Step 1: send challenge ────────────────────────────────────────────────
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

    // ── Step 2: await auth (5s timeout) ──────────────────────────────────────
    let auth_result = tokio::time::timeout(AUTH_TIMEOUT, async {
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
    })
    .await;

    let auth_msg = match auth_result {
        Ok(Some(a)) => a,
        _ => {
            debug!(channel_id = %channel_id, "audio auth timeout or disconnect");
            return;
        }
    };

    // Extract NIP-OA auth tag before verify_auth_event consumes the event.
    let auth_tag_json = crate::handlers::auth::extract_auth_tag_json(&auth_msg.event);

    let relay_url = state.config.relay_url.clone();
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

    // ── Relay membership gate (with NIP-OA fallback) ────────────────────────────
    if crate::api::relay_members::enforce_relay_membership(
        &state,
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
    if let Err(e) = ensure_membership(&state, channel_id, &pubkey_bytes, parent_channel_id).await {
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

    // ── Step 4: join room ─────────────────────────────────────────────────────
    let room = state.audio_rooms.get_or_create(channel_id);

    // Re-check archived status after obtaining the room. This closes the
    // cross-boundary race: a joiner that passed ensure_membership before
    // the last peer archived the channel could get a fresh room via
    // get_or_create (the old room was already cleaned up). This DB check
    // catches that case. The room-level ended flag (checked inside add_peer)
    // handles the same-room case.
    match state.db.get_channel(channel_id).await {
        Ok(ch) if ch.archived_at.is_some() => {
            debug!(channel_id = %channel_id, "channel archived before room join");
            let _ = ws_send
                .send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"huddle has ended"})
                        .to_string()
                        .into(),
                ))
                .await;
            state.audio_rooms.cleanup_if_empty(channel_id);
            return;
        }
        Err(e) => {
            warn!(channel_id = %channel_id, "pre-join channel check failed (fail-closed): {e}");
            state.audio_rooms.cleanup_if_empty(channel_id);
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

    let (peer_id, peer_index, audio_rx, peer_ctrl_rx) = match room
        .add_peer(pubkey_hex.clone(), requested_version)
    {
        Ok(v) => v,
        Err(crate::audio::room::AdmissionError::Full) => {
            warn!(channel_id = %channel_id, "audio room full (255 peers exhausted)");
            let _ = ws_send
                    .send(WsMessage::Text(
                        serde_json::json!({"type":"error","code":"room_full","message":"peer index space exhausted"})
                            .to_string().into(),
                    ))
                    .await;
            return;
        }
        Err(crate::audio::room::AdmissionError::Ended) => {
            debug!(channel_id = %channel_id, "room ended before admission");
            let _ = ws_send
                    .send(WsMessage::Text(
                        serde_json::json!({"type":"error","code":"room_ended","message":"huddle has ended"})
                            .to_string().into(),
                    ))
                    .await;
            return;
        }
        Err(crate::audio::room::AdmissionError::VersionMismatch { pinned, requested }) => {
            info!(
                channel_id = %channel_id,
                pubkey = %pubkey_hex,
                pinned,
                requested,
                "audio: protocol version mismatch — upgrade required"
            );
            let _ = ws_send
                    .send(WsMessage::Text(
                        serde_json::json!({
                            "type": "error",
                            "code": "upgrade_required",
                            "message": format!(
                                "this huddle is using audio protocol v{pinned}; your client requested v{requested}"
                            ),
                            "pinned_version": pinned,
                            "requested_version": requested,
                        })
                        .to_string()
                        .into(),
                    ))
                    .await;
            return;
        }
    };

    info!(
        channel_id = %channel_id,
        pubkey = %pubkey_hex,
        peer_index,
        "audio peer joined"
    );

    // ── Step 5: broadcast joined + send welcome ───────────────────────────────
    let peers_snapshot: Vec<serde_json::Value> = room
        .peer_pubkeys()
        .into_iter()
        .map(|(pk, idx)| serde_json::json!({"pubkey": pk, "peer_index": idx}))
        .collect();

    let joined_msg = serde_json::json!({
        "type": "joined",
        "pubkey": pubkey_hex,
        "peer_index": peer_index,
        "peers": peers_snapshot,
    })
    .to_string();

    room.broadcast_control(joined_msg);

    // ── Step 6: emit kind:48101 (PARTICIPANT_JOINED) ──────────────────────────
    let parent_id_for_event = parent_channel_id.unwrap_or(channel_id);
    emit_participant_event(
        &state,
        Kind::Custom(48101),
        channel_id,
        parent_id_for_event,
        &pubkey_hex,
    )
    .await;

    // ── Step 7: spawn send + heartbeat loops ──────────────────────────────────
    let cancel = CancellationToken::new();
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

    // ── Step 8: audio forward loop (room channels → WS send channels) ────────
    let fwd_cancel = cancel.child_token();
    let forward_task = tokio::spawn(audio_forward_loop(
        audio_rx,
        peer_ctrl_rx,
        data_tx,
        ctrl_tx.clone(),
        fwd_cancel,
    ));

    // ── Step 9: recv loop (blocks until disconnect) ───────────────────────────
    recv_loop(
        ws_recv,
        Arc::clone(&room),
        peer_id,
        requested_version,
        ctrl_tx,
        Arc::clone(&missed_pongs),
        cancel.clone(),
    )
    .await;

    // ── Cleanup ───────────────────────────────────────────────────────────────
    cancel.cancel();
    let _ = send_task.await;
    let _ = heartbeat_task.await;
    let _ = forward_task.await;

    // Atomic remove + end check: remove_peer_and_check_ended holds the
    // AdmissionGuard lock across index recycling AND the is_empty + ended=true
    // check. This is the SAME lock that add_peer holds across its ended check
    // + insert. So they are mutually exclusive — no concurrent add_peer can
    // succeed between the removal and the ended flag being set.
    let (_, should_auto_end) = room
        .remove_peer_and_check_ended(peer_id)
        .unwrap_or((peer_index, false));

    let left_msg = serde_json::json!({
        "type": "left",
        "pubkey": pubkey_hex,
        "peer_index": peer_index,
    })
    .to_string();
    room.broadcast_control(left_msg);

    emit_participant_event(
        &state,
        Kind::Custom(48102),
        channel_id,
        parent_id_for_event,
        &pubkey_hex,
    )
    .await;

    if should_auto_end {
        info!(channel_id = %channel_id, "audio room empty — auto-ending huddle");

        match state.db.archive_channel(channel_id).await {
            Err(e) => {
                warn!(channel_id = %channel_id, "auto-archive failed, huddle stays alive: {e}");
                room.clear_ended();
            }
            Ok(()) => {
                state.audio_rooms.cleanup_if_empty(channel_id);

                emit_participant_event(
                    &state,
                    Kind::Custom(48103),
                    channel_id,
                    parent_id_for_event,
                    &pubkey_hex,
                )
                .await;
            }
        }
    } else {
        state.audio_rooms.cleanup_if_empty(channel_id);
    }

    info!(
        channel_id = %channel_id,
        pubkey = %pubkey_hex,
        "audio peer left"
    );
}

// ── Recv loop ─────────────────────────────────────────────────────────────────

async fn recv_loop(
    mut ws_recv: futures_util::stream::SplitStream<WebSocket>,
    room: Arc<crate::audio::room::Room>,
    peer_id: Uuid,
    protocol_version: u8,
    ctrl_tx: mpsc::Sender<WsMessage>,
    missed_pongs: Arc<AtomicU8>,
    cancel: CancellationToken,
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

                        room.broadcast_frame(peer_id, data);
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

// ── Send loop ─────────────────────────────────────────────────────────────────

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

// ── Audio forward loop ────────────────────────────────────────────────────────
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

// ── Heartbeat loop ────────────────────────────────────────────────────────────

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

// ── Membership helper ─────────────────────────────────────────────────────────

async fn ensure_membership(
    state: &AppState,
    channel_id: Uuid,
    pubkey_bytes: &[u8],
    parent_channel_id: Option<Uuid>,
) -> Result<(), String> {
    // Load channel first — reject archived channels before any membership check.
    // This ensures auto-ended huddles can't be rejoined by existing members.
    let channel = state
        .db
        .get_channel(channel_id)
        .await
        .map_err(|e| format!("db error: {e}"))?;

    if channel.archived_at.is_some() {
        return Err("channel is archived".into());
    }

    // Fast path: already a member.
    let is_member = state
        .is_member_cached(channel_id, pubkey_bytes)
        .await
        .map_err(|e| format!("db error: {e}"))?;

    if is_member {
        return Ok(());
    }

    if channel.visibility == "open" {
        return Ok(());
    }

    // Auto-add path: private ephemeral channel + caller is member of parent.
    //
    // TODO(security): parent_channel_id is client-supplied and unverified.
    // We don't confirm it's the *actual* parent of this ephemeral channel.
    // Security relies on the ephemeral UUID being unguessable (UUIDv4) and
    // only discoverable via the kind:48100 event in the real parent channel
    // — which requires parent membership. A future hardening pass should
    // verify the parent→ephemeral linkage by checking that a kind:48100
    // event exists in the claimed parent channel referencing this channel ID.
    if channel.ttl_seconds.is_some() {
        if let Some(parent_id) = parent_channel_id {
            let parent_member = state
                .is_member_cached(parent_id, pubkey_bytes)
                .await
                .map_err(|e| format!("db error: {e}"))?;

            if parent_member {
                state
                    .db
                    .add_member(
                        channel_id,
                        pubkey_bytes,
                        MemberRole::Member,
                        Some(&channel.created_by),
                    )
                    .await
                    .map_err(|e| format!("auto-add failed: {e}"))?;
                state.invalidate_membership(channel_id, pubkey_bytes);

                return Ok(());
            }
        }
    }

    Err("not a member".into())
}

// ── Lifecycle event helper ────────────────────────────────────────────────────

async fn emit_participant_event(
    state: &AppState,
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
    let stored = match state.db.insert_event(&event, Some(parent_channel_id)).await {
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
    state.mark_local_event(&event.id);

    // 3. Local fan-out to WS subscribers on this node (same pattern as
    //    dispatch_persistent_event in the ingest handler).
    let matches = state.sub_registry.fan_out(&stored);
    if !matches.is_empty() {
        let event_json = serde_json::to_string(&event)
            .expect("nostr::Event serialization is infallible for well-formed events");
        let mut drop_count = 0u32;
        for (target_conn_id, sub_id) in &matches {
            let msg = format!(r#"["EVENT","{}",{}]"#, sub_id, event_json);
            if !state.conn_manager.send_to(*target_conn_id, msg) {
                drop_count += 1;
            }
        }
        if drop_count > 0 {
            warn!(
                event_id = %event_id_hex,
                drop_count,
                "audio lifecycle fan-out: {drop_count} connection(s) dropped"
            );
        }
    }

    // 4. Cross-node broadcast via Redis pub/sub.
    if let Err(e) = state.pubsub.publish_event(parent_channel_id, &event).await {
        state.local_event_ids.invalidate(&event.id.to_bytes());
        warn!(
            event_id = %event_id_hex,
            channel_id = %parent_channel_id,
            "audio: failed to publish lifecycle event: {e}"
        );
    }
}
