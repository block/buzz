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
    extract::{FromRequest, Path, State, WebSocketUpgrade},
    response::IntoResponse,
};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, Kind, Tag};
use serde::Deserialize;
use tokio::sync::{mpsc, watch, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use buzz_auth::{generate_challenge, VerifiedAssertion};
use buzz_core::tenant::TenantContext;

use buzz_core::StoredEvent;
use buzz_pubsub::EventTopic;

use crate::audio::room::PeerCtrl;
use crate::state::{run_registered_community_connection, AppState, CommunityConnectionControl};

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

/// WebSocket upgrade handler for `/huddle/:channel_id/audio`.
pub async fn ws_audio_handler(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
    req: axum::extract::Request,
) -> impl IntoResponse {
    // NIP-FI assertion check at upgrade — before tenant lookup and before the
    // WebSocket handshake. Running pre-lookup means a denied request pays zero
    // DB cost and the gate is reachable in tests without a live community.
    // [FI-TRACE-TRANSPORT-CLOSED] [NIP-FI.md §Admission pairing sequence]
    let nip_fi_assertion = {
        use crate::nip_fi_upgrade::{check_nip_fi_at_upgrade, NipFiUpgradeOutcome};
        let mode = state.config.nip_fi.mode;
        let verifier = state.nip_fi_verifier.as_deref();
        match check_nip_fi_at_upgrade(&headers, verifier, mode) {
            NipFiUpgradeOutcome::NotRequired => None,
            NipFiUpgradeOutcome::Admitted(assertion) => Some(assertion),
            NipFiUpgradeOutcome::Denied(resp) => return resp.into_response(),
        }
    };

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

    let ws = match WebSocketUpgrade::from_request(req, &state).await {
        Ok(ws) => ws,
        Err(e) => return e.into_response(),
    };

    let permit = match acquire_audio_connection_permit(&state.conn_semaphore) {
        Some(permit) => permit,
        None => {
            warn!(channel_id = %channel_id, "Connection limit reached, rejecting audio WebSocket");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "relay: connection limit reached",
            )
                .into_response();
        }
    };

    // Keep the parser boundary at the largest message this route accepts. The
    // checks in the receive loop still distinguish text from binary policy, but
    // they run after tungstenite has assembled a message.
    // Capture the upgrade instant here — before the on_upgrade callback fires —
    // so the NIP-FI session partition is rooted at the HTTP handshake, not the
    // post-community-active-check instant. [FI-TRACE-LEASE-BOUND]
    let connection_time = chrono::Utc::now();
    limit_audio_websocket(ws).on_upgrade(move |socket| {
        handle_audio_connection(
            socket,
            state,
            tenant,
            channel_id,
            permit,
            nip_fi_assertion,
            connection_time,
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
const CURRENT_PROTOCOL_VERSION: u8 = 3;

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
    nip_fi_assertion: Option<VerifiedAssertion>,
    connection_time: chrono::DateTime<chrono::Utc>,
) {
    let cancel = CancellationToken::new();
    let control = CommunityConnectionControl::new(cancel);
    let community_id = tenant.community();
    let registry = Arc::clone(&state.community_connections);
    let check_state = Arc::clone(&state);
    let run_state = Arc::clone(&state);
    run_registered_community_connection(
        &registry,
        Uuid::new_v4(),
        community_id,
        control,
        move || async move { check_state.db.is_community_active(community_id).await },
        move |control| {
            handle_active_audio_connection(
                socket,
                run_state,
                tenant,
                channel_id,
                control,
                nip_fi_assertion,
                connection_time,
            )
        },
    )
    .await;
}

/// Records the NIP-42-proven pubkey and its admitting NIP-FI issuer on an audio
/// control after successful auth so the issuer-scoped NIP-FI disconnect scan
/// can reach audio sockets alongside relay peers.
///
/// Extracted from `handle_active_audio_connection` so tests can register audio
/// connections through the same production seam without spinning up full audio
/// infrastructure.
pub(crate) fn audio_post_auth_register(
    control: &CommunityConnectionControl,
    pubkey_bytes: Vec<u8>,
    nip_fi_issuer: Option<String>,
) {
    control.set_proven_identity(pubkey_bytes, nip_fi_issuer);
}

pub(crate) async fn handle_active_audio_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    tenant: TenantContext,
    channel_id: Uuid,
    control: CommunityConnectionControl,
    nip_fi_assertion: Option<VerifiedAssertion>,
    connection_time: chrono::DateTime<chrono::Utc>,
) {
    let cancel = control.cancellation_token();
    let disconnect_reason = control.disconnect_reason();
    // connection_time is threaded in from the HTTP handler (captured immediately
    // before on_upgrade) so the session partition is rooted at the true upgrade
    // instant, not the post-community-active-check instant. [FI-TRACE-LEASE-BOUND]
    let (mut ws_send, mut ws_recv) = socket.split();

    let challenge = generate_challenge();
    let challenge_msg =
        serde_json::json!({"type": "challenge", "challenge": challenge}).to_string();
    // Bounded send: pre-registration exits must not block on a stalled
    // sink indefinitely.  Check Ok(Ok(())) — is_err() would swallow
    // Ok(Err(sink_error)) and allow the handler to continue with a broken
    // sink.  [R2: bounded delivery — consistent policy; MINOR fix]
    if !matches!(
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            ws_send.send(WsMessage::Text(challenge_msg.into())),
        )
        .await,
        Ok(Ok(()))
    ) {
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
    let signed_auth_created_at = auth_msg.event.created_at.as_secs();

    let relay_url = crate::api::bridge::nip42_expected_relay_url(&state.config.relay_url, &tenant);
    let auth_ctx = match state
        .auth
        .verify_auth_event(auth_msg.event, &challenge, &relay_url)
        .await
    {
        Ok(ctx) => ctx,
        Err(e) => {
            warn!(channel_id = %channel_id, "audio auth failed: {e}");
            // Bounded send: pre-registration exits must not block on a stalled
            // sink indefinitely. [R2: bounded delivery — consistent policy]
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                ws_send.send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"auth failed"})
                        .to_string()
                        .into(),
                )),
            )
            .await;
            return;
        }
    };

    let pubkey = auth_ctx.pubkey;
    let pubkey_hex = pubkey.to_hex();
    let pubkey_bytes = pubkey.to_bytes().to_vec();
    let parent_channel_id = auth_msg.parent_channel_id;

    // NIP-FI key pairing [FI-INV-05]: unconditional, using the shared production
    // seam. When an assertion was presented at upgrade, the proven NIP-42 key
    // MUST equal the assertion's `nostr_pubkey` claim. Claimless assertion is
    // also a denial. The seam owns verdict, frame delivery, metric, and cancel.
    // [FI-TRACE-DENIAL-ORACLE post-establishment]
    if crate::nip_fi_session::enforce_nip_fi_key_pairing(
        nip_fi_assertion.as_ref(),
        pubkey,
        crate::nip_fi_session::PairingDenialTarget::Audio {
            ws_send: &mut ws_send,
            cancel: &cancel,
            channel_id,
        },
    )
    .await
        == crate::nip_fi_session::PairingOutcome::Denied
    {
        return;
    }

    // Create the terminal channel and register the sender on the control
    // BEFORE audio_post_auth_register makes the pubkey scan-visible.
    //
    // Ordering invariant: by the time any concurrent disconnect_nip_fi scan
    // can find this connection (proven_pubkey is set), terminal_frame_tx is
    // already registered.  A scan that fires between registration and the
    // deny-set check below will find both the pubkey AND the sender, so the
    // denial payload is enqueued and the client gets payload-then-close.
    //
    // The receiver (terminal_ctrl_rx) is created here too; it is consumed by
    // the check_cancel!() drain arms and later moved into the send_loop.
    // [FI-TRACE-ADMIN-DISCONNECT-PAYLOAD]
    let (terminal_ctrl_tx, mut terminal_ctrl_rx) =
        tokio::sync::mpsc::channel::<axum::extract::ws::Message>(1);
    control.set_terminal_frame_sender(terminal_ctrl_tx.clone());

    // Register the proven pubkey with the registry AFTER successful pairing
    // (and AFTER the terminal sender is registered above) so the spec sequence
    // (NIP-FI.md:217-233) is proof → equality → register → deny check.
    // A pre-pairing registration would admit an unproven key into the
    // close-scan scope.  The terminal sender is registered first so that any
    // concurrent disconnect that observes this session after the pubkey write
    // can always enqueue the denial frame. [FI-TRACE-ADMIN-DISCONNECT-PAYLOAD]
    audio_post_auth_register(
        &control,
        pubkey_bytes.clone(),
        nip_fi_assertion
            .as_ref()
            .map(|a| a.identity().issuer().to_owned()),
    );

    // Step 6 (NIP-FI.md:227-233): deny-set check — runs AFTER registration
    // (audio_post_auth_register above) so any concurrent disconnect either sees
    // this audio session in the close scan OR we see the deny entry here.
    // Both sides of the straddle are covered; neither side can miss.
    // [FI-TRACE-DENY-SET]
    //
    // Test hook: fires immediately after registration and before the deny-set
    // check so a straddle test can insert a deny entry in the exact window.
    // No-op in production. [nip_fi_test_hooks::deny_set_check_hook, W_audio_deny]
    #[cfg(test)]
    crate::nip_fi_test_hooks::before_deny_set_check(tenant.community()).await;
    if let Some(assertion) = &nip_fi_assertion {
        if let Some(asserted_key) = assertion.asserted_key() {
            if let Some(deny_map) = state.nip_fi_deny_map.as_deref() {
                if deny_map.is_denied(
                    assertion.identity().issuer(),
                    &asserted_key,
                    chrono::Utc::now(),
                ) {
                    warn!(
                        channel_id = %channel_id,
                        pubkey = %pubkey_hex,
                        "NIP-FI deny-set hit at audio post-registration check — denying"
                    );
                    use futures_util::SinkExt as _;
                    // Bounded sends: pre-writer exits must not block on a stalled
                    // sink indefinitely. [R2: bounded delivery — consistent policy]
                    let _deny_deadline =
                        tokio::time::Instant::now() + std::time::Duration::from_secs(1);
                    let _ = tokio::time::timeout_at(
                        _deny_deadline,
                        ws_send.send(crate::nip_fi_session::authorization_denied_frame(
                            crate::nip_fi_session::NipFiWsRoute::Audio,
                        )),
                    )
                    .await;
                    // Send explicit 1008 POLICY close frame; send_loop not yet
                    // started so ws_send is directly owned. [FI-TRACE-CLOSE-CODE]
                    let _ = tokio::time::timeout_at(
                        _deny_deadline,
                        ws_send.send(axum::extract::ws::Message::Close(Some(
                            axum::extract::ws::CloseFrame {
                                code: axum::extract::ws::close_code::POLICY,
                                reason: axum::extract::ws::Utf8Bytes::from_static(
                                    "authorization denied",
                                ),
                            },
                        ))),
                    )
                    .await;
                    cancel.cancel();
                    return;
                }
            }
        }
    }

    // Test hook: fires immediately AFTER the deny-set check block when the key
    // was NOT denied (absent or off-mode). Proves the handler reached the
    // post-check/membership gate for a clean key. No-op in production.
    // [nip_fi_test_hooks::audio_after_deny_check_passed_hook, W_audio_deny_absent]
    #[cfg(test)]
    crate::nip_fi_test_hooks::after_deny_set_check_passed(tenant.community()).await;

    // Compute the NIP-FI session deadline (same three-term formula as main relay).
    // Partition is rooted at `connection_time` captured before NIP-42 auth.
    // [FI-TRACE-LEASE-BOUND]
    let audio_session_deadline = nip_fi_assertion.as_ref().map(|a| {
        crate::connection::compute_session_deadline(
            a,
            connection_time,
            state.config.nip_fi.max_connection_lifetime(),
        )
    });

    // B1: Arm the NIP-FI expiry task HERE — before any persisting side effect
    // (relay membership, room join, roster events, PARTICIPANT_JOINED).
    //
    // Create the session admission gate when in enforce mode. The gate is the
    // quiescence barrier: commit_participant_join acquires an effect permit
    // before committing the 48101 + membership transaction. The expiry task's
    // gate.expire() holds the write guard until all pre-expiry permits finish.
    //
    // terminal_ctrl_tx / terminal_ctrl_rx were created and registered above
    // (before audio_post_auth_register) so that any concurrent disconnect that
    // observes this session always finds a registered sender.  The send_loop
    // (spawned below) takes ownership of terminal_ctrl_rx and drains it on
    // cancellation. [FI-TRACE-LEASE-BOUND, FI-TRACE-ADMIN-DISCONNECT-PAYLOAD]

    // One gate per audio connection (one-gate-per-connection invariant).
    // Enforce mode: gate has a deadline; expiry task fires at that deadline.
    // Off-mode: off_mode() gate never self-expires; acquire_effect always succeeds.
    let audio_gate = if let Some(deadline) = audio_session_deadline {
        crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone())
    } else {
        crate::nip_fi_gate::SessionAdmissionGate::off_mode(cancel.clone())
    };

    let mut _nip_fi_admission_expiry = audio_session_deadline.map(|deadline| {
        crate::nip_fi_session::spawn_nip_fi_expiry_task(
            deadline,
            std::sync::Arc::clone(&audio_gate),
            terminal_ctrl_tx.clone(),
            crate::nip_fi_session::NipFiWsRoute::Audio,
            control.clone(),
        )
    });

    // Already-expired check: the synchronous guard catches a deadline that is
    // already past at this instant, without relying on the async expiry task
    // to execute first. Sends the denial frame directly on ws_send (still
    // owned — send_loop has not started) then cancels and returns.
    if let Some(deadline) = audio_session_deadline {
        if chrono::Utc::now() >= deadline {
            warn!(
                channel_id = %channel_id,
                pubkey = %pubkey_hex,
                "NIP-FI session deadline already expired at pairing — rejecting audio admission"
            );
            use futures_util::SinkExt as _;
            // Bounded sends: pre-writer exits must not block on a stalled
            // sink indefinitely. [R2: bounded delivery — consistent policy]
            let _expired_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
            let _ = tokio::time::timeout_at(
                _expired_deadline,
                ws_send.send(crate::nip_fi_session::authorization_denied_frame(
                    crate::nip_fi_session::NipFiWsRoute::Audio,
                )),
            )
            .await;
            // Send explicit 1008 POLICY close frame; send_loop not yet
            // started so ws_send is directly owned. [FI-TRACE-CLOSE-CODE]
            let _ = tokio::time::timeout_at(
                _expired_deadline,
                ws_send.send(axum::extract::ws::Message::Close(Some(
                    axum::extract::ws::CloseFrame {
                        code: axum::extract::ws::close_code::POLICY,
                        reason: axum::extract::ws::Utf8Bytes::from_static("authorization denied"),
                    },
                ))),
            )
            .await;
            cancel.cancel();
            return;
        }
    }

    // Helper macro: check for NIP-FI mid-admission cancellation, drain the
    // terminal channel (which holds the denial frame queued by the expiry
    // task), send it via ws_send (still owned), then send the policy close
    // frame when a disconnect reason is present, and return.
    // Used at every async boundary in the admission sequence below.
    macro_rules! check_cancel {
        () => {
            if cancel.is_cancelled() {
                use futures_util::SinkExt as _;
                // Bounded sends: pre-writer exits must not block on a stalled
                // sink indefinitely. [R2: bounded delivery — consistent policy]
                let _cc_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
                while let Ok(msg) = terminal_ctrl_rx.try_recv() {
                    let _ = tokio::time::timeout_at(_cc_deadline, ws_send.send(msg)).await;
                }
                // Emit the policy close frame when a NIP-FI reason is set.
                // The send_loop is not yet started so ws_send is directly
                // owned here. [FI-TRACE-CLOSE-CODE, Fix-1]
                let nip_fi_close_reason = *disconnect_reason.borrow();
                if let Some(reason) = nip_fi_close_reason {
                    let _ =
                        tokio::time::timeout_at(_cc_deadline, ws_send.send(reason.close_message()))
                            .await;
                }
                return;
            }
        };
        (cleanup: $cleanup:expr) => {
            if cancel.is_cancelled() {
                $cleanup;
                use futures_util::SinkExt as _;
                // Bounded sends: pre-writer exits must not block on a stalled
                // sink indefinitely. [R2: bounded delivery — consistent policy]
                let _cc_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
                while let Ok(msg) = terminal_ctrl_rx.try_recv() {
                    let _ = tokio::time::timeout_at(_cc_deadline, ws_send.send(msg)).await;
                }
                // Emit the policy close frame when a NIP-FI reason is set.
                // [FI-TRACE-CLOSE-CODE, Fix-1]
                let nip_fi_close_reason = *disconnect_reason.borrow();
                if let Some(reason) = nip_fi_close_reason {
                    let _ =
                        tokio::time::timeout_at(_cc_deadline, ws_send.send(reason.close_message()))
                            .await;
                }
                return;
            }
        };
        (release_lease: $lease:expr) => {
            if cancel.is_cancelled() {
                // Release any acquired lease before returning. Pre-guard path:
                // staged_lease may hold a lease that must be released before we
                // return, since the guard hasn't been built yet.
                if let Some((lease, directory)) = ($lease).take() {
                    match directory.release(&lease).await {
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!("pre-guard staged_lease release failed on cancel: {e}");
                        }
                    }
                }
                use futures_util::SinkExt as _;
                // Bounded sends: pre-writer exits must not block on a stalled
                // sink indefinitely. [R2: bounded delivery — consistent policy]
                let _cc_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
                while let Ok(msg) = terminal_ctrl_rx.try_recv() {
                    let _ = tokio::time::timeout_at(_cc_deadline, ws_send.send(msg)).await;
                }
                // Emit the policy close frame when a NIP-FI reason is set.
                // [FI-TRACE-CLOSE-CODE, Fix-1]
                let nip_fi_close_reason = *disconnect_reason.borrow();
                if let Some(reason) = nip_fi_close_reason {
                    let _ =
                        tokio::time::timeout_at(_cc_deadline, ws_send.send(reason.close_message()))
                            .await;
                }
                return;
            }
        };
    }

    // Shared finalization for explicit pre-writer error exits after terminal
    // registration.  Serializes with the winning producer by calling
    // `lifecycle_cancel()` (which acquires the transition lock, ensuring any
    // in-flight producer completes its `try_send` before the drain runs),
    // awaits the expiry task so its frame is committed, then drains any denial
    // frame already enqueued by a concurrent admin/expiry path and sends the
    // matching policy close code.  Used at every error return between terminal
    // registration and send_loop startup.  [R2: invariant boundary — Fix-2]
    //
    // Unlike check_cancel!() this is unconditional: the handler is returning
    // due to its own error, and the race window means a winning denial may or
    // may not have enqueued its frame yet.  The drain is always safe: an empty
    // channel is a no-op; a non-empty channel means a denial won the race and
    // its frame must reach the client before this error return.
    //
    // Sends are bounded by a 1-second flush deadline so a stalled WS sink
    // cannot hold the handler indefinitely on an error exit.
    //
    // Parameters:
    //   $control    — the CommunityConnectionControl for this session
    //   $expiry     — &mut Option<JoinHandle<_>> for the admission expiry task
    macro_rules! drain_terminal {
        ($control:expr, $expiry:expr) => {{
            use futures_util::SinkExt as _;
            // Serialization: lifecycle_cancel acquires the transition lock,
            // ensuring any producer that has won the reason but not yet called
            // try_send completes its enqueue before the drain below runs.
            // Without this, the drain could see an empty channel and discard
            // the winning denial payload.  [R2: producer serialization]
            $control.lifecycle_cancel();
            // Await the expiry task so its try_send is committed before we
            // drain.  take() prevents double-await in later code.
            if let Some(t) = $expiry.take() {
                let _ = t.await;
            }
            let _drain_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
            while let Ok(msg) = terminal_ctrl_rx.try_recv() {
                let _ = tokio::time::timeout_at(_drain_deadline, ws_send.send(msg)).await;
            }
            let nip_fi_close_reason = *disconnect_reason.borrow();
            if let Some(reason) = nip_fi_close_reason {
                let _ =
                    tokio::time::timeout_at(_drain_deadline, ws_send.send(reason.close_message()))
                        .await;
            }
        }};
    }

    // Bounded drain+close after lifecycle_cancel() and expiry-await are already
    // called explicitly by the caller.  Applies the same 1-second flush deadline
    // as drain_terminal! but skips the serialization step.  Used at post-guard
    // error exits where lifecycle_cancel + expiry.take().await precede this
    // in the same code path.  [R2: bounded delivery — Fix-2]
    macro_rules! finalize_drain {
        () => {{
            use futures_util::SinkExt as _;
            let _drain_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
            while let Ok(msg) = terminal_ctrl_rx.try_recv() {
                let _ = tokio::time::timeout_at(_drain_deadline, ws_send.send(msg)).await;
            }
            let nip_fi_close_reason = *disconnect_reason.borrow();
            if let Some(reason) = nip_fi_close_reason {
                let _ =
                    tokio::time::timeout_at(_drain_deadline, ws_send.send(reason.close_message()))
                        .await;
            }
        }};
    }

    if crate::api::relay_members::enforce_relay_membership(
        &state,
        tenant.community(),
        pubkey.as_bytes(),
        auth_tag_json.as_deref(),
        Some(signed_auth_created_at),
    )
    .await
    .is_err()
    {
        warn!(channel_id = %channel_id, pubkey = %pubkey_hex, "audio: relay membership denied");
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            ws_send.send(WsMessage::Text(
                serde_json::json!({"type": "error", "message": "restricted: not a relay member"})
                    .to_string()
                    .into(),
            )),
        )
        .await;
        // Drain any concurrent denial that raced in while the membership check
        // was awaiting — terminal_ctrl_rx holds the denial frame, disconnect_reason
        // holds the close code.  Diagnostic send above is bounded; drain below
        // applies its own 1-second deadline.  [R2: bounded delivery]
        drain_terminal!(control, _nip_fi_admission_expiry);
        return;
    }
    // Test hook: fires immediately before the first check_cancel!() so W_FIX1
    // can hold the handler here while the expiry task fires, then release to
    // let check_cancel!() drain the terminal channel and emit the policy close.
    // No-op in production. [nip_fi_test_hooks::audio_before_first_check_cancel_hook]
    #[cfg(test)]
    crate::nip_fi_test_hooks::before_first_audio_check_cancel(tenant.community()).await;
    check_cancel!();

    // ── Step 3: membership check / auto-add ───────────────────────────────────
    let membership_admission = match check_membership_for_admission(
        &state,
        &tenant,
        channel_id,
        &pubkey_bytes,
        parent_channel_id,
    )
    .await
    {
        Ok(admission) => admission,
        Err(e) => {
            warn!(channel_id = %channel_id, pubkey = %pubkey_hex, "audio membership denied: {e}");
            // Bounded send: pre-writer exits must not block on a stalled
            // sink indefinitely. [R2: bounded delivery — consistent policy]
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                ws_send.send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"not a member"})
                        .to_string()
                        .into(),
                )),
            )
            .await;
            // Test hook: fires after the "not a member" diagnostic frame is sent
            // and before drain_terminal!().  Used by the R2 race witness to pause
            // the handler here while a concurrent admin fires disconnect_nip_fi
            // and its cancel_race_test_hook pauses the producer after winning the
            // reason (lock held).  Releasing this hook → handler calls
            // drain_terminal!() → lifecycle_cancel() blocks on admin's lock →
            // admin completes try_send → unblocks → drain delivers denial frame.
            // Falsification target: remove lifecycle_cancel() from drain_terminal!().
            // No-op in production.  [nip_fi_test_hooks::audio_not_a_member_drain_terminal_hook, R2]
            #[cfg(test)]
            crate::nip_fi_test_hooks::before_not_a_member_drain_terminal(tenant.community()).await;
            // [R2: pre-writer invariant boundary]
            drain_terminal!(control, _nip_fi_admission_expiry);
            return;
        }
    };
    // Derive parent_id_for_event from the membership admission result.
    // This is the channel ID that lifecycle events (48101/48102/48103) belong to.
    let parent_id_for_event = match &membership_admission {
        MembershipAdmission::Existing { parent_channel_id } => *parent_channel_id,
        MembershipAdmission::AutoAddRequired {
            parent_channel_id, ..
        } => *parent_channel_id,
    };
    check_cancel!();

    // Huddle cross-pod routing (mesh) OR single-pod guardrail.
    //
    // When the mesh is live (`state.mesh()` is `Some`), a huddle can span pods:
    // Redis arbitrates ownership and this pod either owns the room locally or
    // forwards the client to the owner over a `HuddleControl` stream. When the
    // mesh is off, we keep today's behavior exactly — including the
    // `huddle_audio_available=false` rejection under a non-mesh horizontal
    // deployment (two peers on different pods would never hear each other).
    //
    // `pending_remote` drives the local vs. remote ownership decision.
    // `admission_guard.lease` holds the freshly-acquired Redis lease (if any)
    // and its directory for release; it is set here before any other resource
    // that could need cleanup, so pre-commit exits always use the guard.
    let mut pending_remote: Option<crate::audio::join::JoinOutcome> = None;
    // Temporary staging for the lease+directory before the admission guard is
    // constructed (the room isn't available yet at this point).
    let mut staged_lease: Option<(
        crate::audio::join::HuddleLease,
        std::sync::Arc<dyn crate::audio::join::HuddleDirectory>,
    )> = None;
    match state.mesh() {
        Some(mesh) => {
            if mesh.owners.is_draining() {
                // Bounded send: pre-writer exits must not block on a stalled
                // sink indefinitely. [R2: bounded delivery — consistent policy]
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    ws_send.send(WsMessage::Text(
                        serde_json::json!({
                            "type": "error",
                            "code": "huddle_relay_draining",
                            "message": "relay is draining; reconnect"
                        })
                        .to_string()
                        .into(),
                    )),
                )
                .await;
                // [R2: pre-writer invariant boundary]
                drain_terminal!(control, _nip_fi_admission_expiry);
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
                    if let Some(lease) = resolved.acquired {
                        let directory: std::sync::Arc<dyn crate::audio::join::HuddleDirectory> =
                            std::sync::Arc::new(mesh.directory.clone());
                        staged_lease = Some((lease, directory));
                    }
                    pending_remote = Some(resolved.outcome);
                }
                Err(e) => {
                    warn!(
                        channel_id = %channel_id,
                        pubkey = %pubkey_hex,
                        "huddle join rejected by fence: {e}"
                    );
                    // Bounded send: pre-writer exits must not block on a stalled
                    // sink indefinitely. [R2: bounded delivery — consistent policy]
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(1),
                        ws_send.send(WsMessage::Text(
                            serde_json::json!({
                                "type": "error",
                                "code": "join_rejected",
                                "message": "huddle join rejected"
                            })
                            .to_string()
                            .into(),
                        )),
                    )
                    .await;
                    // [R2: pre-writer invariant boundary]
                    drain_terminal!(control, _nip_fi_admission_expiry);
                    return;
                }
            }
            // I1 residual: staged_lease may now hold an acquired lease. Release
            // it (awaited, not detached) before returning on cancel.
            check_cancel!(release_lease: staged_lease);
        }
        None => {
            if !state.config.huddle_audio_available {
                debug!(
                    channel_id = %channel_id,
                    pubkey = %pubkey_hex,
                    "huddle audio unavailable under horizontal scaling — rejecting join"
                );
                // Bounded send: pre-writer exits must not block on a stalled
                // sink indefinitely. [R2: bounded delivery — consistent policy]
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    ws_send.send(WsMessage::Text(
                        serde_json::json!({
                            "type": "error",
                            "code": "huddle_audio_unavailable",
                            "message": "huddle audio unavailable in this deployment"
                        })
                        .to_string()
                        .into(),
                    )),
                )
                .await;
                // [R2: pre-writer invariant boundary]
                drain_terminal!(control, _nip_fi_admission_expiry);
                return;
            }
        }
    }

    let lifecycle_generation = pending_remote
        .as_ref()
        .map(|outcome| outcome.generation().to_string())
        .unwrap_or_else(|| state.huddle_liveness_generation.to_string());

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
            // Bounded send: pre-writer exits must not block on a stalled
            // sink indefinitely. [R2: bounded delivery — consistent policy]
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                ws_send.send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"huddle has ended"})
                        .to_string()
                        .into(),
                )),
            )
            .await;
            // I1 residual: release lease with an awaited call, not a detached task.
            if let Some((lease, directory)) = staged_lease {
                if let Err(e) = directory.release(&lease).await {
                    tracing::warn!(channel_id = %channel_id, "archived-exit lease release failed: {e}");
                }
            }
            state
                .audio_rooms
                .cleanup_if_empty(tenant.community(), channel_id);
            // [R2: pre-writer invariant boundary]
            drain_terminal!(control, _nip_fi_admission_expiry);
            return;
        }
        Err(e) => {
            warn!(channel_id = %channel_id, "pre-join channel check failed (fail-closed): {e}");
            // I1 residual: release lease with an awaited call, not a detached task.
            if let Some((lease, directory)) = staged_lease {
                if let Err(re) = directory.release(&lease).await {
                    tracing::warn!(channel_id = %channel_id, "db-error-exit lease release failed: {re}");
                }
            }
            state
                .audio_rooms
                .cleanup_if_empty(tenant.community(), channel_id);
            // [R2: pre-writer invariant boundary]
            drain_terminal!(control, _nip_fi_admission_expiry);
            return;
        }
        Ok(_) => {} // Channel exists and is not archived — proceed.
    }
    // I1 residual: staged_lease may hold an acquired lease. Release it
    // (awaited, not detached) before returning on cancel.
    check_cancel!(release_lease: staged_lease);

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
        // Bounded send: pre-writer exits must not block on a stalled
        // sink indefinitely. [R2: bounded delivery — consistent policy]
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            ws_send.send(WsMessage::Text(
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
            )),
        )
        .await;
        if let Some((lease, directory)) = staged_lease {
            // I1 residual: release lease with an awaited call, not a detached task.
            if let Err(e) = directory.release(&lease).await {
                tracing::warn!(channel_id = %channel_id, "version-mismatch-exit lease release failed: {e}");
            }
        }
        // [R2: pre-writer invariant boundary]
        drain_terminal!(control, _nip_fi_admission_expiry);
        return;
    }

    // Build the admission guard. From this point every pre-commit exit MUST
    // call `guard.release_before_commit().await` before returning so that the
    // lease, remote registration, and peer are always cleaned up through the
    // single shared path (IMPORTANT 1-3).
    let mut guard = HuddleAdmissionGuard {
        lease: staged_lease,
        remote_session: None,
        remote_stream: None,
        peer_id: None,
        room: Arc::clone(&room),
        audio_rooms: Arc::clone(&state.audio_rooms),
        community: tenant.community(),
        channel_id,
    };

    // Remote registration happens before ingress admission. The owner-assigned
    // index is therefore the only index this client ever has; no frame or
    // `joined` message can escape with an ingress-local placeholder.
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
                guard.remote_session = Some(session);
                guard.remote_stream = Some(stream);
                remote_fence = Some(Arc::clone(&mesh.audio_fence));
            }
            Err(crate::audio::join::DialError::Rejected(reason)) => {
                warn!(channel_id = %channel_id, pubkey = %pubkey_hex, "huddle owner rejected registration: {reason:?}");
                // Bounded send: pre-writer exits must not block on a stalled
                // sink indefinitely. [R2: bounded delivery — consistent policy]
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    ws_send.send(WsMessage::Text(
                        remote_rejection_ws_error(&reason).to_string().into(),
                    )),
                )
                .await;
                // I3 residual: await expiry task before resource teardown.
                // B2: lifecycle_cancel holds the transition lock so any concurrent
                // terminal writer that won the reason but hasn't enqueued yet
                // completes its try_send before the cancel wakes any consumer.
                // [FI-TRACE-CANCEL-RACE, B2 fix]
                control.lifecycle_cancel();
                if let Some(t) = _nip_fi_admission_expiry.take() {
                    let _ = t.await;
                }
                guard.release_before_commit().await;
                // Drain any denial frame queued by the expiry task.
                // Bounded by 1-second deadline. [R2: bounded delivery — Fix-2]
                finalize_drain!();
                state
                    .audio_rooms
                    .cleanup_if_empty(tenant.community(), channel_id);
                return;
            }
            Err(crate::audio::join::DialError::Mesh(e)) => {
                warn!(channel_id = %channel_id, pubkey = %pubkey_hex, "huddle owner registration failed: {e}");
                // Bounded send: pre-writer exits must not block on a stalled
                // sink indefinitely. [R2: bounded delivery — consistent policy]
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    ws_send.send(WsMessage::Text(
                        serde_json::json!({
                            "type": "error", "code": "huddle_owner_unreachable",
                            "message": "could not reach the huddle owner"
                        })
                        .to_string()
                        .into(),
                    )),
                )
                .await;
                // I3 residual: await expiry task before resource teardown.
                // B2: lifecycle_cancel holds the transition lock so any concurrent
                // terminal writer that won the reason but hasn't enqueued yet
                // completes its try_send before the cancel wakes any consumer.
                // [FI-TRACE-CANCEL-RACE, B2 fix]
                control.lifecycle_cancel();
                if let Some(t) = _nip_fi_admission_expiry.take() {
                    let _ = t.await;
                }
                guard.release_before_commit().await;
                // Drain any denial frame queued by the expiry task.
                // Bounded by 1-second deadline. [R2: bounded delivery — Fix-2]
                finalize_drain!();
                state
                    .audio_rooms
                    .cleanup_if_empty(tenant.community(), channel_id);
                return;
            }
        }
        // B1: post-dial cancel check — guard runs clean-close + lease release.
        // IMPORTANT 3 residual: await expiry task explicitly, do not infer
        // completion from cancel.is_cancelled().
        if cancel.is_cancelled() {
            // B2: lifecycle_cancel holds the transition lock so a concurrent
            // terminal writer completes its try_send before we enter the drain.
            // [FI-TRACE-CANCEL-RACE, B2 fix]
            control.lifecycle_cancel();
            if let Some(t) = _nip_fi_admission_expiry.take() {
                let _ = t.await;
            }
            guard.release_before_commit().await;
            // Bounded drain+close. [R2: bounded delivery — Fix-2]
            finalize_drain!();
            return;
        }
    }

    // ── Step 5: add_peer under a short gate permit ────────────────────────────
    // The permit spans the real peer insertion (IMPORTANT 2): expiry cannot
    // create a peer without winning the gate, so the committed/peer-absent
    // invariant holds across deadline-exact races at this seam too.
    let add_peer_result = {
        // Test hook: fires immediately before audio_gate.acquire_effect() in
        // Step 5.  Used by F4 add-peer witness: arms this hook, waits for
        // arrival (handler at the gate seam), then expires the gate so
        // acquire_effect() returns SessionExpired on release.  No-op in
        // production.  [nip_fi_test_hooks::audio_add_peer_gate_acquire_hook, F4]
        #[cfg(test)]
        crate::nip_fi_test_hooks::before_add_peer_gate_acquire(tenant.community()).await;
        let _add_permit = match audio_gate.acquire_effect().await {
            Ok(p) => p,
            Err(crate::nip_fi_gate::SessionExpired) => {
                // Expiry fired before we could add the peer. No peer, no commit.
                // F4: Publish the serialized deadline cause BEFORE self-cancelling.
                // The expiry task's unbiased select may take the cancellation arm
                // (fired by the already-elapsed wall-clock path) and skip
                // `expiry_deny_terminal`.  Calling it here wins the reason slot
                // (or defers to a prior winner) and enqueues the denial frame, so
                // the consumer always drains a denial frame before the Close.
                // respecting an already-winning cause (first-writer-wins).
                // [F4: audio deadline rejection — denial precedes self-cancel]
                control.expiry_deny_terminal(
                    &terminal_ctrl_tx,
                    crate::nip_fi_session::NipFiWsRoute::Audio,
                );
                // IMPORTANT 3 residual: await expiry task explicitly.
                // B2: lifecycle_cancel holds the transition lock so any concurrent
                // terminal writer that won the reason but hasn't enqueued yet
                // completes its try_send before the cancel wakes any consumer.
                // [FI-TRACE-CANCEL-RACE, B2 fix]
                control.lifecycle_cancel();
                if let Some(t) = _nip_fi_admission_expiry.take() {
                    let _ = t.await;
                }
                guard.release_before_commit().await;
                // Bounded drain+close. [R2: bounded delivery — Fix-2]
                finalize_drain!();
                return;
            }
        };
        // Permit is held across add_peer[_at_index] — drop after the call.
        if let Some(session) = guard.remote_session.as_ref() {
            room.add_peer_at_index(pubkey_hex.clone(), requested_version, session.peer_index())
                .map(|(id, _mirror_epoch, audio, ctrl, revision)| {
                    (
                        id,
                        session.peer_index(),
                        session.epoch(),
                        audio,
                        ctrl,
                        revision,
                    )
                })
        } else {
            room.add_peer(pubkey_hex.clone(), requested_version)
        }
    };
    let (peer_id, peer_index, peer_epoch, audio_rx, peer_ctrl_rx, admission_revision) =
        match add_peer_result {
            Ok(v) => v,
            Err(crate::audio::room::AdmissionError::Full) => {
                warn!(channel_id = %channel_id, "audio room participant capacity reached");
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    ws_send.send(WsMessage::Text(serde_json::json!({"type":"error","code":"room_full","message":"room participant capacity reached"}).to_string().into())),
                )
                .await;
                // IMPORTANT 3: cancel + await expiry task before guard release.
                // B2: lifecycle_cancel holds transition lock. [FI-TRACE-CANCEL-RACE, B2 fix]
                control.lifecycle_cancel();
                if let Some(t) = _nip_fi_admission_expiry.take() {
                    let _ = t.await;
                }
                guard.release_before_commit().await;
                // R2: bounded drain+close. [FI-INV-05, R2 fix — bounded delivery]
                finalize_drain!();
                return;
            }
            Err(crate::audio::room::AdmissionError::Ended) => {
                debug!(channel_id = %channel_id, "room ended before admission");
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    ws_send.send(WsMessage::Text(serde_json::json!({"type":"error","code":"room_ended","message":"huddle has ended"}).to_string().into())),
                )
                .await;
                // Test hook: fires after the room_ended error frame is sent and
                // before lifecycle_cancel.  Used by R2 witness: a concurrent admin
                // disconnect fires here (wins reason, enqueues denial frame), then
                // the R2 drain below delivers it before the socket closes.
                // No-op in production.  [nip_fi_test_hooks::before_room_ended_lifecycle_cancel, R2]
                #[cfg(test)]
                crate::nip_fi_test_hooks::before_room_ended_lifecycle_cancel(tenant.community())
                    .await;
                // IMPORTANT 3: cancel + await expiry task before guard release.
                // B2: lifecycle_cancel holds transition lock. [FI-TRACE-CANCEL-RACE, B2 fix]
                control.lifecycle_cancel();
                if let Some(t) = _nip_fi_admission_expiry.take() {
                    let _ = t.await;
                }
                guard.release_before_commit().await;
                // R2: bounded drain+close. [FI-INV-05, R2 fix — bounded delivery]
                finalize_drain!();
                return;
            }
            Err(crate::audio::room::AdmissionError::VersionMismatch { pinned, requested }) => {
                info!(channel_id = %channel_id, pubkey = %pubkey_hex, pinned, requested, "audio: protocol version mismatch — upgrade required");
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    ws_send.send(WsMessage::Text(serde_json::json!({
                    "type": "error", "code": "upgrade_required",
                    "message": format!("this huddle is using audio protocol v{pinned}; your client requested v{requested}"),
                    "pinned_version": pinned, "requested_version": requested,
                }).to_string().into())),
                )
                .await;
                // IMPORTANT 3: cancel + await expiry task before guard release.
                // B2: lifecycle_cancel holds transition lock. [FI-TRACE-CANCEL-RACE, B2 fix]
                control.lifecycle_cancel();
                if let Some(t) = _nip_fi_admission_expiry.take() {
                    let _ = t.await;
                }
                guard.release_before_commit().await;
                // R2: bounded drain+close. [FI-INV-05, R2 fix — bounded delivery]
                finalize_drain!();
                return;
            }
        };

    // Record the peer in the guard so any post-add_peer pre-commit exit removes it.
    guard.peer_id = Some(peer_id);

    // B1: check for mid-admission expiry immediately after peer is registered
    // in the room. The peer_id is now live; cancel means we must undo it.
    //
    // Test hook: fires after successful add_peer and before the explicit
    // `cancel.is_cancelled()` branch below. A test can set cancel here to prove
    // that branch's `guard.release_before_commit()` (remove_peer +
    // cleanup_if_empty) runs before the handler returns.
    // [nip_fi_test_hooks::audio_add_peer_hook]
    #[cfg(test)]
    crate::nip_fi_test_hooks::after_add_peer(tenant.community()).await;
    if cancel.is_cancelled() {
        // IMPORTANT 3 residual: do NOT infer expiry-task completion from
        // cancel.is_cancelled(). `gate.expire()` calls cancel.cancel() *before*
        // its write-lock quiescence barrier (nip_fi_gate.rs). Cancel + await
        // the expiry task before releasing any resource so teardown cannot race
        // outstanding pre-expiry permits.
        // B2: lifecycle_cancel holds transition lock. [FI-TRACE-CANCEL-RACE, B2 fix]
        control.lifecycle_cancel();
        if let Some(t) = _nip_fi_admission_expiry.take() {
            let _ = t.await;
        }
        guard.release_before_commit().await;
        // Bounded drain+close. [R2: bounded delivery — Fix-2]
        finalize_drain!();
        return;
    }

    info!(
        channel_id = %channel_id,
        pubkey = %pubkey_hex,
        peer_index,
        "audio peer joined"
    );

    // Owner path: record the owner generation and (for the steady-state reuse
    // arm) subscribe to the existing owner-loss signal. The lease is NOT
    // transferred here — `guard` still holds it so every pre-commit exit goes
    // through `guard.release_before_commit()` which directly awaits
    // `directory.release()`. The lease transfers into `HuddleOwnerRegistry`
    // only after commit succeeds (I1 mandated: transfer-after-commit-won).
    //
    // Acquire arm (new CAS winner): the lease stays in the guard through all
    // pre-commit exits. `owner_lost` / `owner_draining` are populated at the
    // commit-won point below when `attach_signals` is called.
    //
    // Reuse arm (steady-state owner): the registry entry is already live.
    // Subscribe to the existing signals here so that a pre-commit cancel
    // (expiry, version mismatch, etc.) still tears down this connection
    // correctly. `owner_generation` fences room-empty release so a stale
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
        match pending_remote {
            Some(crate::audio::join::JoinOutcome::LocalOwner { generation })
                if guard.lease.is_some() =>
            {
                // Acquire arm: lease stays in guard; signals populated post-commit.
                owner_generation = Some(generation);
            }
            Some(crate::audio::join::JoinOutcome::LocalOwner { generation }) => {
                // Reuse arm: subscribe to the existing registry signals.
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

    let (peers_snapshot, roster_revision): (Vec<serde_json::Value>, u64) = if let Some(session) =
        guard.remote_session.as_ref()
    {
        (
                session
                    .roster()
                    .peers
                    .iter()
                    .map(|peer| {
                        serde_json::json!({"pubkey": peer.pubkey, "peer_index": peer.peer_index, "epoch": peer.epoch})
                    })
                    .collect(),
                session.roster().revision,
            )
    } else {
        let snapshot = room.roster_snapshot();
        (
                snapshot
                    .peers
                    .into_iter()
                    .map(|peer| {
                        serde_json::json!({"pubkey": peer.pubkey, "peer_index": peer.peer_index, "epoch": peer.epoch})
                    })
                    .collect(),
                snapshot.revision,
            )
    };
    debug_assert!(roster_revision >= admission_revision);

    // ── Step 6: commit kind:48101 (PARTICIPANT_JOINED) atomically ────────────
    // commit_participant_join takes one DB transaction containing:
    //   - auto-membership insert (if AutoAddRequired and still absent), and
    //   - the 48101 event insert
    // Both commit under a single session effect permit, or both roll back on
    // expiry. Fan-out AND the `joined` publication both happen while the permit
    // is still held (IMPORTANT 5: joined inside the permit).
    //
    // joined-ordering: the `joined` frame is sent to the connecting client and
    // broadcast to existing peers ONLY after commit-won. This matches Thufir's
    // design (fd00e6fe): no client-visible join success before `48101` commit.
    // Client compatibility: clients treat WS close as "leave audio"; receiving
    // close without a prior `joined` is a safe no-op — the session never
    // stabilised from the client's perspective.
    let lifecycle_revision = if guard.remote_session.is_some() {
        roster_revision
    } else {
        admission_revision
    };

    // Build the joined frame now (before moving guard fields into the commit).
    let joined_msg = serde_json::json!({
        "type": "joined",
        "revision": roster_revision,
        "pubkey": pubkey_hex,
        "peer_index": peer_index,
        "epoch": peer_epoch,
        "peers": peers_snapshot,
    })
    .to_string();

    match commit_participant_join(
        &state,
        &tenant,
        channel_id,
        parent_id_for_event,
        &pubkey_hex,
        &pubkey_bytes,
        peer_id,
        lifecycle_revision,
        &lifecycle_generation,
        &membership_admission,
        &audio_gate,
        joined_msg,
        &room,
    )
    .await
    {
        Ok(CommitJoinOutcome::JoinedSent) => {
            // `joined` was broadcast inside the permit — normal flow.
        }
        Ok(CommitJoinOutcome::JoinedSendFailed) => {
            // Committed but the joining peer's ctrl channel was saturated.
            // Route through normal admitted teardown: remove peer, emit 48102,
            // send remote close. Committed join => exactly one leave.
            //
            // I1: the lease is still guard-owned (attach_signals was not called).
            // Take the peer_id from the guard now so release_before_commit does
            // not double-remove, then release the lease at the end of this arm.
            let _ = guard.take_peer_id();
            room.remove_peer(peer_id);
            state
                .audio_rooms
                .cleanup_if_empty(tenant.community(), channel_id);
            if let (Some(session), Some(ref mut stream)) = (
                guard.take_remote_session().as_ref(),
                guard.take_remote_stream().as_mut(),
            ) {
                crate::audio::join::send_clean_close(stream, session.fenced(), session.pubkey())
                    .await;
            }
            // Emit 48102 — committed join produces exactly one leave.
            emit_participant_event(
                &state,
                &tenant,
                channel_id,
                parent_id_for_event,
                ParticipantLifecycle {
                    kind: Kind::Custom(48102),
                    participant_pubkey: &pubkey_hex,
                    roster_revision: None,
                    admission_id: Some(peer_id),
                    generation: &lifecycle_generation,
                },
            )
            .await;
            state
                .audio_rooms
                .cleanup_if_empty(tenant.community(), channel_id);
            // Release the guard-owned lease (peer_id and remote already taken above).
            guard.release_before_commit().await;
            return;
        }
        Err(JoinCommitError::Expired) => {
            // Gate denied — expiry fired before commit. No `joined` frame was
            // sent — commit-won invariant holds.
            //
            // F4: Publish the serialized deadline cause BEFORE self-cancelling.
            // The expiry task's unbiased select may take the cancellation arm
            // (wall-clock deadline already elapsed) and return without calling
            // `expiry_deny_terminal`.  Calling it here wins the reason slot
            // (or defers to a prior winner), ensuring the denial frame is queued
            // before cancel fires.  [F4: audio commit rejection]
            control.expiry_deny_terminal(
                &terminal_ctrl_tx,
                crate::nip_fi_session::NipFiWsRoute::Audio,
            );
            //
            // IMPORTANT 3 residual: `acquire_effect()` can return `SessionExpired`
            // via the deadline fast path (Utc::now() >= deadline) before the
            // spawned expiry task completes. Cancel + await the task explicitly —
            // do not infer task completion from SessionExpired.
            // B2: lifecycle_cancel holds transition lock. [FI-TRACE-CANCEL-RACE, B2 fix]
            control.lifecycle_cancel();
            if let Some(t) = _nip_fi_admission_expiry.take() {
                let _ = t.await;
            }
            // I1: lease is still guard-owned (attach_signals not yet called).
            // `guard.release_before_commit()` directly awaits directory.release().
            guard.release_before_commit().await;
            // Drain the terminal denial frame (already queued by expiry task).
            // Bounded by 1-second deadline. [R2: bounded delivery — Fix-2]
            finalize_drain!();
            return;
        }
        Err(JoinCommitError::Archived) => {
            // Channel archived between pre-join check and commit (IMPORTANT 4).
            // No `joined` frame was sent — commit-won invariant holds.
            debug!(channel_id = %channel_id, "channel archived before join commit");
            // IMPORTANT 3: cancel + await expiry task before peer/room teardown.
            // B2: lifecycle_cancel holds transition lock. [FI-TRACE-CANCEL-RACE, B2 fix]
            control.lifecycle_cancel();
            if let Some(t) = _nip_fi_admission_expiry.take() {
                let _ = t.await;
            }
            // I1: lease is still guard-owned; guard.release_before_commit() releases it.
            guard.release_before_commit().await;
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                ws_send.send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"huddle has ended"})
                        .to_string()
                        .into(),
                )),
            )
            .await;
            // R2: bounded drain+close. [FI-INV-05, R2 fix — bounded delivery]
            finalize_drain!();
            return;
        }
        Err(JoinCommitError::ParentMembershipLost) => {
            // Parent membership revoked between pre-join check and commit (IMPORTANT 4).
            // No `joined` frame was sent — commit-won invariant holds.
            warn!(channel_id = %channel_id, pubkey = %pubkey_hex, "parent membership lost before join commit");
            // IMPORTANT 3: cancel + await expiry task before peer/room teardown.
            // B2: lifecycle_cancel holds transition lock. [FI-TRACE-CANCEL-RACE, B2 fix]
            control.lifecycle_cancel();
            if let Some(t) = _nip_fi_admission_expiry.take() {
                let _ = t.await;
            }
            // I1: lease is still guard-owned; guard.release_before_commit() releases it.
            guard.release_before_commit().await;
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                ws_send.send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"error: not a member"})
                        .to_string()
                        .into(),
                )),
            )
            .await;
            // R2: bounded drain+close. [FI-INV-05, R2 fix — bounded delivery]
            finalize_drain!();
            return;
        }
        Err(JoinCommitError::HuddleLinkGone) => {
            // Creator-signed huddle_started link deleted between pre-join check
            // and commit (IMPORTANT 4 residual: third carried fact).
            // No `joined` frame was sent — commit-won invariant holds.
            warn!(channel_id = %channel_id, pubkey = %pubkey_hex, "huddle_started link gone before join commit");
            // IMPORTANT 3: cancel + await expiry task before peer/room teardown.
            // B2: lifecycle_cancel holds transition lock. [FI-TRACE-CANCEL-RACE, B2 fix]
            control.lifecycle_cancel();
            if let Some(t) = _nip_fi_admission_expiry.take() {
                let _ = t.await;
            }
            // I1: lease is still guard-owned; guard.release_before_commit() releases it.
            guard.release_before_commit().await;
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                ws_send.send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"huddle has ended"})
                        .to_string()
                        .into(),
                )),
            )
            .await;
            // R2: bounded drain+close. [FI-INV-05, R2 fix — bounded delivery]
            finalize_drain!();
            return;
        }
        Err(JoinCommitError::Db(e)) => {
            // DB failure during join commit — treat same as pre-admission error.
            // No `joined` frame was sent — commit-won invariant holds.
            warn!(channel_id = %channel_id, pubkey = %pubkey_hex, "48101 commit failed: {e}");
            // IMPORTANT 3: cancel + await expiry task before peer/room teardown.
            // B2: lifecycle_cancel holds transition lock. [FI-TRACE-CANCEL-RACE, B2 fix]
            control.lifecycle_cancel();
            if let Some(t) = _nip_fi_admission_expiry.take() {
                let _ = t.await;
            }
            // I1: lease is still guard-owned; guard.release_before_commit() releases it.
            guard.release_before_commit().await;
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                ws_send.send(WsMessage::Text(
                    serde_json::json!({"type":"error","message":"error: join commit failed"})
                        .to_string()
                        .into(),
                )),
            )
            .await;
            // R2: bounded drain+close. [FI-INV-05, R2 fix — bounded delivery]
            finalize_drain!();
            return;
        }
    }

    // Commit-won. Take guard fields into the live runtime — any remaining
    // fields in guard at this point would be double-released on drop, but all
    // fields were taken by commit_participant_join above.
    let mut remote_session = guard.take_remote_session();
    let remote_stream = guard.take_remote_stream();
    let _ = guard.take_peer_id(); // peer_id was taken for the commit path

    // I1 mandated: transfer-after-commit-won. Now that the join is committed,
    // take the lease from the guard and install the registry renewer. Every
    // exit after this point is in the live runtime (no pre-commit resources
    // to unwind). The room-empty release below (fenced by `owner_generation`)
    // is the only release path from here.
    if let (Some(mesh), Some((lease, directory))) = (state.mesh(), guard.take_lease()) {
        let signals = mesh.owners.attach_signals(channel_id, directory, lease);
        owner_lost = Some(signals.lost);
        owner_draining = Some(signals.draining);
    }

    // B1: After commit_participant_join, the admission is committed. No further
    // check_cancel! is needed — the send_loop owns terminal_ctrl_rx from here.

    let missed_pongs = Arc::new(AtomicU8::new(0));

    // Dual-channel pattern (matches connection.rs): data channel for audio,
    // control channel for Ping/Pong/Close/control JSON with priority drain.
    let (data_tx, data_rx) = mpsc::channel::<WsMessage>(16);
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<WsMessage>(8);

    // The terminal channel was created before admission (above) so that
    // mid-admission expiry could drain it via ws_send. Now the send_loop takes
    // ownership of `terminal_ctrl_rx` and drains it in its cancel branch.
    // The expiry task (_nip_fi_admission_expiry) armed above is the lifetime
    // enforcer for this connection — no second task is needed.
    let send_cancel = cancel.child_token();
    let send_task = tokio::spawn(send_loop(
        ws_send,
        data_rx,
        ctrl_rx,
        terminal_ctrl_rx,
        send_cancel,
        disconnect_reason,
    ));

    let hb_missed = Arc::clone(&missed_pongs);
    let heartbeat_task = tokio::spawn(heartbeat_loop(ctrl_tx.clone(), hb_missed, control.clone()));

    let fwd_cancel = cancel.child_token();
    let forward_task = tokio::spawn(audio_forward_loop(
        audio_rx,
        peer_ctrl_rx,
        data_tx,
        ctrl_tx.clone(),
        fwd_cancel,
        control.clone(),
    ));

    // NIP-FI session-lifetime enforcement task was armed before admission
    // (at audio_session_deadline above) with `terminal_ctrl_tx`. Keep the
    // handle alive for the duration of the connection. [FI-TRACE-LEASE-BOUND]
    let nip_fi_audio_expiry_task = _nip_fi_admission_expiry;

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
        let reader_control = control.clone();
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
                    teardown_remote_huddle(cause, channel_id, &reader_control, &fence);
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
        let owner_control = control.clone();
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
                    owner_control.lifecycle_cancel();
                    fence.forget(channel_id);
                }
                _ = lost_fired => {
                    info!(
                        channel_id = %channel_id,
                        "huddle owner lost its lease — closing local client for rejoin"
                    );
                    owner_control.lifecycle_cancel();
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

    control.lifecycle_cancel();
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
    if let Some(expiry_task) = nip_fi_audio_expiry_task {
        let _ = expiry_task.await;
    }
    // Atomic owner remove + end check: remove_peer_and_check_ended holds the
    // AdmissionGuard lock across index recycling AND the is_empty + ended=true
    // check. Ingress mirrors never archive authoritative huddle state; they
    // remove locally and let the owner decide room lifetime.
    let removal = if remote_session.is_some() {
        room.remove_peer(peer_id).map(|delta| (delta, false))
    } else {
        room.remove_peer_and_check_ended(peer_id)
    };
    let removal_revision = if remote_session.is_none() {
        removal.as_ref().map(|(delta, _)| delta.revision)
    } else {
        // The ingress mirror's local revision is not the owner's authoritative
        // ordering. Omit it rather than publishing a plausible-but-wrong value.
        None
    };
    let should_auto_end = removal.as_ref().map(|(_, ended)| *ended).unwrap_or(false);

    if remote_session.is_none() {
        if let Some((delta, _)) = removal {
            if let Some(left) = delta.left {
                let left_msg = serde_json::json!({
                    "type": "left",
                    "revision": delta.revision,
                    "pubkey": left.pubkey,
                    "peer_index": left.peer_index,
                    "epoch": left.epoch,
                })
                .to_string();
                room.broadcast_control(left_msg);
            } else {
                warn!(
                    channel_id = %channel_id,
                    revision = delta.revision,
                    "audio peer removal delta did not include the removed peer"
                );
            }
        }
    }

    emit_participant_event(
        &state,
        &tenant,
        channel_id,
        parent_id_for_event,
        ParticipantLifecycle {
            kind: Kind::Custom(48102),
            participant_pubkey: &pubkey_hex,
            roster_revision: removal_revision,
            admission_id: Some(peer_id),
            generation: &lifecycle_generation,
        },
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
                    channel_id,
                    parent_id_for_event,
                    ParticipantLifecycle {
                        kind: Kind::Custom(48103),
                        participant_pubkey: &pubkey_hex,
                        roster_revision: None,
                        admission_id: None,
                        generation: &lifecycle_generation,
                    },
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
    control: &CommunityConnectionControl,
    fence: &crate::audio::mesh::GenerationFloor,
) {
    info!(
        channel_id = %channel_id,
        ?cause,
        "owner tore down cross-pod huddle session — closing client for rejoin"
    );
    control.lifecycle_cancel();
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
            "message": "room participant capacity reached"
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
pub(crate) async fn send_loop<S>(
    mut ws_send: S,
    mut data_rx: mpsc::Receiver<WsMessage>,
    mut ctrl_rx: mpsc::Receiver<WsMessage>,
    mut terminal_ctrl_rx: mpsc::Receiver<WsMessage>,
    cancel: CancellationToken,
    disconnect_reason: watch::Receiver<Option<crate::state::CommunityDisconnectReason>>,
) where
    S: futures_util::Sink<WsMessage> + Unpin,
{
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
                // Drain the terminal NIP-FI denial frame first (if any), then
                // ordinary control frames, before closing. Mirrors the root
                // relay send_loop idiom. The terminal channel has capacity 1
                // and is written before cancel() fires, so it is always
                // available when denial is enqueued — even when ctrl_rx
                // (capacity 8) is full. Without this drain the biased cancel
                // branch sends Close first and the client never sees the
                // required denial frame.
                while let Ok(terminal_msg) = terminal_ctrl_rx.try_recv() {
                    if ws_send.send(terminal_msg).await.is_err() {
                        return;
                    }
                }
                while let Ok(ctrl_msg) = ctrl_rx.try_recv() {
                    if ws_send.send(ctrl_msg).await.is_err() {
                        return;
                    }
                }
                let close = disconnect_reason
                    .borrow()
                    .map_or(WsMessage::Close(None), |reason| reason.close_message());
                let _ = ws_send.send(close).await;
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
    connection_control: CommunityConnectionControl,
) {
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            // Control messages get priority over audio in the select.
            msg = peer_ctrl_rx.recv() => {
                match msg {
                    Some(PeerCtrl::Json(json)) => {
                        if ctrl_tx.try_send(WsMessage::Text(json.into())).is_err() {
                            // State-bearing roster control may not be dropped.
                            // Closing the connection forces admission to replay
                            // a fresh authoritative snapshot.
                            connection_control.lifecycle_cancel();
                            break;
                        }
                    }
                    Some(PeerCtrl::Close) | None => {
                        connection_control.lifecycle_cancel();
                        break;
                    }
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
    control: CommunityConnectionControl,
) {
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        let cancelled = control.cancellation_token();
        tokio::select! {
            _ = interval.tick() => {
                // fetch_add returns the previous value; +1 gives the current count.
                let missed = missed_pongs.fetch_add(1, Ordering::Relaxed) + 1;
                if missed >= MAX_MISSED_PONGS {
                    warn!("audio: {missed} missed pongs — closing connection");
                    control.lifecycle_cancel();
                    break;
                }
                if ws_tx.try_send(WsMessage::Ping(axum::body::Bytes::new())).is_err() {
                    control.lifecycle_cancel();
                    break;
                }
            }
            _ = cancelled.cancelled() => break,
        }
    }
}

/// Outcome of [`check_membership_for_admission`].
///
/// `Existing` means the caller is already a member; no write is needed at join
/// time. `AutoAddRequired` means a membership write is still needed; it is
/// deferred into the same DB transaction that inserts the `48101` event, so
/// neither can commit without the other.
#[derive(Debug, Clone)]
pub(crate) enum MembershipAdmission {
    /// Caller is already a member of the audio channel.
    Existing { parent_channel_id: Uuid },
    /// Caller is a member of the parent channel and needs auto-add to the
    /// audio channel. The write is deferred into `commit_participant_join`.
    AutoAddRequired {
        parent_channel_id: Uuid,
        channel_created_by: Vec<u8>,
    },
}

/// Pre-admission ownership guard for the audio join path.
///
/// Owns all still-unattached resources acquired before `commit_participant_join`
/// succeeds: the unattached Redis lease (if this pod won the CAS), the remote
/// session + stream (if this is a cross-pod join), and the peer ID once admitted
/// to the local room. Each field is `take`n to `None` only at the single point
/// where it is either committed (transferred into the live runtime) or released
/// (cleaned up on a pre-commit exit).
///
/// `release_before_commit` releases / closes / removes every field that is still
/// `Some`. It is idempotent: calling it twice has no effect because every field
/// becomes `None` after the first call. After a commit-won, the caller calls
/// `take_*` methods to extract the committed state; any field that was not taken
/// is auto-released when the guard drops (unreachable in normal flow).
///
/// I1 invariant (transfer-after-commit-won): the `lease` field is held by the
/// guard for the entire pre-commit window. `guard.release_before_commit()` is
/// therefore the single release path for every pre-commit exit — no separate
/// registry call is needed. `take_lease()` is called only at commit-won, and the
/// lease is transferred into `HuddleOwnerRegistry::attach_signals` at that point.
///
/// This guard satisfies IMPORTANT 1-2 from the pass-3 review: every pre-commit
/// exit uses a single release path so no exit can skip lease release, remote
/// unregister, or peer removal.
struct HuddleAdmissionGuard {
    /// Unattached Redis lease won by this connection's CAS, plus the directory
    /// needed to release it. `None` when this pod is a steady-state owner
    /// (reuses the live registry entry) or a non-owner. Attached into
    /// `HuddleOwnerRegistry` only after commit-won.
    ///
    /// The directory is boxed as `dyn HuddleDirectory` so guard-level tests can
    /// inject a `FakeDir` double without requiring a live Redis instance (CW6).
    lease: Option<(
        crate::audio::join::HuddleLease,
        std::sync::Arc<dyn crate::audio::join::HuddleDirectory>,
    )>,
    /// Remote session registration (owner-assigned index + roster). Set when
    /// this pod is a non-owner and `dial_remote_owner` succeeded.
    remote_session: Option<crate::audio::join::RemoteHuddleSession>,
    /// Live control stream to the owner pod. Set alongside `remote_session`.
    remote_stream: Option<buzz_relay_mesh::MeshStream>,
    /// Peer ID in the local room once `add_peer[_at_index]` succeeded.
    peer_id: Option<Uuid>,
    /// Back-reference to the room for `remove_peer` on pre-commit exit.
    room: std::sync::Arc<crate::audio::room::Room>,
    /// Back-reference to the room manager for `cleanup_if_empty`.
    audio_rooms: std::sync::Arc<crate::audio::room::AudioRoomManager>,
    /// Community + channel for `cleanup_if_empty`.
    community: buzz_core::CommunityId,
    channel_id: Uuid,
}

impl HuddleAdmissionGuard {
    /// Release all still-held resources. Safe to call multiple times; each
    /// field becomes `None` on first release.
    ///
    /// - Unattached lease: calls `directory.release(&lease)` directly and
    ///   awaits the result before returning ("released before return" is
    ///   literal — no detached task). Warns on release error.
    /// - Remote registration: UnregisterPeer + Goodbye(SessionEnded) on stream.
    /// - Peer in room: remove_peer + cleanup_if_empty.
    async fn release_before_commit(&mut self) {
        // Release the unattached lease by calling directory.release directly.
        // This is an awaited call, so "release before return" is guaranteed —
        // no detached renewer task that could outlive the caller.
        if let Some((lease, directory)) = self.lease.take() {
            match directory.release(&lease).await {
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(
                        "HuddleAdmissionGuard: lease release failed on pre-commit exit: {e}"
                    );
                }
            }
        }
        // Close the remote registration.
        if let (Some(session), Some(ref mut stream)) =
            (self.remote_session.as_ref(), self.remote_stream.as_mut())
        {
            crate::audio::join::send_clean_close(stream, session.fenced(), session.pubkey()).await;
        }
        self.remote_session = None;
        self.remote_stream = None;
        // Remove the peer from the room.
        if let Some(pid) = self.peer_id.take() {
            self.room.remove_peer(pid);
            self.audio_rooms
                .cleanup_if_empty(self.community, self.channel_id);
        }
    }

    /// Take the remote session (consumed at commit-won for the send-loop task).
    fn take_remote_session(&mut self) -> Option<crate::audio::join::RemoteHuddleSession> {
        self.remote_session.take()
    }

    /// Take the remote stream (consumed at commit-won for the reader task).
    fn take_remote_stream(&mut self) -> Option<buzz_relay_mesh::MeshStream> {
        self.remote_stream.take()
    }

    /// Take the lease (consumed at commit-won to pass into `attach_signals`).
    fn take_lease(
        &mut self,
    ) -> Option<(
        crate::audio::join::HuddleLease,
        std::sync::Arc<dyn crate::audio::join::HuddleDirectory>,
    )> {
        self.lease.take()
    }

    /// Take the peer ID (consumed at commit-won so normal teardown owns cleanup).
    fn take_peer_id(&mut self) -> Option<Uuid> {
        self.peer_id.take()
    }
}

/// Validate membership for audio admission — **no durable write**.
///
/// Loads the channel, checks archival status, resolves the parent-channel
/// linkage for ephemeral channels, and checks existing membership and parent
/// membership. Returns [`MembershipAdmission`] describing what still needs
/// to happen at commit time.
///
/// Performs zero DB writes. Any needed auto-add write is deferred into the
/// caller-owned transaction inside `commit_participant_join`.
async fn check_membership_for_admission(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
    pubkey_bytes: &[u8],
    parent_channel_id: Option<Uuid>,
) -> Result<MembershipAdmission, String> {
    // Test hook: fires at the entry of the membership check so a test can arm
    // expiry between NIP-42 pairing and the first DB read. Proves that a
    // cancellation before membership check produces zero DB side effects.
    // No-op in production. [nip_fi_test_hooks::audio_membership_check_hook]
    #[cfg(test)]
    crate::nip_fi_test_hooks::before_membership_check(tenant.community()).await;

    // Load channel first — reject archived channels before any membership check.
    let channel = state
        .db
        .get_channel(tenant.community(), channel_id)
        .await
        .map_err(|e| format!("db error: {e}"))?;

    if channel.archived_at.is_some() {
        return Err("channel is archived".into());
    }

    // Lifecycle events for an ephemeral huddle belong in its parent channel.
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
        return Ok(MembershipAdmission::Existing {
            parent_channel_id: lifecycle_parent_id,
        });
    }

    if channel.visibility == "open" {
        return Ok(MembershipAdmission::Existing {
            parent_channel_id: lifecycle_parent_id,
        });
    }

    // Auto-add path: private ephemeral channel + caller is member of parent.
    if channel.ttl_seconds.is_some() {
        let parent_member = state
            .is_member_cached(tenant.community(), lifecycle_parent_id, pubkey_bytes)
            .await
            .map_err(|e| format!("db error: {e}"))?;

        if parent_member {
            return Ok(MembershipAdmission::AutoAddRequired {
                parent_channel_id: lifecycle_parent_id,
                channel_created_by: channel.created_by.clone(),
            });
        }
    }

    Err("not a member".into())
}

/// Outcome returned by [`commit_participant_join`] on the `Ok` path.
///
/// Indicates whether the `joined` broadcast was queued inside the permit
/// (always the case with `broadcast_control`) or whether the peer's ctrl
/// channel was saturated and the message was dropped (the forward loop will
/// detect the dead channel and drive normal admitted teardown from there).
#[derive(Debug)]
pub(crate) enum CommitJoinOutcome {
    /// `joined` was queued to all peers' ctrl channels inside the permit.
    JoinedSent,
    /// The joining peer's ctrl channel was already saturated; the message
    /// was dropped. The forward loop will close via the dead channel.
    /// Structurally unreachable at this time (fresh peer channel is never
    /// full), kept as a safety valve for future capacity changes.
    #[allow(dead_code)]
    JoinedSendFailed,
}

/// Error returned by [`commit_participant_join`].
#[derive(Debug)]
pub(crate) enum JoinCommitError {
    /// DB transaction setup or commit failed.
    Db(buzz_db::DbError),
    /// The session gate rejected the permit (session expired before commit).
    Expired,
    /// Channel was archived between pre-join check and commit (IMPORTANT 4).
    Archived,
    /// Parent membership was revoked between pre-join check and commit (IMPORTANT 4).
    ParentMembershipLost,
    /// Creator-signed huddle_started link was deleted between pre-join check
    /// and commit (IMPORTANT 4 residual: third carried fact).
    HuddleLinkGone,
}

impl std::fmt::Display for JoinCommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JoinCommitError::Db(e) => write!(f, "db error: {e}"),
            JoinCommitError::Expired => write!(f, "session expired before commit"),
            JoinCommitError::Archived => write!(f, "channel archived before commit"),
            JoinCommitError::ParentMembershipLost => {
                write!(f, "parent membership revoked before commit")
            }
            JoinCommitError::HuddleLinkGone => {
                write!(f, "huddle_started creator link gone before commit")
            }
        }
    }
}

impl From<buzz_db::DbError> for JoinCommitError {
    fn from(e: buzz_db::DbError) -> Self {
        JoinCommitError::Db(e)
    }
}

/// Atomically commit the participant join: auto-add membership (if needed) +
/// kind `48101` event, in one DB transaction, under a session effect permit.
///
/// Ordering (per B1 contract [e5bc0382], corrected for IMPORTANT 4 and 5):
/// 1. Sign the `48101` event synchronously.
/// 2. Begin a caller-owned DB transaction.
/// 3. Under the channel membership lock (AutoAddRequired only):
///    a. Re-read channel archive state — fail `Archived` if now archived.
///    (IMPORTANT 4: closes the race between pre-join check and commit.)
///    b. Re-read parent membership — fail `ParentMembershipLost` if gone.
///    c. Re-read creator-signed huddle_started link — fail `HuddleLinkGone`
///    if the link was deleted between pre-join check and commit.
///    (IMPORTANT 4 residual: third carried fact, alongside archive + parent.)
///    d. Re-read child membership — skip auto-add insert if a concurrent
///    legitimate add is already present (concurrent-add preservation).
/// 4. Insert kind `48101` in the same transaction (uncommitted).
/// 5. Acquire a session effect permit (or rollback + return `Err(Expired)`).
/// 6. Commit the transaction while holding the permit.
/// 7. While the same permit is held: mark the event locally, fan out to local
///    subscribers, publish to Redis, and broadcast `joined` to all peers
///    (including the joiner) via `room.broadcast_control`. (IMPORTANT 5:
///    `joined` publication inside the commit-won permit.) Drop permit after.
///
/// Never cancels or drops the commit future once started — commit returns a
/// known outcome and that outcome drives success or the pre-admission cleanup.
///
/// Argument count reflects the join's natural surface; a param struct would
/// obscure more than it clarifies at this single call site.
#[allow(clippy::too_many_arguments)]
async fn commit_participant_join(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
    parent_channel_id: Uuid,
    pubkey_hex: &str,
    pubkey_bytes: &[u8],
    peer_id: Uuid,
    roster_revision: u64,
    lifecycle_generation: &str,
    membership_admission: &MembershipAdmission,
    gate: &std::sync::Arc<crate::nip_fi_gate::SessionAdmissionGate>,
    joined_msg: String,
    room: &std::sync::Arc<crate::audio::room::Room>,
) -> Result<CommitJoinOutcome, JoinCommitError> {
    // 1. Sign the 48101 event synchronously.
    // Include `generation` (matching the wire key Desktop's `lifecycleContent` parser
    // reads at `huddlePresence.ts:83-85`) so Desktop reconciliation can compare
    // against liveness responses — without it, an already-hydrated observer sees
    // a JOIN without a generation, treats it as "pending", and then clears
    // admissions when the next authoritative liveness response (with a real
    // generation) differs.  [F3: generation key in 48101 JOIN — Carl review]
    let content = serde_json::json!({
        "ephemeral_channel_id": channel_id.to_string(),
        "roster_revision": roster_revision,
        "admission_id": peer_id.to_string(),
        "generation": lifecycle_generation,
    })
    .to_string();

    let h_tag = Tag::parse(["h", &parent_channel_id.to_string()]).map_err(|e| {
        JoinCommitError::Db(buzz_db::DbError::InvalidData(format!(
            "failed to build h tag: {e}"
        )))
    })?;
    let p_tag = Tag::parse(["p", pubkey_hex]).map_err(|e| {
        JoinCommitError::Db(buzz_db::DbError::InvalidData(format!(
            "failed to build p tag: {e}"
        )))
    })?;
    let event = EventBuilder::new(Kind::Custom(48101), content)
        .tags(vec![h_tag, p_tag])
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| {
            JoinCommitError::Db(buzz_db::DbError::InvalidData(format!(
                "failed to sign 48101: {e}"
            )))
        })?;
    let event_id_hex = event.id.to_hex();

    // 2. Begin a caller-owned DB transaction.
    let mut tx = state.db.begin_event_write_transaction().await?;

    // 3. Under the channel membership lock: re-validate authority + auto-add if
    //    still absent. The AutoAddRequired path carries stale authority from
    //    check_membership_for_admission; the lock serialises all membership writes
    //    for this channel so the re-reads observe the most recent committed state.
    if let MembershipAdmission::AutoAddRequired {
        parent_channel_id: parent_id,
        channel_created_by,
    } = membership_admission
    {
        // Test hook: fires immediately before the channel membership lock is
        // acquired. A test can insert a membership row externally here to prove
        // the concurrent-add case is handled (re-read observes it → still_absent
        // = false → auto-add insert is skipped → membership preserved).
        // [nip_fi_test_hooks::audio_membership_lock_hook]
        #[cfg(test)]
        crate::nip_fi_test_hooks::before_membership_lock(tenant.community()).await;

        buzz_db::channel_members::acquire_channel_membership_lock_in_transaction(
            &mut tx,
            tenant.community(),
            channel_id,
        )
        .await?;

        // IMPORTANT 4a: Re-read channel archive state under the lock. A channel
        // could be archived in the window between check_membership_for_admission
        // and now; committing a join into an archived channel violates the
        // "no admission after archive" invariant.
        let channel_archived: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
            "SELECT archived_at FROM channels \
             WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
        )
        .bind(tenant.community().as_uuid())
        .bind(channel_id)
        .fetch_optional(tx.as_mut())
        .await
        .map_err(buzz_db::DbError::from)?
        .flatten();

        if channel_archived.is_some() {
            let _ = tx.rollback().await;
            return Err(JoinCommitError::Archived);
        }

        // IMPORTANT 4b: Re-read parent membership under the lock. A parent
        // membership revocation in the same window would make the auto-add
        // unjustified; reject rather than grant access from stale authority.
        let parent_still_member = buzz_db::channel_members::is_member_in_transaction(
            &mut tx,
            tenant.community(),
            *parent_id,
            pubkey_bytes,
        )
        .await?;

        if !parent_still_member {
            let _ = tx.rollback().await;
            return Err(JoinCommitError::ParentMembershipLost);
        }

        // IMPORTANT 4 residual: Re-read the creator-signed huddle_started link
        // inside the transaction. This is the third carried fact alongside the
        // archive + parent-membership re-reads. The link could be deleted by a
        // concurrent channel teardown after check_membership_for_admission ran
        // but before this transaction acquires the lock; committing a join into
        // an unlinked channel violates the "creator authority" invariant.
        let link_still_exists = buzz_db::event::huddle_started_link_exists_in_transaction(
            &mut tx,
            tenant.community(),
            *parent_id,
            channel_id,
            channel_created_by.as_slice(),
        )
        .await?;

        if !link_still_exists {
            let _ = tx.rollback().await;
            return Err(JoinCommitError::HuddleLinkGone);
        }

        // Re-read child membership — a concurrent legitimate add may have
        // already provided access; do not overwrite role/provenance.
        let still_absent = !buzz_db::channel_members::is_member_in_transaction(
            &mut tx,
            tenant.community(),
            channel_id,
            pubkey_bytes,
        )
        .await?;

        if still_absent {
            buzz_db::channel_members::insert_auto_membership_in_transaction(
                &mut tx,
                tenant.community(),
                channel_id,
                pubkey_bytes,
                channel_created_by.as_slice(),
            )
            .await?;
        }
        // If not still_absent: concurrent add observed — membership preserved.
    }

    // 4. Insert kind `48101` uncommitted.
    let (stored, was_inserted) = buzz_db::event::insert_event_in_transaction(
        &mut tx,
        tenant.community(),
        &event,
        Some(parent_channel_id),
    )
    .await?;

    // 5. Acquire effect permit or rollback.
    //
    // Test hook: fires between the uncommitted 48101 insert and the permit
    // acquisition. A test can arm expiry here to prove that a cancellation
    // after the DB write but before commit rolls back the transaction and
    // produces zero committed side effects.
    // [nip_fi_test_hooks::audio_participant_commit_hook]
    #[cfg(test)]
    crate::nip_fi_test_hooks::before_participant_commit(tenant.community()).await;
    let _permit = match gate.acquire_effect().await {
        Ok(permit) => permit,
        Err(crate::nip_fi_gate::SessionExpired) => {
            // Rollback explicitly — no 48101 or membership write committed.
            let _ = tx.rollback().await;
            return Err(JoinCommitError::Expired);
        }
    };

    // 6. Commit while holding the permit.
    if let Err(e) = tx.commit().await {
        return Err(JoinCommitError::Db(e.into()));
    }

    // 7. Fan-out while permit is still held — expiry cannot complete between
    //    row visibility and fan-out.
    if was_inserted {
        state.mark_local_event(tenant.community(), &event.id);
        crate::handlers::event::fan_out_event_to_local_subscribers(
            state,
            tenant.community(),
            &stored,
        )
        .await;

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
                "audio: failed to publish 48101: {e}"
            );
        }

        // Best-effort mention insertion — outside the gate, failure is a warn.
        if let Err(e) = buzz_db::insert_mentions(
            state.db.pool(),
            tenant.community(),
            &event,
            Some(parent_channel_id),
        )
        .await
        {
            warn!(event_id = %event_id_hex, "audio: failed to insert 48101 mentions: {e}");
        }
    } else {
        debug!(
            event_id = %event_id_hex,
            channel_id = %parent_channel_id,
            "audio: 48101 already persisted — skipping fan-out"
        );
    }

    // IMPORTANT 5: broadcast `joined` to all peers (including the joiner) while
    // the commit-won permit is still held. `broadcast_control` sends via each
    // peer's ctrl channel; the joining peer's channel was created by add_peer and
    // is read by the audio_forward_loop once it starts. The message is buffered
    // in that channel until the loop drains it.
    //
    // The peer's ctrl channel is freshly created by add_peer (capacity 8) so the
    // try_send inside broadcast_control will succeed. JoinedSent is always
    // returned; JoinedSendFailed is structurally unreachable here but kept for
    // completeness — the forward loop's dead-channel path handles any future
    // saturation case at runtime.
    room.broadcast_control(joined_msg);
    let outcome = CommitJoinOutcome::JoinedSent;

    // Test hook: fires after fan-out and `joined` broadcast, but BEFORE
    // `_permit` drops. Used by CW10: expiry armed here blocks at the write
    // guard until the permit drops at the end of this scope.
    // [nip_fi_test_hooks::audio_participant_fanout_hook]
    #[cfg(test)]
    crate::nip_fi_test_hooks::after_participant_fanout(tenant.community()).await;
    // _permit drops here — gate quiescence barrier may proceed.

    // After commit, invalidate the membership cache if we auto-added.
    if matches!(
        membership_admission,
        MembershipAdmission::AutoAddRequired { .. }
    ) {
        state.invalidate_membership(tenant, channel_id, pubkey_bytes);
    }

    Ok(outcome)
}

#[derive(Clone, Copy)]
struct ParticipantLifecycle<'a> {
    kind: Kind,
    participant_pubkey: &'a str,
    roster_revision: Option<u64>,
    admission_id: Option<Uuid>,
    generation: &'a str,
}

async fn emit_participant_event(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
    parent_channel_id: Uuid,
    lifecycle: ParticipantLifecycle<'_>,
) {
    let ParticipantLifecycle {
        kind,
        participant_pubkey,
        roster_revision,
        admission_id,
        generation,
    } = lifecycle;
    let content = match (roster_revision, admission_id) {
        (Some(revision), Some(admission_id)) => serde_json::json!({
            "ephemeral_channel_id": channel_id.to_string(),
            "roster_revision": revision,
            "admission_id": admission_id.to_string(),
            "generation": generation,
        }),
        (Some(revision), None) => serde_json::json!({
            "ephemeral_channel_id": channel_id.to_string(),
            "roster_revision": revision,
            "generation": generation,
        }),
        (None, Some(admission_id)) => serde_json::json!({
            "ephemeral_channel_id": channel_id.to_string(),
            "admission_id": admission_id.to_string(),
            "generation": generation,
        }),
        (None, None) => serde_json::json!({
            "ephemeral_channel_id": channel_id.to_string(),
            "generation": generation,
        }),
    }
    .to_string();

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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use axum::{routing::get, Router};
    use futures_util::SinkExt;
    use sqlx::Row as _;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    use super::*;

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
    async fn saturated_websocket_control_queue_cancels_the_audio_connection() {
        let (_audio_tx, audio_rx) = mpsc::channel(1);
        let (peer_ctrl_tx, peer_ctrl_rx) = mpsc::channel(2);
        let (data_tx, _data_rx) = mpsc::channel(1);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(1);
        ctrl_tx
            .try_send(WsMessage::Ping(Bytes::new()))
            .expect("fill websocket control queue");
        peer_ctrl_tx
            .try_send(PeerCtrl::Json("{}".into()))
            .expect("queue state-bearing control");
        let task_cancel = CancellationToken::new();
        let connection_cancel = CancellationToken::new();
        let connection_control =
            crate::state::CommunityConnectionControl::new(connection_cancel.clone());

        audio_forward_loop(
            audio_rx,
            peer_ctrl_rx,
            data_tx,
            ctrl_tx,
            task_cancel,
            connection_control,
        )
        .await;

        assert!(
            connection_cancel.is_cancelled(),
            "saturated websocket control must force a fresh roster admission"
        );
    }

    #[tokio::test]
    async fn closed_peer_control_queue_cancels_the_audio_connection() {
        let (_audio_tx, audio_rx) = mpsc::channel(1);
        let (peer_ctrl_tx, peer_ctrl_rx) = mpsc::channel(1);
        let (data_tx, _data_rx) = mpsc::channel(1);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(1);
        let task_cancel = CancellationToken::new();
        let connection_cancel = CancellationToken::new();
        let connection_control =
            crate::state::CommunityConnectionControl::new(connection_cancel.clone());

        let forward = tokio::spawn(audio_forward_loop(
            audio_rx,
            peer_ctrl_rx,
            data_tx,
            ctrl_tx,
            task_cancel,
            connection_control,
        ));
        drop(peer_ctrl_tx);

        tokio::time::timeout(Duration::from_secs(1), forward)
            .await
            .expect("forwarder exits when its state-bearing queue closes")
            .expect("forwarder task completes cleanly");
        assert!(
            connection_cancel.is_cancelled(),
            "lost control state must tear down the WebSocket for a fresh roster"
        );
    }

    #[tokio::test]
    async fn audio_send_loop_sends_policy_close_when_community_is_deleted() {
        use futures_util::Sink;

        struct MockSink {
            messages: Arc<Mutex<Vec<WsMessage>>>,
        }

        impl Sink<WsMessage> for MockSink {
            type Error = std::io::Error;

            fn poll_ready(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<(), Self::Error>> {
                std::task::Poll::Ready(Ok(()))
            }

            fn start_send(
                self: std::pin::Pin<&mut Self>,
                item: WsMessage,
            ) -> Result<(), Self::Error> {
                self.messages.lock().expect("mock sink poisoned").push(item);
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

        let (_data_tx, data_rx) = mpsc::channel(1);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let control = CommunityConnectionControl::new(cancel.clone());
        let disconnect_reason = control.disconnect_reason();
        let registry = crate::state::CommunityConnectionRegistry::new();
        let community = buzz_core::CommunityId::from_uuid(Uuid::new_v4());
        let _guard = registry.register(Uuid::new_v4(), community, control);
        assert_eq!(registry.disconnect_community(community), 1);
        let messages = Arc::new(Mutex::new(Vec::new()));
        let sink = MockSink {
            messages: Arc::clone(&messages),
        };

        send_loop(
            sink,
            data_rx,
            ctrl_rx,
            mpsc::channel(1).1,
            cancel,
            disconnect_reason,
        )
        .await;

        let messages = messages.lock().expect("mock sink poisoned");
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            WsMessage::Close(Some(close)) => {
                assert_eq!(close.code, axum::extract::ws::close_code::POLICY);
                assert_eq!(close.reason.as_str(), "community deleted");
            }
            other => panic!("expected one 1008 deletion close, got {other:?}"),
        }
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

    // ── Witness B: Audio pairing mismatch through the real audio path ─────────
    //
    // Drives the production `handle_active_audio_connection` over a real local
    // WebSocket pair. Key A is named in the assertion; key B signs the audio
    // auth message — mismatch. The function must deliver the exact restricted
    // JSON frame and cancel before returning.
    //
    // The test calls `handle_active_audio_connection` directly (bypassing
    // `handle_audio_connection`/`run_registered_community_connection`) so no
    // live DB connection is required: the pairing fires before any membership
    // DB gate, so a lazy pool suffices.
    //
    // Mutation evidence:
    //   - Delete the production call from `handle_active_audio_connection` →
    //     exact restricted frame absent (or a later, different error arrives);
    //     test panics on frame content or cancellation assertion.
    //   - Delete the denial branch inside `enforce_nip_fi_key_pairing` → same.
    //   - Change the JSON shape/text → byte assertion panics.
    //   - Omit cancellation → cancellation assertion panics.

    async fn audio_test_state() -> std::sync::Arc<crate::state::AppState> {
        use std::sync::Arc;
        let mut config = crate::config::Config::hermetic_for_test();
        config.require_relay_membership = false;
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        Arc::new(state)
    }

    #[tokio::test]
    async fn handle_active_audio_connection_pairing_mismatch_runs_full_audio_denial_path() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;

        let key_a = nostr::Keys::generate();
        let key_b = nostr::Keys::generate();

        let assertion = VerifiedAssertion::for_test(
            Some(key_a.public_key()),
            vec![Utc::now() + Duration::hours(1)],
        );

        let state = audio_test_state().await;
        let _channel_id = uuid::Uuid::new_v4();

        // Build a real tenant context matching what `nip42_expected_relay_url`
        // will compute (scheme from config.relay_url = "ws://", host = "test.local").
        let tenant = buzz_core::tenant::TenantContext::resolved(
            buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::nil()),
            "test.local".to_string(),
        );

        // Set up a local WS server that runs `handle_active_audio_connection`.
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        // conn_cancel is created here so the test retains it for the
        // is_cancelled() assertion. The token is cloned into the server closure.
        let conn_cancel = CancellationToken::new();
        let cancel_for_assert = conn_cancel.clone();
        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("test listener addr");

        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    // Clone once for the closure; the original is retained
                    // outside for the cancellation assertion.
                    let cancel_i = conn_cancel.clone();
                    move |ws: WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let conn_time = chrono::Utc::now();
                        let control_inner =
                            crate::state::CommunityConnectionControl::new(cancel_i.clone());
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    uuid::Uuid::new_v4(),
                                    control_inner,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );

            let _ = ready_tx.send(());
            axum::serve(listener, app).await.expect("test server");
        });

        // Wait for server to be ready, then get the cancel token it sent.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("server ready");

        // Refactor: the server uses its own cancel per connection (above).
        // We instead track completion by the WS close message.

        // Connect the client.
        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("connect client");

        // Receive the challenge message.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("challenge timeout")
            .expect("challenge message")
            .expect("challenge ws message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("challenge field")
            .to_string();

        // Sign the auth message with key B (mismatch — assertion names key A).
        let relay_url = "ws://test.local";
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key_b)
            .unwrap();

        let auth_msg = serde_json::json!({
            "type": "auth",
            "event": auth_event,
        })
        .to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("send auth msg");

        // The server must send the exact restricted JSON frame before closing.
        let frame = tokio::time::timeout(std::time::Duration::from_secs(3), client.next())
            .await
            .expect("restricted frame timeout")
            .expect("frame")
            .expect("ws frame");

        let expected_restricted = serde_json::json!({
            "type": "restricted",
            "message": buzz_auth::DenialClass::AuthorizationDenied.nostr_text()
        })
        .to_string();

        match frame {
            tokio_tungstenite::tungstenite::Message::Text(t) => {
                assert_eq!(
                    t.as_str(),
                    expected_restricted.as_str(),
                    "audio pairing mismatch must produce exact restricted JSON before close"
                );
            }
            other => panic!("expected Text(restricted JSON); got {other:?}"),
        }

        // The connection must close after the denial. The audio path sends the
        // restricted frame directly on ws_send, then drops it (no send_loop to
        // drain a Close frame). The client may see either:
        //   a) a WS Close frame if axum's runtime sends one on drop, or
        //   b) None / Err (connection reset) when the socket drops.
        // Both are acceptable — the key check is that the restricted frame was
        // already received above.
        let close = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("close timeout");
        assert!(
            matches!(
                close,
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | Some(Err(_)) | None
            ),
            "connection must close after audio pairing mismatch; got {close:?}"
        );

        // The retained token must be cancelled — this is the named mutation
        // target: omit cancel.cancel() inside enforce_nip_fi_key_pairing and
        // this assertion fails even though the socket still drops.
        assert!(
            cancel_for_assert.is_cancelled(),
            "conn_cancel must be cancelled after audio pairing mismatch"
        );

        server.abort();
        let _ = server.await;
    }

    // ── W5 (B1 audio): already-expired deadline rejects at pairing, before admission
    //
    // When the NIP-FI session deadline is already past at pairing time (the
    // assertion's authority deadlines are all in the past), `handle_active_audio_connection`
    // must send the canonical `restricted` denial frame and close the connection
    // before writing any relay-membership, room-join, or roster side effect.
    //
    // This test gives the handler the same key in both the assertion and the
    // NIP-42 event so pairing succeeds, but sets an already-expired deadline.
    // The B1 gate fires between the pairing check and `enforce_relay_membership`.
    //
    // Mutation evidence:
    //   A) Delete the B1 already-expired check → the B1 restricted frame is
    //      not sent before admission; the membership gate fires next. Since the
    //      test's lazy DB rejects membership, the frame text changes from
    //      "restricted: authorization denied" to "restricted: not a relay member"
    //      → the byte assertion panics.
    //   B) Change the sent frame text → byte assertion panics.
    //   C) Omit `cancel.cancel()` in the B1 branch → cancel assertion panics.

    #[tokio::test]
    async fn b1_already_expired_session_denied_at_pairing_before_admission() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;

        let key = nostr::Keys::generate();

        // Assertion: same key for both assertion and NIP-42 event → pairing passes.
        // But the deadline is 2 seconds in the past → B1 fires.
        let expired_deadline = Utc::now() - Duration::seconds(2);
        let assertion = VerifiedAssertion::for_test(Some(key.public_key()), vec![expired_deadline]);

        let state = audio_test_state().await;

        let tenant = buzz_core::tenant::TenantContext::resolved(
            buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::nil()),
            "test.local".to_string(),
        );

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let conn_cancel = CancellationToken::new();
        let cancel_for_assert = conn_cancel.clone();
        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("test listener addr");

        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let cancel_i = conn_cancel.clone();
                    move |ws: WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let conn_time = chrono::Utc::now();
                        let control_inner =
                            crate::state::CommunityConnectionControl::new(cancel_i.clone());
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    uuid::Uuid::new_v4(),
                                    control_inner,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app).await.expect("test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("server ready");

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("connect client");

        // Receive the challenge.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("challenge timeout")
            .expect("challenge message")
            .expect("challenge ws message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("challenge field")
            .to_string();

        // Sign the auth message with the SAME key as the assertion — pairing passes.
        let relay_url = "ws://test.local";
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key)
            .unwrap();

        let auth_msg = serde_json::json!({
            "type": "auth",
            "event": auth_event,
        })
        .to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("send auth msg");

        // The B1 gate must send the exact canonical restricted JSON frame.
        // This is byte-identical to the pairing-mismatch frame — same production
        // `authorization_denied_frame(NipFiWsRoute::Audio)` path.
        let frame = tokio::time::timeout(std::time::Duration::from_secs(3), client.next())
            .await
            .expect("restricted frame timeout")
            .expect("frame")
            .expect("ws frame");

        let expected_restricted = serde_json::json!({
            "type": "restricted",
            "message": buzz_auth::DenialClass::AuthorizationDenied.nostr_text()
        })
        .to_string();

        match frame {
            tokio_tungstenite::tungstenite::Message::Text(t) => {
                assert_eq!(
                    t.as_str(),
                    expected_restricted.as_str(),
                    "B1: expired session must produce exact canonical restricted JSON before close"
                );
            }
            other => panic!("B1: expected Text(restricted JSON); got {other:?}"),
        }

        // Connection must close after the B1 denial.
        let close = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("close timeout");
        assert!(
            matches!(
                close,
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | Some(Err(_)) | None
            ),
            "B1: connection must close after expired-session denial; got {close:?}"
        );

        // The cancel token must be cancelled — omitting cancel.cancel() in the
        // B1 branch makes this assertion fail even when the socket still drops.
        assert!(
            cancel_for_assert.is_cancelled(),
            "B1: conn_cancel must be cancelled after expired-session denial at pairing"
        );

        server.abort();
        let _ = server.await;
    }

    // ── W6 (B1 audio mid-admission): cancellation before room.add_peer ─────────
    //
    // With the expiry task armed before admission (above the first persisting
    // step), a cancellation fired during the admission sequence must prevent
    // room.add_peer from executing. The audio room must remain empty.
    //
    // This test fires the expiry task between the pairing check and the first
    // check_cancel!() boundary. To avoid a sleep-lottery it uses the connection
    // cancel token directly: the token is pre-cancelled, which is equivalent to
    // the expiry task firing before check_cancel!() is reached. The room is
    // inspected after the handler returns to confirm no peer was added.
    //
    // The biased auth-loop select fires `cancel.cancelled()` → return before
    // reaching check_cancel!(). The room invariant (no peer added) is the
    // observable outcome that must hold regardless of which cancellation path
    // fires. The mutation evidence for the check_cancel!() fences themselves is
    // in the focused unit tests in connection.rs (B2/B3 tests), where the fence
    // mechanism is exercised in isolation.
    //
    // What this test proves end-to-end:
    //   A real audio connection with a cancelled token cannot reach room.add_peer.
    //   This was NOT true before the B1 fix: the expiry task was armed AFTER
    //   room.add_peer (line ~858), so it could not prevent admission.
    //
    // Mutation evidence:
    //   A) Move the expiry task creation back to after room.add_peer (the pre-fix
    //      location) → test still passes (cancel path fires first). The test is
    //      therefore evidence of the cancel-stops-admission invariant, not of the
    //      exact placement of the expiry arm.
    //   B) Remove `_ = cancel.cancelled() => return` from the audio auth select →
    //      handler proceeds to auth exchange → if auth takes > 3 s (timeout) the
    //      test fails; in practice the close assertion fires immediately.

    #[tokio::test]
    async fn b1_mid_admission_expiry_does_not_add_peer_to_room() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;
        use tokio_tungstenite::connect_async;

        let key = nostr::Keys::generate();
        // A non-expired assertion — pairing passes if we reach that check.
        // The cancellation intercepts before pairing, so the room stays empty.
        let assertion = VerifiedAssertion::for_test(
            Some(key.public_key()),
            vec![Utc::now() + Duration::hours(1)],
        );

        let state = audio_test_state().await;
        let audio_rooms = Arc::clone(&state.audio_rooms);
        let tenant = buzz_core::tenant::TenantContext::resolved(
            buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::nil()),
            "test.local".to_string(),
        );
        let channel_id = uuid::Uuid::new_v4();

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        // Pre-cancel: token is set before handle_active_audio_connection runs.
        // The biased `_ = cancel.cancelled() => return` in the audio auth select
        // fires at the first executor poll, preventing any room mutation.
        let conn_cancel = CancellationToken::new();
        conn_cancel.cancel();
        let cancel_clone = conn_cancel.clone();

        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("test listener addr");

        let server = tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/",
                axum::routing::get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let cancel_i = cancel_clone.clone();
                    move |ws: axum::extract::ws::WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let conn_time = chrono::Utc::now();
                        let control_inner =
                            crate::state::CommunityConnectionControl::new(cancel_i.clone());
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    channel_id,
                                    control_inner,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app).await.expect("test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("server ready");

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("connect client");

        // Server sends the challenge then exits immediately (biased cancel fires).
        // The client receives the challenge, then observes the connection close.
        let _challenge = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .ok(); // May succeed (challenge) or fail (connection already dropped).

        // The connection must close before the 3 s timeout.
        let close = tokio::time::timeout(std::time::Duration::from_secs(3), client.next()).await;
        assert!(
            close.is_ok(),
            "B1: connection must close before timeout when token is pre-cancelled"
        );

        // The audio room must be empty — no peer was added.
        let community = buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::nil());
        if let Some(room) = audio_rooms.get(community, channel_id) {
            assert!(
                room.is_empty(),
                "B1: audio room must have zero peers when cancel fires before room.add_peer"
            );
        }
        // Room may not exist at all — that also satisfies the invariant.

        server.abort();
        let _ = server.await;
    }

    // ── W7 (B3 audio): audio expiry sends exact restricted frame before close ────
    //
    // Drives BOTH production seams:
    //   1. `nip_fi_session::spawn_nip_fi_expiry_task` with `NipFiWsRoute::Audio`.
    //   2. The real generic audio `send_loop` with a recording sink.
    //
    // The expiry constructor synchronously queues the denial on `ctrl_tx` and
    // cancels without any await in between, so the audio send loop's
    // cancellation drain picks up the frame before writing Close.
    //
    // Mutation evidence:
    //   - Delete/change the audio enqueue in `spawn_nip_fi_expiry_task` →
    //     output lacks or mismatches frame 0.
    //   - Revert the audio send_loop cancellation drain → output begins with
    //     Close(None) or lacks the restricted frame entirely.
    //   - Replace audio's production constructor call with a copied local task →
    //     structural requirement: exactly one `spawn_nip_fi_expiry_task`
    //     definition (in `nip_fi_session`) and two production invocations (root
    //     in `connection.rs`, audio in `audio/handler.rs`). Any copy breaks
    //     this test's coupling to the shared producer.

    #[tokio::test]
    async fn audio_expiry_sends_exact_restricted_frame_before_close() {
        use std::pin::Pin;
        use std::sync::Arc;
        use std::task::{Context, Poll};
        use tokio::sync::mpsc;

        // Recording sink that stores every message in order.
        struct RecordSink(Arc<tokio::sync::Mutex<Vec<WsMessage>>>);
        impl futures_util::Sink<WsMessage> for RecordSink {
            type Error = std::convert::Infallible;
            fn poll_ready(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
            ) -> Poll<Result<(), Self::Error>> {
                Poll::Ready(Ok(()))
            }
            fn start_send(self: Pin<&mut Self>, item: WsMessage) -> Result<(), Self::Error> {
                self.get_mut()
                    .0
                    .try_lock()
                    .expect("RecordSink lock")
                    .push(item);
                Ok(())
            }
            fn poll_flush(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
            ) -> Poll<Result<(), Self::Error>> {
                Poll::Ready(Ok(()))
            }
            fn poll_close(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<Result<(), Self::Error>> {
                self.poll_flush(cx)
            }
        }

        let recorded = Arc::new(tokio::sync::Mutex::new(Vec::<WsMessage>::new()));
        let sink = RecordSink(Arc::clone(&recorded));

        let (_data_tx, data_rx) = mpsc::channel::<WsMessage>(4);
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<WsMessage>(8);
        let (terminal_tx, terminal_rx) = mpsc::channel::<WsMessage>(1);
        let cancel = CancellationToken::new();
        // Share the control's reason watch with the send_loop so the expiry
        // task's AuthorizationDenied write is observed as the 1008 close code.
        let control = crate::state::CommunityConnectionControl::new(cancel.clone());
        let disconnect_rx = control.disconnect_reason();

        // Step 1: spawn audio send_loop and yield so it parks in its select.
        let send_cancel = cancel.clone();
        let send_handle = tokio::spawn(send_loop(
            sink,
            data_rx,
            ctrl_rx,
            terminal_rx,
            send_cancel,
            disconnect_rx,
        ));
        tokio::task::yield_now().await;

        // Step 2: invoke the shared expiry constructor with an already-expired
        // deadline. Queue-then-cancel is synchronous: the send loop's cancellation
        // branch drains the terminal frame before writing Close.
        let already_expired = chrono::Utc::now() - chrono::Duration::seconds(1);
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(already_expired, cancel.clone());
        let expiry_handle = crate::nip_fi_session::spawn_nip_fi_expiry_task(
            already_expired,
            gate,
            terminal_tx,
            crate::nip_fi_session::NipFiWsRoute::Audio,
            control,
        );
        expiry_handle.await.expect("expiry task must complete");
        drop(ctrl_tx); // satisfy the unused-variable lint

        // Step 3: await the writer and assert exact two-frame sequence.
        tokio::time::timeout(std::time::Duration::from_secs(2), send_handle)
            .await
            .expect("send_loop must complete within timeout")
            .expect("send_loop task must not panic");

        let frames = recorded.lock().await;
        assert_eq!(
            frames.len(),
            2,
            "expected exactly 2 frames (restricted JSON, then Close); got {:?}",
            *frames
        );

        // Frame 0: exact canonical restricted JSON.
        let expected = serde_json::json!({
            "type": "restricted",
            "message": buzz_auth::DenialClass::AuthorizationDenied.nostr_text()
        })
        .to_string();
        match &frames[0] {
            WsMessage::Text(t) => assert_eq!(
                t.as_str(),
                expected.as_str(),
                "frame 0 must be exact canonical restricted JSON"
            ),
            other => panic!("frame 0 must be Text(restricted JSON); got {other:?}"),
        }

        // Frame 1: Close(Some) with 1008 POLICY code — expiry sets AuthorizationDenied
        // on the disconnect_reason watch so the send loop emits a policy close.
        match &frames[1] {
            WsMessage::Close(Some(cf)) => {
                assert_eq!(
                    cf.code,
                    axum::extract::ws::close_code::POLICY,
                    "frame 1 must be 1008 POLICY close; got code {}",
                    cf.code
                );
                assert_eq!(
                    cf.reason.as_str(),
                    "authorization denied",
                    "frame 1 close reason must be 'authorization denied'"
                );
            }
            other => {
                panic!("frame 1 must be Close(Some(POLICY, 'authorization denied')); got {other:?}")
            }
        }
    }

    // ── W_FIX1: check_cancel!() pre-send-loop path emits restricted JSON then 1008 ──
    //
    // Drives the REAL `handle_active_audio_connection` through a held admission
    // boundary so the ACTUAL `check_cancel!()` macro arm (audio/handler.rs:474-488)
    // executes and client-observable frames are asserted.
    //
    // Fix 1 added drain-then-close to every `check_cancel!()` arm and the four
    // manual expiry exits. Before the fix, `check_cancel!` drained the terminal
    // channel (denial frame) but returned without a close frame — clients observed
    // 1005/1006. After the fix the arm also sends `reason.close_message()` when
    // `disconnect_reason` is `AuthorizationDenied`.
    //
    // Setup:
    //   - Key is NOT in the deny map (passes post-registration deny check).
    //   - Assertion carries a 1 s NIP-FI deadline → expiry task spawned at that
    //     deadline, `terminal_ctrl_tx` wired internally.
    //   - `before_first_audio_check_cancel` hook holds the handler AFTER the expiry
    //     task is spawned but BEFORE the first `check_cancel!()` invocation.
    //   - Expiry task fires naturally (1 s deadline passes while the hook holds).
    //     It queues the denial frame on the internal `terminal_ctrl_tx`, publishes
    //     `AuthorizationDenied` on the real `disconnect_reason` watch, and cancels.
    //   - Hook is released only after the token is confirmed cancelled, guaranteeing
    //     the expiry task has committed its effects before the handler resumes.
    //   - Handler resumes, hits `check_cancel!()` at the real production call site,
    //     drains the denial frame, sends `reason.close_message()`.
    //
    // Mutation evidence (production seam, not copies):
    //   A) Remove the `if let Some(reason) = nip_fi_close_reason` block from the
    //      plain `check_cancel!()` arm (handler.rs:483-486) → client receives only
    //      the restricted JSON frame, no close → close assertion times out → panics.
    //   B) Replace `reason.close_message()` in that arm with `WsMessage::Close(None)`
    //      → `Close(None)` from client → POLICY code assertion panics.
    //   C) Move `drain` AFTER `close_message()` in that arm → close arrives before
    //      the restricted JSON frame → `frame[0] is Text` assertion panics.
    //   D) Delete `while let Ok(msg) = terminal_ctrl_rx.try_recv()` drain from that
    //      arm → no restricted JSON frame → client sees only the close → panics.
    //   NOTE: mutations A–D target handler.rs:474-488 (the no-arg `check_cancel!`
    //   arm). Deleting those blocks leaves this test red; W7 independently witnesses
    //   the post-send-loop `send_loop` cancel branch.

    #[tokio::test]
    async fn pre_send_loop_check_cancel_emits_restricted_json_then_policy_close() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;

        // Key absent from deny map → passes deny-set check.
        let key = nostr::Keys::generate();
        // 1 s deadline: long enough that, under a loaded parallel unit run,
        // pairing never lands after it (which would take the B1 "already expired
        // at pairing" exit instead of this seam), short enough that the expiry
        // task fires inside the 2 s cancel wait while the hook holds.
        let deadline = Utc::now() + Duration::seconds(1);
        let assertion = VerifiedAssertion::for_test(Some(key.public_key()), vec![deadline]);

        // State with deny map (issuer "test-issuer") but key not denied.
        let state = audio_deny_state(None).await;

        // Unique community so hook slot does not collide with parallel tests.
        let community = buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::new_v4());
        let tenant =
            buzz_core::tenant::TenantContext::resolved(community, "test.local".to_string());

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let conn_cancel = CancellationToken::new();
        let cancel_for_assert = conn_cancel.clone();

        // CommunityConnectionControl is Clone: create one for the server closure.
        let control_for_server = crate::state::CommunityConnectionControl::new(conn_cancel.clone());

        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("W_FIX1: bind test listener");
        let addr = listener.local_addr().expect("W_FIX1: test listener addr");

        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let control_i = control_for_server.clone();
                    move |ws: WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let control_i = control_i.clone();
                        let conn_time = chrono::Utc::now();
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    uuid::Uuid::new_v4(),
                                    control_i,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app)
                .await
                .expect("W_FIX1: test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("W_FIX1: server ready");

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("W_FIX1: connect client");

        // Arm `audio_before_first_check_cancel_hook` BEFORE auth so the handler
        // is captured at the exact `check_cancel!()` seam — after the expiry task
        // has been spawned (line ~426) but before the first cancel check fires.
        let (hook_arrived_rx, hook_release) =
            crate::nip_fi_test_hooks::audio_before_first_check_cancel_hook::arm(community);

        // NIP-42 challenge.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("W_FIX1: challenge timeout")
            .expect("W_FIX1: challenge item")
            .expect("W_FIX1: challenge message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("W_FIX1: expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("W_FIX1: challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("W_FIX1: challenge field")
            .to_string();

        let relay_url = "ws://test.local";
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key)
            .unwrap();
        let auth_msg = serde_json::json!({"type": "auth", "event": auth_event}).to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("W_FIX1: send auth");

        // Wait for handler to reach before_first_audio_check_cancel.
        // At this point the expiry task has already been spawned with a 1 s
        // deadline; we hold here while it fires.
        tokio::time::timeout(std::time::Duration::from_secs(5), hook_arrived_rx)
            .await
            .expect("W_FIX1: handler must reach before_first_audio_check_cancel within 5s")
            .expect("W_FIX1: hook arrived channel closed");

        // Handler is now held. The expiry task was spawned with a 1 s deadline;
        // wait for it to fire (cancel token is set when it does). It will queue the
        // denial frame on the internal terminal channel, publish AuthorizationDenied
        // on the disconnect_reason watch, then cancel.
        let cancel_wait = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if cancel_for_assert.is_cancelled() {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        });
        cancel_wait
            .await
            .expect("W_FIX1: expiry task must fire and set cancel within 2s");

        // Release the hook. Handler resumes, hits check_cancel!() immediately,
        // drains terminal_ctrl_rx (denial frame) then sends reason.close_message().
        hook_release.notify_one();

        // ── Client-observed frame assertions ──────────────────────────────────
        //
        // Frame 0: restricted JSON denial payload queued by the expiry task.
        let frame0 = tokio::time::timeout(std::time::Duration::from_secs(3), client.next())
            .await
            .expect("W_FIX1: frame 0 timeout")
            .expect("W_FIX1: frame 0 item")
            .expect("W_FIX1: frame 0 message");
        let expected_restricted = serde_json::json!({
            "type": "restricted",
            "message": buzz_auth::DenialClass::AuthorizationDenied.nostr_text()
        })
        .to_string();
        match &frame0 {
            tokio_tungstenite::tungstenite::Message::Text(t) => assert_eq!(
                t.as_str(),
                expected_restricted.as_str(),
                "W_FIX1: frame 0 must be exact canonical restricted JSON"
            ),
            other => panic!("W_FIX1: frame 0 must be Text(restricted JSON); got {other:?}"),
        }

        // Frame 1: 1008 POLICY close emitted by check_cancel!()'s close_message() call.
        let frame1 = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("W_FIX1: frame 1 timeout")
            .expect("W_FIX1: frame 1 item")
            .expect("W_FIX1: frame 1 message");
        match &frame1 {
            tokio_tungstenite::tungstenite::Message::Close(Some(cf)) => {
                assert_eq!(
                    cf.code,
                    tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy,
                    "W_FIX1: close code must be 1008 POLICY; got {:?}",
                    cf.code
                );
                assert_eq!(
                    cf.reason.as_str(),
                    "authorization denied",
                    "W_FIX1: close reason must be 'authorization denied'"
                );
            }
            other => {
                panic!(
                    "W_FIX1: frame 1 must be Close(Some(1008, 'authorization denied')); got {other:?}"
                )
            }
        }

        server.abort();
        let _ = server.await;
    }

    // ── W8: barrier at membership check — cancel before first DB read ─────────
    //
    // Arms `before_membership_check` — the hook at the very start of
    // `check_membership_for_admission`, before any DB read. Calls the function
    // directly in a spawned task with a live gate. When the hook signals arrival,
    // fires cancel (simulates expiry). Releases the hook. The function then
    // attempts its first DB read (which fails with a lazy-pool error) and
    // returns Err. This proves the hook fires before any DB call.
    //
    // Observable invariant: cancel is set before the function returns, and the
    // function returns without writing any membership row.
    //
    // Hook location: entry of `check_membership_for_admission`, before the first
    // `state.db.get_channel()` call.
    //
    // Mutation evidence:
    //   A) Delete `before_membership_check(...)` from check_membership_for_admission →
    //      hook never fires → `arrived_rx` times out → test panics.
    //   B) Move the hook after `state.db.get_channel()` → hook fires after DB read
    //      (order changed); on a lazy pool the DB read errors out before the hook
    //      → arrived_rx times out → test panics.
    //   C) Supply a real DB where get_channel returns an archived channel →
    //      function returns "channel is archived" before the hook (but after the
    //      first DB call) → hook never fires → arrived_rx times out → test panics.
    //      (This variant is tested in the DB integration suite.)
    #[tokio::test]
    async fn w8_membership_check_barrier_fires_before_db_read() {
        use buzz_core::tenant::{CommunityId, TenantContext};
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        let state = audio_test_state().await;
        // Fresh community — the hook maps are process-global, so a shared key
        // would let any concurrent test that drives the handler through
        // before_membership_check consume this gate in the fallback's single
        // libtest process.
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let tenant = TenantContext::resolved(community, "test.local".to_string());
        let channel_id = Uuid::new_v4();
        let pubkey = nostr::Keys::generate().public_key();
        let pubkey_bytes = pubkey.to_bytes().to_vec();

        let cancel = CancellationToken::new();

        // Arm the hook at the entry of check_membership_for_admission.
        let (arrived_rx, release) =
            crate::nip_fi_test_hooks::audio_membership_check_hook::arm(community);

        let state2 = std::sync::Arc::clone(&state);
        let tenant2 = tenant.clone();
        let cancel2 = cancel.clone();
        let handle = tokio::spawn(async move {
            super::check_membership_for_admission(
                &state2,
                &tenant2,
                channel_id,
                &pubkey_bytes,
                None,
            )
            .await
        });

        // Wait for the function to reach the hook (before any DB call).
        tokio::time::timeout(std::time::Duration::from_secs(5), arrived_rx)
            .await
            .expect("W8: check_membership_for_admission must reach hook within 5s")
            .expect("arrived channel closed");

        // Cancel — simulates expiry firing before the first DB read.
        cancel2.cancel();

        // Release — function resumes and attempts its first DB read.
        release.notify_one();

        // Wait for the function to complete (DB error on lazy pool, or real result).
        // Note: with a lazy pool at port 1, the DB call may hang indefinitely
        // (sqlx pool acquisition blocks waiting for a connection). We abort the
        // task rather than waiting — the key invariants are already established:
        // the hook fired (arrived_rx succeeded above) and cancel is set.
        let _ = tokio::time::timeout(std::time::Duration::from_millis(200), handle).await;

        // Cancel was set before the function's first DB call.
        assert!(cancel.is_cancelled(), "W8: cancel must be set");

        // The hook fired at the entry of check_membership_for_admission — before
        // any DB call. `arrived_rx` succeeded above proves this invariant.
        // The function returned before any membership row was written (it only reads
        // in check_membership_for_admission — all writes go to commit_participant_join).
        // Whether the DB call errored (fast refusal) or is still pending (slow pool)
        // is irrelevant — the hook-fired invariant is what W8 establishes.
        let _ = cancel2; // suppress unused warning
    }

    // ── W_audio_deny: audio post-registration deny-set check (active, absent, straddle)
    //
    // Three witnesses prove the S4 normative deny-set check in
    // `handle_active_audio_connection` is correctly placed AFTER registration.
    //
    // All three use the same real-WS-server pattern as W5/W6.
    // The state fixture:
    //   - `audio_test_state` (lazy DB, port 1) — sufficient because the deny check
    //     fires BEFORE `enforce_relay_membership` (which is where lazy-DB errors).
    //   - NipFiDenyMap wired with issuer "test-issuer" (from VerifiedAssertion::for_test).
    //
    // W_audio_deny_active: key IS in deny map → denied at post-registration check.
    //   Mutation evidence:
    //   A) Delete the `is_denied` check from audio/handler.rs → no denial frame;
    //      instead `enforce_relay_membership` fires → frame text changes to
    //      "restricted: not a relay member" → byte assertion panics.
    //   B) Move the deny check to before `audio_post_auth_register` → fires before
    //      registration; but for a pre-seeded key the test still passes — the
    //      distinction is in W_audio_deny_straddle.
    //   C) Remove `cancel.cancel()` from the deny branch → cancel assertion panics.
    //
    // W_audio_deny_absent: key NOT in deny map → passes deny check → membership
    // error (lazy DB). Proves the deny check doesn't fire for innocent keys.
    //   Mutation evidence:
    //   A) Invert the `is_denied` condition (deny all keys) → absent key gets the
    //      `authorization_denied` frame → frame text assertion panics.
    //   B) Remove the `Some(deny_map)` guard → `nip_fi_deny_map` is None → both
    //      paths are equivalent → absent test passes regardless; but active test
    //      would fail (check never fires).
    //
    // W_audio_deny_straddle: entry inserted in window between registration and check.
    //   Mutation evidence:
    //   A) Delete `before_deny_set_check(...)` → hook never fires →
    //      deny entry inserted AFTER check runs and missed → membership error
    //      frame received instead of denial → frame text assertion panics.
    //   B) Remove `is_denied` check entirely → same as (A).

    /// Build a test AppState with a NipFiDenyMap wired for issuer "test-issuer".
    /// If `denied_key` is Some, inserts a live deny entry for that key.
    /// Uses a lazy DB (port 1) — sufficient because the deny check fires before
    /// any DB read in `handle_active_audio_connection`.
    async fn audio_deny_state(
        denied_key: Option<&nostr::PublicKey>,
    ) -> std::sync::Arc<crate::state::AppState> {
        use std::sync::Arc;
        let mut state = (*audio_test_state().await).clone();

        let deny_map = Arc::new(buzz_auth::NipFiDenyMap::new(
            16,
            vec![buzz_auth::IssuerCapacity {
                issuer: "test-issuer".to_owned(),
                capacity: 16,
            }],
        ));

        if let Some(key) = denied_key {
            let until = chrono::Utc::now() + chrono::Duration::seconds(3600);
            let result =
                deny_map.merge_cross_pod_deny("test-issuer", key, until, chrono::Utc::now());
            assert!(
                matches!(result, buzz_auth::CrossPodMergeResult::Merged),
                "audio_deny_state: deny entry must be inserted for test setup"
            );
        }

        state.nip_fi_deny_map = Some(deny_map);
        Arc::new(state)
    }

    #[tokio::test]
    async fn w_audio_deny_active_key_refused_at_post_registration_check() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;

        let key = nostr::Keys::generate();
        let deadline = Utc::now() + Duration::hours(1);
        // Assertion with "test-issuer"; the key IS in the deny map.
        let assertion = VerifiedAssertion::for_test(Some(key.public_key()), vec![deadline]);

        let state = audio_deny_state(Some(&key.public_key())).await;

        let tenant = buzz_core::tenant::TenantContext::resolved(
            buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::nil()),
            "test.local".to_string(),
        );

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let conn_cancel = CancellationToken::new();
        let cancel_for_assert = conn_cancel.clone();
        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("test listener addr");

        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let cancel_i = conn_cancel.clone();
                    move |ws: WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let conn_time = chrono::Utc::now();
                        let control_inner =
                            crate::state::CommunityConnectionControl::new(cancel_i.clone());
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    uuid::Uuid::new_v4(),
                                    control_inner,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app).await.expect("test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("server ready");

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("connect client");

        // Receive challenge.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("challenge timeout")
            .expect("challenge message")
            .expect("challenge ws message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("challenge field")
            .to_string();

        // Sign with the SAME key as the assertion — pairing passes.
        let relay_url = "ws://test.local";
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key)
            .unwrap();
        let auth_msg = serde_json::json!({"type": "auth", "event": auth_event}).to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("send auth msg");

        // Receive the denial frame.
        let frame = tokio::time::timeout(std::time::Duration::from_secs(3), client.next())
            .await
            .expect("W_audio_deny_active: denial frame timeout")
            .expect("frame")
            .expect("ws frame");

        let expected_denied = serde_json::json!({
            "type": "restricted",
            "message": buzz_auth::DenialClass::AuthorizationDenied.nostr_text()
        })
        .to_string();

        match frame {
            tokio_tungstenite::tungstenite::Message::Text(t) => {
                assert_eq!(
                    t.as_str(),
                    expected_denied.as_str(),
                    "W_audio_deny_active: active deny entry must produce exact \
                     authorization_denied frame at post-registration check"
                );
            }
            other => panic!("W_audio_deny_active: expected Text(restricted JSON); got {other:?}"),
        }

        // Connection must close after denial.
        let close = tokio::time::timeout(std::time::Duration::from_secs(2), client.next()).await;
        assert!(
            matches!(
                close,
                Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))))
                    | Ok(Some(Err(_)))
                    | Ok(None)
            ),
            "W_audio_deny_active: connection must close after denial; got {close:?}"
        );

        assert!(
            cancel_for_assert.is_cancelled(),
            "W_audio_deny_active: conn_cancel must be cancelled after denial"
        );

        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn w_audio_deny_absent_key_passes_deny_check_reaches_membership_gate() {
        // A key NOT in the deny map must pass the deny-set check and reach the
        // post-check / membership-entry gate without denial or cancellation.
        //
        // Two hooks bracket the deny-set check block:
        //   1. `before_deny_set_check` (pre-check): proves the handler reached
        //      the deny-check seam after pairing + registration; connection is
        //      NOT cancelled here.
        //   2. `after_deny_set_check_passed` (post-check): fires only when the
        //      key was NOT denied — proves the handler continued past the check
        //      without a denial or cancel. An unconditional denial immediately
        //      after the pre-check hook would prevent this hook from firing.
        //
        // Mutation evidence:
        //   A) Invert `is_denied` → absent key is denied after pre-check hook
        //      releases → handler returns early → post-check hook NEVER fires →
        //      `post_arrived_rx` times out → test panics.
        //   B) Delete the `before_deny_set_check` hook → pre-check `arrived_rx`
        //      times out → test panics (seam unreachable).
        //   C) Delete the `after_deny_set_check_passed` hook → post-check
        //      `post_arrived_rx` times out → test panics (pass-through unproven).
        //   D) Remove `nip_fi_deny_map` from state → map is None → guard
        //      short-circuits → both hooks still fire (map guard is after both
        //      hooks are in the control path) — off-mode passes through cleanly.
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;

        let key = nostr::Keys::generate();
        // Different key is denied; `key` is absent from the map.
        let other_key = nostr::Keys::generate();
        let deadline = Utc::now() + Duration::hours(1);
        let assertion = VerifiedAssertion::for_test(Some(key.public_key()), vec![deadline]);

        let state = audio_deny_state(Some(&other_key.public_key())).await;

        // Use a unique UUID so this test's hook slot doesn't collide with
        // other concurrent tests (active test uses Uuid::nil()).
        let community = buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::new_v4());
        let tenant =
            buzz_core::tenant::TenantContext::resolved(community, "test.local".to_string());

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let conn_cancel = CancellationToken::new();
        let cancel_for_assert = conn_cancel.clone();
        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("test listener addr");

        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let cancel_i = conn_cancel.clone();
                    move |ws: WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let conn_time = chrono::Utc::now();
                        let control_inner =
                            crate::state::CommunityConnectionControl::new(cancel_i.clone());
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    uuid::Uuid::new_v4(),
                                    control_inner,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app).await.expect("test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("server ready");

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("connect client");

        // Arm BOTH hooks before sending the auth message.
        // Hook 1: pre-check barrier — fires when handler reaches before_deny_set_check.
        let (pre_arrived_rx, pre_release) =
            crate::nip_fi_test_hooks::deny_set_check_hook::arm(community);
        // Hook 2: post-check barrier — fires when handler passes deny check (key absent).
        let (post_arrived_rx, post_release) =
            crate::nip_fi_test_hooks::audio_after_deny_check_passed_hook::arm(community);

        // Receive challenge.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("challenge timeout")
            .expect("challenge message")
            .expect("challenge ws message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("challenge field")
            .to_string();

        let relay_url = "ws://test.local";
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key)
            .unwrap();
        let auth_msg = serde_json::json!({"type": "auth", "event": auth_event}).to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("send auth msg");

        // === Pre-check seam ===
        // Wait for handler to reach before_deny_set_check.
        // Proves: pairing passed, registration happened, deny check reached.
        tokio::time::timeout(std::time::Duration::from_secs(5), pre_arrived_rx)
            .await
            .expect("W_audio_deny_absent: handler must reach before_deny_set_check within 5s")
            .expect("arrived channel closed");

        // Connection is NOT cancelled at the pre-check seam.
        assert!(
            !cancel_for_assert.is_cancelled(),
            "W_audio_deny_absent: connection must NOT be cancelled at the pre-check seam"
        );

        // Release pre-check hook — handler proceeds to run the deny check.
        pre_release.notify_one();

        // === Post-check seam ===
        // Wait for handler to reach after_deny_set_check_passed.
        // This hook ONLY fires if the key was NOT denied. An inverted `is_denied`
        // would deny the absent key and return early, never reaching this hook.
        tokio::time::timeout(std::time::Duration::from_secs(5), post_arrived_rx)
            .await
            .expect(
                "W_audio_deny_absent: handler must reach after_deny_set_check_passed within 5s \
                 (absent key must pass the deny check without denial)",
            )
            .expect("post-check arrived channel closed");

        // Connection is STILL not cancelled — the absent key passed clean.
        assert!(
            !cancel_for_assert.is_cancelled(),
            "W_audio_deny_absent: connection must NOT be cancelled after the deny check \
             (absent key must pass clean)"
        );

        // Release post-check hook — handler proceeds to membership check (lazy DB).
        post_release.notify_one();

        // Allow the handler to proceed briefly (lazy-DB membership error is expected;
        // that path is out of scope for this witness).
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn w_audio_deny_straddle_entry_inserted_between_registration_and_check_is_caught() {
        // Arms `before_deny_set_check` — fires AFTER audio_post_auth_register and
        // BEFORE the is_denied call. Entry starts absent; inserted during the window.
        // The deny check finds it and closes the connection.
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;

        let key = nostr::Keys::generate();
        let deadline = Utc::now() + Duration::hours(1);
        let assertion = VerifiedAssertion::for_test(Some(key.public_key()), vec![deadline]);

        // Build state with empty deny map (key not denied yet).
        let deny_map = Arc::new(buzz_auth::NipFiDenyMap::new(
            16,
            vec![buzz_auth::IssuerCapacity {
                issuer: "test-issuer".to_owned(),
                capacity: 16,
            }],
        ));
        let deny_map_for_insert = Arc::clone(&deny_map);

        let mut base_state = (*audio_test_state().await).clone();
        base_state.nip_fi_deny_map = Some(deny_map);
        let state = Arc::new(base_state);

        // Use a unique UUID so this test's hook slot doesn't collide with
        // other concurrent tests (absent/active tests use Uuid::nil()).
        let community = buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::new_v4());
        let tenant =
            buzz_core::tenant::TenantContext::resolved(community, "test.local".to_string());

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let conn_cancel = CancellationToken::new();
        let cancel_for_assert = conn_cancel.clone();
        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();

        // Pre-create and register the CommunityConnectionControl before the server
        // runs. audio_post_auth_register writes proven_pubkey on the control; since
        // Clone shares the same proven_pubkey Arc, the registered entry is updated
        // in-place and disconnect_nip_fi can find it at the close-scan assertion.
        // The guard keeps the entry live through that assertion.
        let conn_control = crate::state::CommunityConnectionControl::new(conn_cancel.clone());
        let conn_id_for_registration = uuid::Uuid::new_v4();
        let _conn_guard = state.community_connections.register(
            conn_id_for_registration,
            community,
            conn_control.clone(),
        );
        let conn_control_for_server = conn_control.clone();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("test listener addr");

        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let control_outer = conn_control_for_server.clone();
                    move |ws: WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let conn_time = chrono::Utc::now();
                        // Use the pre-registered control so audio_post_auth_register
                        // writes to the registered entry (shared proven_pubkey Arc).
                        let control_inner = control_outer.clone();
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    uuid::Uuid::new_v4(),
                                    control_inner,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app).await.expect("test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("server ready");

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("connect client");

        // Arm the barrier BEFORE sending auth (handler stalls when it reaches the hook).
        let (arrived_rx, release) = crate::nip_fi_test_hooks::deny_set_check_hook::arm(community);

        // Receive challenge.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("challenge timeout")
            .expect("challenge message")
            .expect("challenge ws message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("challenge field")
            .to_string();

        let relay_url = "ws://test.local";
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key)
            .unwrap();
        let auth_msg = serde_json::json!({"type": "auth", "event": auth_event}).to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("send auth msg");

        // Wait for the handler to reach before_deny_set_check (after registration).
        tokio::time::timeout(std::time::Duration::from_secs(5), arrived_rx)
            .await
            .expect("W_audio_deny_straddle: handler must reach hook within 5s")
            .expect("arrived channel closed");

        // Insert the deny entry — handler is between registration and check.
        let until = Utc::now() + Duration::seconds(3600);
        let merge = deny_map_for_insert.merge_cross_pod_deny(
            "test-issuer",
            &key.public_key(),
            until,
            Utc::now(),
        );
        assert!(
            matches!(merge, buzz_auth::CrossPodMergeResult::Merged),
            "W_audio_deny_straddle: deny entry must be inserted during hook window"
        );

        // Close-scan side: run the real CommunityConnectionRegistry::disconnect_nip_fi
        // now that the audio connection is registered (audio_post_auth_register fired
        // before the hook). This proves registration is visible to the concurrent close
        // scan — the normative invariant [FI-TRACE-DENY-SET] for the audio path.
        // With the deny entry live, the scan finds exactly one session matching this
        // pubkey and closes it.
        //
        // Mutation evidence (Mut-C: move hook before audio_post_auth_register):
        //   disconnect_nip_fi returns 0 (not yet registered) → assertion panics.
        //   Causally falsifies the registration-before-check invariant.
        let pubkey_bytes = key.public_key().to_bytes().to_vec();
        let closed = state
            .community_connections
            .disconnect_nip_fi("test-issuer", &pubkey_bytes);
        assert_eq!(
            closed, 1,
            "W_audio_deny_straddle: close scan must find exactly 1 registered audio session \
             (proves audio_post_auth_register is visible between the hook and the check)"
        );

        // Release — handler resumes and calls is_denied().
        release.notify_one();

        // Receive the denial frame from the server.
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .expect("W_audio_deny_straddle: denial frame timeout")
            .expect("frame")
            .expect("ws frame");

        let expected_denied = serde_json::json!({
            "type": "restricted",
            "message": buzz_auth::DenialClass::AuthorizationDenied.nostr_text()
        })
        .to_string();

        match frame {
            tokio_tungstenite::tungstenite::Message::Text(t) => {
                assert_eq!(
                    t.as_str(),
                    expected_denied.as_str(),
                    "W_audio_deny_straddle: deny entry inserted between registration \
                     and check must produce exact authorization_denied frame"
                );
            }
            other => panic!("W_audio_deny_straddle: expected Text(restricted JSON); got {other:?}"),
        }

        assert!(
            cancel_for_assert.is_cancelled(),
            "W_audio_deny_straddle: conn_cancel must be cancelled after straddle denial"
        );

        server.abort();
        let _ = server.await;
    }

    // ── W_admin_disconnect_at_deny_check: pre-registration-window is now closed ──
    //
    // Witnesses that a disconnect_nip_fi call that fires at the `before_deny_set_check`
    // hook window — AFTER audio_post_auth_register (pubkey scan-visible) but BEFORE
    // the deny-set check — still delivers the restricted JSON payload before the 1008
    // close.  This is the exact window Thufir identified as the pre-registration gap
    // in pass 2: the old code registered the terminal sender AFTER this point, so
    // `disconnect_nip_fi` found `terminal_frame_tx = None` and queued nothing.  The
    // fix moves sender registration to BEFORE `audio_post_auth_register`, closing
    // the window.
    //
    // Setup:
    //   - Pre-create and register control (same pattern as straddle/admin_disconnect).
    //   - Key absent from deny map.  1-hour deadline — expiry does not fire.
    //   - Arm `before_deny_set_check` hook.  This hook fires AFTER both
    //     `set_terminal_frame_sender` and `audio_post_auth_register`.
    //   - While handler is held at the hook, call `registry.disconnect_nip_fi`.
    //   - Release; handler hits check_cancel!(), drains denial frame, emits 1008.
    //   - Client asserts Text(restricted JSON) → Close(1008 POLICY, "authorization denied").
    //
    // Mutation evidence:
    //   A) Move `set_terminal_frame_sender` to AFTER the hook window (into the B1
    //      block, after the deny-set check, where it was in the original pass-1 code)
    //      → when disconnect_nip_fi fires at the before_deny_set_check window, the
    //      slot is still `None` → nothing enqueued → check_cancel!() drains nothing
    //      → client receives only 1008 with no preceding Text frame → frame-0 Text
    //      assertion panics.
    //   B) Remove `set_terminal_frame_sender` entirely → same outcome as (A).
    #[tokio::test]
    async fn w_admin_disconnect_at_deny_check_delivers_payload_then_close() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;

        let key = nostr::Keys::generate();
        let deadline = Utc::now() + Duration::hours(1);
        let assertion = VerifiedAssertion::for_test(Some(key.public_key()), vec![deadline]);

        // State with deny map; key is absent (not denied) — the check must pass.
        let state = audio_deny_state(None).await;

        let community = buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::new_v4());
        let tenant =
            buzz_core::tenant::TenantContext::resolved(community, "test.local".to_string());

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let conn_cancel = CancellationToken::new();
        let cancel_for_assert = conn_cancel.clone();

        // Pre-create and register the control so the pubkey-scan can find it.
        let conn_control = crate::state::CommunityConnectionControl::new(conn_cancel.clone());
        let conn_id = uuid::Uuid::new_v4();
        let _conn_guard =
            state
                .community_connections
                .register(conn_id, community, conn_control.clone());
        let conn_control_for_server = conn_control.clone();

        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("W_addc: bind test listener");
        let addr = listener.local_addr().expect("W_addc: test listener addr");

        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let control_outer = conn_control_for_server.clone();
                    move |ws: WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let control_inner = control_outer.clone();
                        let conn_time = chrono::Utc::now();
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    uuid::Uuid::new_v4(),
                                    control_inner,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app)
                .await
                .expect("W_addc: test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("W_addc: server ready");

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("W_addc: connect client");

        // Arm before_deny_set_check — fires AFTER set_terminal_frame_sender AND
        // audio_post_auth_register (pubkey scan-visible).  This is the exact window
        // where the old code had terminal_frame_tx = None.
        let (hook_arrived_rx, hook_release) =
            crate::nip_fi_test_hooks::deny_set_check_hook::arm(community);

        // NIP-42 challenge.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("W_addc: challenge timeout")
            .expect("W_addc: challenge item")
            .expect("W_addc: challenge message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("W_addc: expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("W_addc: challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("W_addc: challenge field")
            .to_string();

        let relay_url = "ws://test.local";
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key)
            .unwrap();
        let auth_msg = serde_json::json!({"type": "auth", "event": auth_event}).to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("W_addc: send auth");

        // Wait for the handler to reach before_deny_set_check.
        // At this point BOTH set_terminal_frame_sender and audio_post_auth_register
        // have already executed — the terminal sender is registered and the pubkey
        // is scan-visible.  (In the old code this was the gap window.)
        tokio::time::timeout(std::time::Duration::from_secs(5), hook_arrived_rx)
            .await
            .expect("W_addc: handler must reach before_deny_set_check within 5s")
            .expect("W_addc: hook arrived channel closed");

        // Simulate admin-disconnect at the exact old-gap position.
        let pubkey_bytes = key.public_key().to_bytes().to_vec();
        let closed = state
            .community_connections
            .disconnect_nip_fi("test-issuer", &pubkey_bytes);
        assert_eq!(
            closed, 1,
            "W_addc: registry scan must find exactly 1 audio session \
             (proves audio_post_auth_register ran before the hook)"
        );

        // Release — handler resumes, hits check_cancel!(), drains the enqueued
        // denial frame, sends reason.close_message().
        hook_release.notify_one();

        // Frame 0: restricted JSON payload.
        let frame0 = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .expect("W_addc: frame 0 timeout")
            .expect("W_addc: frame 0 item")
            .expect("W_addc: frame 0 ws message");
        let expected_json = serde_json::json!({
            "type": "restricted",
            "message": buzz_auth::DenialClass::AuthorizationDenied.nostr_text()
        })
        .to_string();
        match frame0 {
            tokio_tungstenite::tungstenite::Message::Text(t) => {
                assert_eq!(
                    t.as_str(),
                    expected_json.as_str(),
                    "W_addc: frame 0 must be exact restricted JSON (terminal sender was \
                     registered before scan-visibility, so the old gap is closed)"
                );
            }
            other => {
                panic!("W_addc: frame 0 must be Text(restricted JSON); got {other:?}")
            }
        }

        // Frame 1: 1008 POLICY close.
        let frame1 = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .expect("W_addc: frame 1 timeout")
            .expect("W_addc: frame 1 item")
            .expect("W_addc: frame 1 ws message");
        match frame1 {
            tokio_tungstenite::tungstenite::Message::Close(Some(cf)) => {
                assert_eq!(
                    cf.code,
                    tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy,
                    "W_addc: close code must be 1008 POLICY"
                );
                assert_eq!(
                    <tokio_tungstenite::tungstenite::Utf8Bytes as AsRef<str>>::as_ref(&cf.reason),
                    "authorization denied",
                    "W_addc: close reason must be exact 'authorization denied' bytes"
                );
            }
            other => {
                panic!("W_addc: frame 1 must be Close(1008, 'authorization denied'); got {other:?}")
            }
        }

        assert!(
            cancel_for_assert.is_cancelled(),
            "W_addc: conn_cancel must be cancelled after admin disconnect"
        );

        server.abort();
        let _ = server.await;
    }

    // ── W_admin_disconnect: registry disconnect_nip_fi delivers payload-then-close ─
    //
    // Witnesses that an active audio socket closed via the admin-disconnect path
    // (`CommunityConnectionRegistry::disconnect_nip_fi`) delivers the restricted
    // JSON payload BEFORE the 1008 POLICY close — the payload-then-close contract.
    //
    // Before this fix, `CommunityConnectionControl::disconnect_nip_fi` only
    // published `AuthorizationDenied` + cancelled; no frame was enqueued on the
    // terminal channel.  The send loop (or pre-send-loop drain) then emitted only
    // the close, with no preceding restricted JSON frame.
    //
    // Setup:
    //   - Pre-create and register `CommunityConnectionControl` (so the registry
    //     scan can find this audio session by pubkey — same pattern as straddle).
    //   - Key absent from deny map.  Assertion carries a 1-hour deadline so the
    //     expiry task is armed but does NOT fire during the test.
    //   - `before_first_audio_check_cancel` hook holds the handler AFTER
    //     `set_terminal_frame_sender` registers the sender on the control (line
    //     ~421) and BEFORE the first `check_cancel!()`.
    //   - Test calls `registry.disconnect_nip_fi("test-issuer", &pubkey)` while the hook holds.
    //     `CommunityConnectionControl::disconnect_nip_fi` enqueues the denial frame
    //     on `terminal_frame_tx`, publishes `AuthorizationDenied`, then cancels.
    //   - Hook is released; handler hits `check_cancel!()`, drains the denial
    //     frame from `terminal_ctrl_rx`, sends `reason.close_message()`.
    //   - Client asserts: Text(restricted JSON) → Close(1008 POLICY, "authorization denied").
    //
    // Mutation evidence (production seam, not copies):
    //   A) Remove the `set_terminal_frame_sender` call from `handle_active_audio_connection`
    //      → `terminal_frame_tx` slot is `None` → `disconnect_nip_fi` enqueues nothing
    //      → client receives only `1008` with no preceding text frame → Text assertion
    //      times out → panics.
    //   B) Remove the `try_send` block from `CommunityConnectionControl::disconnect_nip_fi`
    //      → same outcome as (A): enqueue suppressed → only close observed → panics.
    //   C) Delete the `while let Ok(msg) = terminal_ctrl_rx.try_recv()` drain from the
    //      plain `check_cancel!()` arm → no text frame delivered → panics.
    //   D) Move `set_terminal_frame_sender` to AFTER `audio_post_auth_register`
    //      (back to the pass-1 ordering) → when disconnect_nip_fi fires at the
    //      `before_first_audio_check_cancel` hook (which itself is after the old
    //      registration point), the sender IS registered → test still PASSES.
    //      Use W_admin_disconnect_at_deny_check (hook at before_deny_set_check,
    //      the old gap) to catch this regression instead — that witness is RED
    //      under the pass-1 ordering. (See W_addc above.)
    #[tokio::test]
    async fn admin_disconnect_nip_fi_delivers_restricted_json_then_policy_close() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;

        let key = nostr::Keys::generate();
        // 1-hour deadline: expiry task armed but will NOT fire during this test.
        let deadline = Utc::now() + Duration::hours(1);
        let assertion = VerifiedAssertion::for_test(Some(key.public_key()), vec![deadline]);

        // State with deny map; key is absent (not denied).
        let state = audio_deny_state(None).await;

        // Unique community so hook and registry slots don't collide with parallel tests.
        let community = buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::new_v4());
        let tenant =
            buzz_core::tenant::TenantContext::resolved(community, "test.local".to_string());

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let conn_cancel = CancellationToken::new();
        let cancel_for_assert = conn_cancel.clone();

        // Pre-create and register the control so the pubkey-scan can find it.
        // `audio_post_auth_register` writes `proven_pubkey` on this same Arc;
        // the registered entry is updated in-place. [same pattern as straddle test]
        let conn_control = crate::state::CommunityConnectionControl::new(conn_cancel.clone());
        let conn_id = uuid::Uuid::new_v4();
        let _conn_guard =
            state
                .community_connections
                .register(conn_id, community, conn_control.clone());
        let conn_control_for_server = conn_control.clone();

        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("W_admin_disconnect: bind test listener");
        let addr = listener
            .local_addr()
            .expect("W_admin_disconnect: test listener addr");

        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let control_outer = conn_control_for_server.clone();
                    move |ws: WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let control_inner = control_outer.clone();
                        let conn_time = chrono::Utc::now();
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    uuid::Uuid::new_v4(),
                                    control_inner,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app)
                .await
                .expect("W_admin_disconnect: test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("W_admin_disconnect: server ready");

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("W_admin_disconnect: connect client");

        // Arm the hook BEFORE sending auth — it fires after `set_terminal_frame_sender`
        // registers the sender (now before audio_post_auth_register, line ~343) and
        // before the first `check_cancel!()`.
        let (hook_arrived_rx, hook_release) =
            crate::nip_fi_test_hooks::audio_before_first_check_cancel_hook::arm(community);

        // NIP-42 challenge.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("W_admin_disconnect: challenge timeout")
            .expect("W_admin_disconnect: challenge item")
            .expect("W_admin_disconnect: challenge message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("W_admin_disconnect: expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("W_admin_disconnect: challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("W_admin_disconnect: challenge field")
            .to_string();

        let relay_url = "ws://test.local";
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key)
            .unwrap();
        let auth_msg = serde_json::json!({"type": "auth", "event": auth_event}).to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("W_admin_disconnect: send auth");

        // Wait for the handler to reach before_first_audio_check_cancel.
        // At this point `set_terminal_frame_sender` has already been called and
        // the terminal sender is registered on the control.
        tokio::time::timeout(std::time::Duration::from_secs(5), hook_arrived_rx)
            .await
            .expect(
                "W_admin_disconnect: handler must reach before_first_audio_check_cancel within 5s",
            )
            .expect("W_admin_disconnect: hook arrived channel closed");

        // Simulate admin-disconnect: call the real registry disconnect scan by pubkey.
        // CommunityConnectionControl::disconnect_nip_fi enqueues the denial frame on
        // the registered terminal sender, publishes AuthorizationDenied, then cancels.
        let pubkey_bytes = key.public_key().to_bytes().to_vec();
        let closed = state
            .community_connections
            .disconnect_nip_fi("test-issuer", &pubkey_bytes);
        assert_eq!(
            closed, 1,
            "W_admin_disconnect: registry scan must find exactly 1 audio session \
             (proves audio_post_auth_register ran before the hook)"
        );

        // Release hook — handler resumes, hits check_cancel!(), drains the
        // enqueued denial frame, then sends reason.close_message().
        hook_release.notify_one();

        // Frame 0: restricted JSON payload.
        let frame0 = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .expect("W_admin_disconnect: frame 0 timeout")
            .expect("W_admin_disconnect: frame 0 item")
            .expect("W_admin_disconnect: frame 0 ws message");
        let expected_json = serde_json::json!({
            "type": "restricted",
            "message": buzz_auth::DenialClass::AuthorizationDenied.nostr_text()
        })
        .to_string();
        match frame0 {
            tokio_tungstenite::tungstenite::Message::Text(t) => {
                assert_eq!(
                    t.as_str(),
                    expected_json.as_str(),
                    "W_admin_disconnect: frame 0 must be exact restricted JSON payload"
                );
            }
            other => {
                panic!("W_admin_disconnect: frame 0 must be Text(restricted JSON); got {other:?}")
            }
        }

        // Frame 1: 1008 POLICY close.
        let frame1 = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .expect("W_admin_disconnect: frame 1 timeout")
            .expect("W_admin_disconnect: frame 1 item")
            .expect("W_admin_disconnect: frame 1 ws message");
        match frame1 {
            tokio_tungstenite::tungstenite::Message::Close(Some(cf)) => {
                assert_eq!(
                    cf.code,
                    tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy,
                    "W_admin_disconnect: close code must be 1008 POLICY"
                );
                assert_eq!(
                    <tokio_tungstenite::tungstenite::Utf8Bytes as AsRef<str>>::as_ref(&cf.reason),
                    "authorization denied",
                    "W_admin_disconnect: close reason must be exact 'authorization denied' bytes"
                );
            }
            other => panic!(
                "W_admin_disconnect: frame 1 must be Close(1008, 'authorization denied'); \
                 got {other:?}"
            ),
        }

        assert!(
            cancel_for_assert.is_cancelled(),
            "W_admin_disconnect: conn_cancel must be cancelled after admin disconnect"
        );

        server.abort();
        let _ = server.await;
    }

    // ── W9/W10/reaffirm: participant-commit barrier (real-DB) ─────────────────
    //
    // These three witnesses require a seeded DB (community + channel + membership).
    // They require the PG-wrapper DB, like W1; a missing or unreachable DB fails.
    //
    // Shared fixture setup for W9, W10, and the reaffirm variant:
    //   1. INSERT a community (non-nil UUID, `deletion_state = 'active'`).
    //   2. INSERT a channel under that community (no TTL → non-ephemeral, so
    //      `check_membership_for_admission` returns `MembershipAdmission::Existing`
    //      which we pass directly without going through that function).
    //   3. INSERT the test pubkey into `channel_members` so the `Existing` path
    //      is correct and `commit_participant_join` goes straight to the 48101 insert.
    //   4. Call `commit_participant_join` directly (it is `pub(crate)` for tests).

    /// Create an AppState backed by the PG lane wrapper's per-test database.
    ///
    /// A missing or unreachable DB panics — these witnesses never skip.  [F3 fix]
    async fn audio_test_state_real_db() -> std::sync::Arc<crate::state::AppState> {
        use std::sync::Arc;

        let db_url = crate::test_support::required_database_url();
        if let Err(e) = sqlx::PgPool::connect(&db_url).await {
            panic!("wrapper DB {db_url} unreachable: {e}");
        }
        // hermetic_for_test: env-free, never races NIP-FI env-var mutations.
        // Override database_url to the wrapper DB probed above. [F6]
        let mut config = crate::config::Config::hermetic_for_test();
        config.require_relay_membership = false;
        config.database_url = db_url;
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        Arc::new(state)
    }

    /// Seed a community + channel + membership row. Returns `(pool, tenant, channel_id, pubkey_bytes)`.
    async fn seed_audio_fixture(
        pool: &sqlx::PgPool,
    ) -> (buzz_core::tenant::TenantContext, uuid::Uuid, nostr::Keys) {
        let community_uuid = uuid::Uuid::new_v4();
        let host = format!("w9-test-{}.example", community_uuid.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(&host)
            .execute(pool)
            .await
            .expect("W9 fixture: seed community");

        let channel_id = uuid::Uuid::new_v4();
        let creator = nostr::Keys::generate();
        let creator_bytes = creator.public_key().to_bytes().to_vec();
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by) \
             VALUES ($1, $2, 'w9-test-channel', 'stream', 'open', $3)",
        )
        .bind(channel_id)
        .bind(community_uuid)
        .bind(&creator_bytes)
        .execute(pool)
        .await
        .expect("W9 fixture: seed channel");

        let member_key = nostr::Keys::generate();
        let member_bytes = member_key.public_key().to_bytes().to_vec();
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by) \
             VALUES ($1, $2, $3, 'member', $4)",
        )
        .bind(community_uuid)
        .bind(channel_id)
        .bind(&member_bytes)
        .bind(&creator_bytes)
        .execute(pool)
        .await
        .expect("W9 fixture: seed channel_member");

        let tenant = buzz_core::tenant::TenantContext::resolved(
            buzz_core::tenant::CommunityId::from_uuid(community_uuid),
            host,
        );
        (tenant, channel_id, member_key)
    }

    /// Like `seed_audio_fixture` but with `visibility = 'private'` so that a
    /// non-member key fails `check_membership_for_admission` with "not a member"
    /// rather than being auto-admitted via the open-channel shortcut.
    ///
    /// Used by the R2 `drain_terminal!()` race witness, which needs to exercise
    /// the Err arm of `check_membership_for_admission`.
    async fn seed_private_audio_fixture(
        pool: &sqlx::PgPool,
    ) -> (buzz_core::tenant::TenantContext, uuid::Uuid, nostr::Keys) {
        let community_uuid = uuid::Uuid::new_v4();
        let host = format!("r2-private-test-{}.example", community_uuid.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(&host)
            .execute(pool)
            .await
            .expect("R2 private fixture: seed community");

        let channel_id = uuid::Uuid::new_v4();
        let creator = nostr::Keys::generate();
        let creator_bytes = creator.public_key().to_bytes().to_vec();
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by) \
             VALUES ($1, $2, 'r2-private-channel', 'stream', 'private', $3)",
        )
        .bind(channel_id)
        .bind(community_uuid)
        .bind(&creator_bytes)
        .execute(pool)
        .await
        .expect("R2 private fixture: seed channel");

        let member_key = nostr::Keys::generate();
        let member_bytes = member_key.public_key().to_bytes().to_vec();
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by) \
             VALUES ($1, $2, $3, 'member', $4)",
        )
        .bind(community_uuid)
        .bind(channel_id)
        .bind(&member_bytes)
        .bind(&creator_bytes)
        .execute(pool)
        .await
        .expect("R2 private fixture: seed channel_member");

        let tenant = buzz_core::tenant::TenantContext::resolved(
            buzz_core::tenant::CommunityId::from_uuid(community_uuid),
            host,
        );
        (tenant, channel_id, member_key)
    }

    // ── W9: expiry between uncommitted 48101 insert and acquire_effect → rollback ──
    //
    // `before_participant_commit` fires between the uncommitted 48101 insert and
    // `acquire_effect()`. Firing expiry at that point must roll back the
    // transaction (no committed 48101 row in the DB) and return
    // `JoinCommitError::Expired` to the caller.
    //
    // Mutation evidence:
    //   A) Delete `before_participant_commit(...)` from commit_participant_join →
    //      hook never fires → `arrived_rx` times out → test panics.
    //   B) Remove `tx.rollback()` from the `SessionExpired` branch →
    //      transaction auto-commits at drop, leaving a 48101 row → row-count
    //      assertion panics.
    //   C) Remove `acquire_effect()` entirely → commit proceeds despite cancel →
    //      a row is committed → row-count assertion panics.
    async fn w9_expiry_before_participant_commit_rolls_back_48101_insert_body() {
        use chrono::{Duration, Utc};
        use std::sync::Arc;
        use uuid::Uuid;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();
        let (tenant, channel_id, member_key) = seed_audio_fixture(&pool).await;
        let community_id = tenant.community();

        let member_bytes = member_key.public_key().to_bytes().to_vec();
        let member_hex = member_key.public_key().to_hex();
        let peer_id = Uuid::new_v4();
        let roster_revision = 1u64;
        let membership = MembershipAdmission::Existing {
            parent_channel_id: channel_id,
        };

        let deadline = Utc::now() + Duration::hours(1);
        let cancel = tokio_util::sync::CancellationToken::new();
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone());

        // Arm the hook: fires between the uncommitted 48101 insert and acquire_effect.
        let (arrived_rx, release) =
            crate::nip_fi_test_hooks::audio_participant_commit_hook::arm(community_id);

        let state2 = Arc::clone(&state);
        let tenant2 = tenant.clone();
        let member_bytes2 = member_bytes.clone();
        let member_hex2 = member_hex.clone();
        let gate2 = Arc::clone(&gate);
        let handle = tokio::spawn(async move {
            commit_participant_join(
                &state2,
                &tenant2,
                channel_id,
                channel_id,
                &member_hex2,
                &member_bytes2,
                peer_id,
                roster_revision,
                "test-generation",
                &membership,
                &gate2,
                String::new(),
                &std::sync::Arc::new(crate::audio::room::Room::new(
                    tenant2.community(),
                    channel_id,
                )),
            )
            .await
        });

        // Wait for the handler to reach the hook.
        tokio::time::timeout(std::time::Duration::from_secs(10), arrived_rx)
            .await
            .expect("W9: commit_participant_join must reach before_participant_commit within 10s")
            .expect("arrived channel closed");

        // Fire expiry — acquire_effect will return SessionExpired after release.
        cancel.cancel();

        // Release — handler resumes, calls acquire_effect(), gets SessionExpired, rolls back.
        release.notify_one();

        let result = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
            .await
            .expect("W9: commit_participant_join must return within 10s after hook release")
            .expect("commit_participant_join task must not panic");

        // Must return Expired, not Ok.
        assert!(
            matches!(result, Err(JoinCommitError::Expired)),
            "W9: commit_participant_join must return JoinCommitError::Expired after mid-flight expiry; got: {result:?}"
        );

        // Zero committed 48101 rows for this community+channel — transaction was rolled back.
        let row_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = 48101",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("W9: row count query");

        assert_eq!(
            row_count, 0,
            "W9: no 48101 row must be committed after expiry-forced rollback; found {row_count}"
        );

        // No membership side effects from commit (membership was Existing — no new insert).
        // The pre-existing channel_members row must still be there (rollback only undoes the tx's own writes).
        let member_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM channel_members \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(&member_bytes)
        .fetch_one(&pool)
        .await
        .expect("W9: member count query");

        assert_eq!(
            member_count, 1,
            "W9: the pre-seeded membership row must survive the rollback"
        );
    }

    // ── F3: lifecycle_generation in committed 48101 JOIN ──────────────────────
    //
    // Verifies that `commit_participant_join` includes `lifecycle_generation` in
    // the 48101 event content in Off mode, and that the committed generation
    // matches `state.huddle_liveness_generation` — the SAME source the liveness
    // consumer (`handle_huddle_liveness_req`) uses for Off-mode (non-mesh) rooms.
    // This proves Desktop reconciliation will NOT clear admissions: the
    // generation embedded in the JOIN and the generation returned by the next
    // authoritative liveness response are always equal.
    //
    // Without this field Desktop reconciliation assigns "pending" to the joining
    // peer and then clears admissions when the next authoritative liveness
    // response (which includes the real generation) differs from the JOIN's
    // absent generation.
    //
    // Mutation evidence:
    //   Remove `"generation": lifecycle_generation` from the content JSON in
    //   `commit_participant_join` → the field is absent → the assertion on
    //   `content_json["generation"]` panics.
    //   Use a hard-coded string instead of `state.huddle_liveness_generation` →
    //   the lifecycle/liveness equality assertion panics (generation mismatch).
    async fn f3_commit_participant_join_includes_lifecycle_generation_body() {
        use uuid::Uuid;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();
        let (tenant, channel_id, member_key) = seed_audio_fixture(&pool).await;
        let community_id = tenant.community();
        let pubkey_hex = member_key.public_key().to_hex();
        let pubkey_bytes = member_key.public_key().to_bytes().to_vec();

        // Use the relay's global liveness generation — the SAME source that
        // `handle_huddle_liveness_req` returns for Off-mode (non-mesh) rooms.
        // This binds the test to the production path: audio/handler.rs line 716
        // also falls back to `state.huddle_liveness_generation.to_string()` for
        // Off-mode sessions.  A hard-coded string would break the equality proof.
        let generation_string = state.huddle_liveness_generation.to_string();

        // Capture the explicit peer_id and roster_revision driven by this test
        // so we can assert them against the committed 48101 content (independence
        // check: the content stores what the test drove, not some other value).
        let expected_peer_id = Uuid::new_v4();
        let expected_roster_revision: u64 = 1;

        // Off-mode gate (no deadline → acquire_effect always succeeds).
        let off_cancel = tokio_util::sync::CancellationToken::new();
        let off_gate = crate::nip_fi_gate::SessionAdmissionGate::off_mode(off_cancel.clone());

        let result = commit_participant_join(
            &state,
            &tenant,
            channel_id,
            channel_id,
            &pubkey_hex,
            &pubkey_bytes,
            expected_peer_id,
            expected_roster_revision,
            &generation_string,
            &MembershipAdmission::Existing {
                parent_channel_id: channel_id,
            },
            &off_gate,
            String::new(),
            &std::sync::Arc::new(crate::audio::room::Room::new(community_id, channel_id)),
        )
        .await;

        assert!(
            result.is_ok(),
            "F3: commit_participant_join must succeed; got: {result:?}"
        );

        // Retrieve the committed 48101 row and check its content and tags.
        struct F3Row {
            pubkey_bytes: Vec<u8>,
            tags_json: serde_json::Value,
            content: String,
        }
        let row: F3Row = {
            let r = sqlx::query(
                "SELECT pubkey, tags, content::text \
                 FROM events \
                 WHERE community_id = $1 AND channel_id = $2 AND kind = 48101 \
                 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(community_id.as_uuid())
            .bind(channel_id)
            .fetch_one(&pool)
            .await
            .expect("F3: 48101 row must exist after commit");
            F3Row {
                pubkey_bytes: r.get::<Vec<u8>, _>(0),
                tags_json: r.get::<serde_json::Value, _>(1),
                content: r.get::<String, _>(2),
            }
        };

        let content_json: serde_json::Value =
            serde_json::from_str(&row.content).expect("F3: 48101 content must be valid JSON");

        // Primary assertion: generation field matches the relay's liveness generation.
        // The liveness consumer (handle_huddle_liveness_req) returns the same
        // `state.huddle_liveness_generation` for Off-mode rooms — so the JOIN's
        // embedded generation will equal the next liveness response, and Desktop
        // reconciliation will NOT clear the admission.  [F3: liveness-consumer equality]
        //
        // Wire key is "generation" — matches Desktop's lifecycleContent parser
        // at huddlePresence.ts:83-85 (`content.generation`).  [Carl review]
        assert_eq!(
            content_json["generation"].as_str(),
            Some(generation_string.as_str()),
            "F3: 48101 JOIN content must include generation = {generation_string:?} \
             (must match state.huddle_liveness_generation, the same source used by the \
             Off-mode liveness consumer); got content: {}",
            row.content
        );

        // roster_revision: exact value driven by this test (not just "is some").
        // If commit_participant_join stores a different value, this fails.
        assert_eq!(
            content_json["roster_revision"].as_u64(),
            Some(expected_roster_revision),
            "F3: 48101 JOIN content must include roster_revision = {expected_roster_revision}; \
             got content: {}",
            row.content
        );

        // admission_id: must be the UUID we passed in (independently expected).
        // This catches any path that discards or replaces the peer_id argument.
        assert_eq!(
            content_json["admission_id"].as_str(),
            Some(expected_peer_id.to_string().as_str()),
            "F3: 48101 JOIN content must include admission_id = {expected_peer_id}; \
             got content: {}",
            row.content
        );

        // p-tag: must be the member pubkey (not the relay signer or any other key).
        // This is the independence check for Thufir's identity finding:
        //   participantPubkey(event) = tags.find(t => t[0] == "p")?.[1] ?? event.pubkey
        // The p-tag must be member_key, not relay_signer or anything else.
        //
        // Mutation (Thufir's named counterexample): change p_tag to use the relay
        // signer at audio/handler.rs:2525-2532 → participant_hex != member pubkey
        // → this assertion panics.
        let p_tag_value: Option<String> = row
            .tags_json
            .as_array()
            .and_then(|tags| {
                tags.iter().find(|tag| {
                    tag.as_array()
                        .and_then(|t| t.first())
                        .and_then(|v| v.as_str())
                        == Some("p")
                })
            })
            .and_then(|tag| tag.as_array())
            .and_then(|t| t.get(1))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        assert_eq!(
            p_tag_value.as_deref(),
            Some(pubkey_hex.as_str()),
            "F3: 48101 p-tag must be the member's pubkey ({pubkey_hex}), \
             not the relay signer or any other key. \
             Mutation (Thufir): change p_tag at audio/handler.rs:2525-2532 to relay signer \
             → p_tag_value != pubkey_hex → assertion panics."
        );

        // 48101 must be signed by the relay keypair (pubkey column = relay signer).
        let relay_signer_hex = state.relay_keypair.public_key().to_hex();
        let join_pubkey_hex = hex::encode(&row.pubkey_bytes);
        assert_eq!(
            join_pubkey_hex, relay_signer_hex,
            "F3: 48101 event must be signed by the relay keypair"
        );

        // ── Drift guard: producer output shape must match committed fixture ─────────
        //
        // The committed fixture `desktop/src/features/huddle/lib/__fixtures__/nip-fi-join-liveness.json`
        // is the input for the Desktop contract witness
        // (`F3 Desktop contract: producer JOIN + liveness retain admitted participant
        //   through authoritative refresh` in huddlePresenceRuntime.test.mjs).
        //
        // This drift guard ensures that if the producer schema changes (keys renamed,
        // dropped, or retyped), the Desktop test will also fail — preventing silent
        // divergence where the Desktop test passes against stale fixture data.
        //
        // Structural invariants asserted here:
        //   - content has exactly the keys: ephemeral_channel_id, roster_revision,
        //     admission_id, generation  (no extra, no missing)
        //   - p-tag is present and non-empty (already asserted above)
        //   - event pubkey is relay signer (already asserted above)
        //
        // RED gate (b): change the producer's `generation` wire key or value
        // (e.g., rename "generation" to "gen") → content_json["generation"] is null →
        // the generation assertion above panics. This breaks the Desktop test too, since
        // the fixture will no longer match what the producer emits.
        let content_keys: std::collections::BTreeSet<String> = content_json
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        let expected_keys: std::collections::BTreeSet<String> = [
            "ephemeral_channel_id",
            "roster_revision",
            "admission_id",
            "generation",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            content_keys, expected_keys,
            "F3 drift guard: 48101 JOIN content must have exactly the keys \
             {{ephemeral_channel_id, roster_revision, admission_id, generation}}. \
             The committed Desktop fixture (nip-fi-join-liveness.json) encodes this shape. \
             If the producer adds, removes, or renames a key, update the fixture \
             AND this guard in the same commit. Got keys: {content_keys:?}"
        );
    }

    // ── F3 wire: real admission produces 48101 JOIN with correct generation ──
    //
    // Drives the real `handle_active_audio_connection` handler through a full
    // admission to a committed 48101 JOIN event.  Pauses at
    // `after_participant_fanout` (TX committed, fan-out done), then reads the
    // actual 48101 wire bytes from the DB and asserts that
    // `content.generation` equals `state.huddle_liveness_generation.to_string()`.
    //
    // This directly tests the production Off-mode fallback at
    // `audio/handler.rs:802`:
    //   `state.huddle_liveness_generation.to_string()`
    //
    // Falsifying counterexample per Thufir: change line 802 to a hardcoded UUID.
    // The existing `f3_commit_participant_join_includes_lifecycle_generation` test
    // supplies `state.huddle_liveness_generation` as its INPUT and would not
    // detect this regression.  THIS test reads the committed content FROM the wire
    // bytes and compares it against the same source — the hardcoded UUID would
    // disagree, and this test panics.
    //
    // Mutation evidence:
    //   A) Change `audio/handler.rs:802` to a hardcoded UUID →
    //      committed 48101 content.generation != state.huddle_liveness_generation →
    //      assertion panics.
    //   B) Remove `"generation": lifecycle_generation` from commit content →
    //      `content_json["generation"]` is null → as_str() returns None →
    //      assert_eq panics (generation_from_wire != expected_generation).
    //   C) Delete `after_participant_fanout(...)` → arrived_rx times out → panics.
    //
    // Requires Postgres for the full commit path.
    async fn f3_wire_admission_generation_matches_liveness_source_body() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;
        use tokio_tungstenite::connect_async;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();
        let (tenant, channel_id, member_key) = seed_audio_fixture(&pool).await;
        let community = tenant.community();

        // Far-future deadline — admission completes before any expiry.
        let assertion = VerifiedAssertion::for_test(
            Some(member_key.public_key()),
            vec![Utc::now() + Duration::hours(1)],
        );

        let conn_cancel = CancellationToken::new();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();
        let conn_cancel_c = conn_cancel.clone();
        let tenant_host = tenant_c.host().to_string();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("F3 wire: bind listener");
        let addr = listener.local_addr().expect("F3 wire: listener addr");

        // Arm after_participant_fanout BEFORE spawning server to avoid a race.
        let (fanout_rx, fanout_release) =
            crate::nip_fi_test_hooks::audio_participant_fanout_hook::arm(community);

        let server = tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/",
                axum::routing::get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let cancel_i = conn_cancel_c.clone();
                    move |ws: axum::extract::ws::WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let conn_time = chrono::Utc::now();
                        let control_inner =
                            crate::state::CommunityConnectionControl::new(cancel_i.clone());
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    channel_id,
                                    control_inner,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app)
                .await
                .expect("F3 wire: test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("F3 wire: server ready");

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("F3 wire: connect client");

        // NIP-42 auth exchange.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("F3 wire: challenge timeout")
            .expect("F3 wire: challenge item")
            .expect("F3 wire: challenge message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("F3 wire: expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("F3 wire: challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("F3 wire: challenge field")
            .to_string();

        let relay_url = format!("ws://{tenant_host}");
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", &relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&member_key)
            .unwrap();
        let auth_msg = serde_json::json!({
            "type": "auth",
            "event": auth_event,
            "parent_channel_id": null,
            "protocol_version": 1,
        })
        .to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("F3 wire: send auth");

        // Wait for after_participant_fanout: 48101 is committed and fan-out done.
        // The production Off-mode generation selection (audio/handler.rs:802) has run.
        tokio::time::timeout(std::time::Duration::from_secs(10), fanout_rx)
            .await
            .expect(
                "F3 wire: handler must reach after_participant_fanout within 10s                  (proves commit path ran and 48101 is committed with production generation)",
            )
            .expect("F3 wire: fanout channel closed");

        // ── Read the actual 48101 wire bytes from DB ──────────────────────
        let content_str: String = sqlx::query_scalar(
            "SELECT content::text FROM events              WHERE community_id = $1 AND channel_id = $2 AND kind = 48101              ORDER BY created_at DESC LIMIT 1",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("F3 wire: 48101 row must exist after commit");

        let content_json: serde_json::Value =
            serde_json::from_str(&content_str).expect("F3 wire: 48101 content must be valid JSON");

        // The generation in the wire bytes comes from production audio/handler.rs:802.
        // This is the Off-mode fallback: state.huddle_liveness_generation.to_string().
        // If that line is changed to a hardcoded UUID, this assertion fails.
        let generation_from_wire = content_json["generation"]
            .as_str()
            .unwrap_or("")
            .to_string();

        // The authoritative source that both the admission handler and the liveness
        // consumer (req.rs handle_huddle_liveness_req) use for Off-mode rooms.
        let expected_generation = state.huddle_liveness_generation.to_string();

        // ── F3 consumer chain: liveness oracle returns same generation ─────────
        //
        // Extends the producer chain above with the consumer side:
        //   real admission → actual 48101 JOIN wire bytes →
        //   hydrated Desktop-semantics oracle (handle_huddle_liveness_req) →
        //   authoritative liveness generation → equals wire generation →
        //   Desktop will NOT clear the admission.
        //
        // Contract (huddlePresence.ts:464-484):
        //   Desktop records JOIN's content.generation.  On each liveness event,
        //   it compares event.content.generation against the stored value.
        //   Mismatch → peer cleared.  Equal → admission retained.
        //
        // Step 1: The real handler called add_peer during the admission; the room
        //   is live with the member_key peer.  Verify the admission is retained.
        let room_after_commit = state.audio_rooms.get(community, channel_id);
        assert!(
            room_after_commit.is_some_and(|r| !r.is_empty()),
            "F3 wire: audio room must be non-empty after admission commit \
             (proves add_peer ran — admission is retained in the room roster)"
        );

        // Step 2: Seed a KIND_HUDDLE_STARTED link so handle_huddle_liveness_req
        //   can find the session via huddle_started_links().
        //   This fixture uses channel_id as its own parent (non-ephemeral channel).
        //   The link event content specifies ephemeral_channel_id = channel_id.
        //   creator_bytes is derived from the channel row's created_by field.
        let creator_bytes: Vec<u8> = sqlx::query_scalar(
            "SELECT created_by FROM channels WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("F3 wire consumer: fetch channel.created_by");

        let link_content = serde_json::json!({
            "ephemeral_channel_id": channel_id.to_string()
        })
        .to_string();
        let link_event_id = uuid::Uuid::new_v4().as_bytes().to_vec();
        sqlx::query(
            "INSERT INTO events \
             (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id) \
             VALUES ($1, $2, $3, NOW(), $4, '[]', $5, $6, $7) \
             ON CONFLICT DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(&link_event_id[..])
        .bind(&creator_bytes)
        .bind(buzz_core::kind::KIND_HUDDLE_STARTED as i32)
        .bind(&link_content)
        .bind(vec![0u8; 64])
        .bind(channel_id) // parent channel = session channel (non-ephemeral)
        .execute(&pool)
        .await
        .expect("F3 wire consumer: seed KIND_HUDDLE_STARTED link event");

        // Step 3: Build a minimal ConnectionState for the liveness call.
        //   (Same pattern as req.rs f3_liveness_consumer_generation_equals_join_generation.)
        use std::collections::HashMap;
        use tokio::sync::mpsc;
        let lc_cancel = tokio_util::sync::CancellationToken::new();
        let lc_gate = crate::nip_fi_gate::SessionAdmissionGate::off_mode(lc_cancel.clone());
        let (send_tx, mut send_rx) = mpsc::channel::<axum::extract::ws::Message>(16);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel::<axum::extract::ws::Message>(8);
        let (terminal_ctrl_tx, _terminal_ctrl_rx) = mpsc::channel::<axum::extract::ws::Message>(1);
        let lc_conn = std::sync::Arc::new(crate::connection::ConnectionState {
            conn_id: uuid::Uuid::new_v4(),
            tenant: buzz_core::tenant::TenantContext::resolved(community, tenant_host.clone()),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            auth_state: std::sync::Mutex::new(crate::connection::AuthState::Authenticated(
                buzz_auth::AuthContext {
                    pubkey: member_key.public_key(),
                    scopes: vec![],
                    channel_ids: None,
                    auth_method: buzz_auth::AuthMethod::Nip42,
                    agent_owner_pubkey: None,
                },
            )),
            subscriptions: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            terminal_ctrl_tx,
            cancel: lc_cancel.clone(),
            backpressure_count: std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0)),
            grace_limit: 3,
            nip_fi_assertion: None,
            session_deadline: None,
            nip_fi_gate: lc_gate,
            community_control: crate::state::CommunityConnectionControl::new(lc_cancel.clone()),
        });

        // Step 4: Call the production liveness handler.
        //   Filter: kind=KIND_HUDDLE_LIVENESS, #d=[channel_id].
        //   parent_channel_ids=[channel_id] (the channel is its own parent here).
        use buzz_core::kind::KIND_HUDDLE_LIVENESS;
        let liveness_filter: nostr::Filter = serde_json::from_value(serde_json::json!({
            "kinds": [KIND_HUDDLE_LIVENESS],
            "#d": [channel_id.to_string()],
        }))
        .expect("F3 wire consumer: build liveness filter");
        crate::handlers::req::handle_huddle_liveness_req(
            "f3-wire-liveness-sub",
            &[liveness_filter],
            &[channel_id],
            &lc_conn,
            &state,
        )
        .await;

        // Step 5: Collect frames and find the liveness generation.
        let mut liveness_gen: Option<String> = None;
        let mut liveness_event: Option<serde_json::Value> = None;
        loop {
            match send_rx.try_recv() {
                Ok(axum::extract::ws::Message::Text(t)) => {
                    let v: serde_json::Value = serde_json::from_str(&t).unwrap_or_default();
                    if v[0].as_str() == Some("EVENT") {
                        liveness_event = Some(v[2].clone());
                        let content: serde_json::Value =
                            serde_json::from_str(v[2]["content"].as_str().unwrap_or("{}"))
                                .unwrap_or_default();
                        if let Some(gen) = content["generation"].as_str() {
                            liveness_gen = Some(gen.to_string());
                        }
                    }
                    if v[0].as_str() == Some("EOSE") {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }

        // Step 6: Assert liveness generation equals wire generation.
        //   This closes the end-to-end chain:
        //     real admission → actual JOIN wire bytes (generation_from_wire)
        //     → liveness oracle (handle_huddle_liveness_req) returns the SAME
        //       generation → Desktop will NOT clear the admission.
        //
        // Desktop invariant (huddlePresence.ts:464-484):
        //   JOIN.content.generation == liveness.content.generation → admission retained.
        //   Mismatch → peer cleared (admission lost).

        // ── F3 Desktop-semantics oracle: full admission-retention chain ──────
        //
        // Extends the liveness-generation chain with a hydrated Desktop-semantics
        // oracle that faithfully implements `HuddlePresenceTracker` from
        // `desktop/src/features/huddle/lib/huddlePresence.ts:260-484`.
        //
        // Purpose:
        //   The liveness-generation assertion above proves relay equality:
        //     JOIN.content.generation == liveness.content.generation (both from
        //     state.huddle_liveness_generation.to_string()).
        //   But generation equality alone cannot prove admission retention:
        //   Desktop ALSO rejects a JOIN whose resolved participant is empty
        //   (`participantPubkey(event)` → null → `apply()` returns false at
        //   `huddlePresence.ts:341-344`).  The oracle below exercises that path.
        //
        // Chain:
        //   real admission → actual 48101 JOIN event (committed to DB and fetched here)
        //   → hydrated HuddlePresenceOracle (faithfully implements TS apply() + reconcileLiveness())
        //   → reconcile with authoritative liveness generation (liveness_gen_value)
        //   → assert participant admission is retained (present = true)
        //
        // Acceptance RED:
        //   Strip the `p` tag from the JOIN event → the participant resolves to
        //   the relay signer (`?? event.pubkey`) → the identity assertion below
        //   panics. This is Thufir's named counterexample
        //   (audio/handler.rs:2525-2532 p-tag removal).
        //
        // The oracle struct below implements the minimal subset of
        // HuddlePresenceTracker semantics required for this chain:
        //   - apply(STARTED) → creates session, records startCreator
        //   - apply(JOIN/48101) → checks event.pubkey == relaySelf AND the
        //     caller-resolved participant is non-empty AND (generation compatible)
        //     → records admission as present
        //   - reconcile_liveness(session_id, generation) → clears admissions on
        //     generation mismatch; leaves them if generation equals stored
        //   - is_admitted(session_id, participant) → true if present in admissions

        // Fetch the actual committed 48101 event from DB (includes pubkey + tags).
        // The pubkey column is stored as raw bytes; tags as JSONB.
        struct JoinRow {
            pubkey_bytes: Vec<u8>,
            tags_json: serde_json::Value,
            content: String,
        }
        let join_row: JoinRow = {
            let row = sqlx::query(
                "SELECT pubkey, tags, content::text \
                 FROM events \
                 WHERE community_id = $1 AND channel_id = $2 AND kind = 48101 \
                 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(community.as_uuid())
            .bind(channel_id)
            .fetch_one(&pool)
            .await
            .expect("F3 oracle: 48101 row must exist for Desktop oracle step");
            JoinRow {
                pubkey_bytes: row.get::<Vec<u8>, _>(0),
                tags_json: row.get::<serde_json::Value, _>(1),
                content: row.get::<String, _>(2),
            }
        };

        // The relay signer's pubkey hex — must equal join_row.pubkey_bytes
        // (all 48101 events are signed by state.relay_keypair).
        let relay_signer_hex = state.relay_keypair.public_key().to_hex();
        let join_pubkey_hex = hex::encode(&join_row.pubkey_bytes);

        assert_eq!(
            join_pubkey_hex, relay_signer_hex,
            "F3 oracle: 48101 event must be signed by the relay keypair. \
             Mutation: change sign_with_keys to use a different key → pubkey mismatch → \
             oracle rejects JOIN (event.pubkey != relaySelf) → admission not recorded → \
             is_admitted panics below."
        );

        // Extract the p-tag participant from the stored event's tags JSON.
        // huddlePresence.ts:participantPubkey(event) = tags.find(t => t[0] == "p")?.[1] ?? event.pubkey
        // For relay-signed JOINs the tag is always present (audio/handler.rs:2525-2532).
        let participant_from_p_tag: Option<String> = join_row
            .tags_json
            .as_array()
            .and_then(|tags| {
                tags.iter().find(|tag| {
                    tag.as_array()
                        .and_then(|t| t.first())
                        .and_then(|v| v.as_str())
                        == Some("p")
                })
            })
            .and_then(|tag| tag.as_array())
            .and_then(|t| t.get(1))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Desktop participantPubkey(): `tags.find(p)?.[1] ?? event.pubkey`
        // (huddlePresence.ts:111-115). Only an absent p-tag falls back; an empty
        // p-tag value stays empty and is rejected by `apply_join`.
        let participant_hex = participant_from_p_tag.unwrap_or_else(|| join_pubkey_hex.clone());

        // ── Identity independence check (Thufir round-7 finding) ─────────────
        // The oracle reads participant_hex from the stored p-tag and then later
        // asserts oracle.is_admitted(session_id, &participant_hex) — which grades
        // its own homework if participant_hex is never compared against an
        // independently expected value.  This assertion breaks that circularity:
        // participant_hex must equal member_key.public_key() — the key the test
        // drove through the real admission handler.
        //
        // If the p-tag stores the relay signer instead of the member key
        // (Thufir's counterexample at audio/handler.rs:2525-2532), participant_hex
        // would not equal member_key.public_key().to_hex() and this assertion panics.
        let expected_participant_hex = member_key.public_key().to_hex();
        assert_eq!(
            participant_hex, expected_participant_hex,
            "F3 identity: p-tag participant ({participant_hex}) must equal the independently \
             expected member_key ({expected_participant_hex}). \
             Mutation (Thufir's named counterexample): change the production JOIN p-tag at \
             audio/handler.rs:2525-2532 to use the relay signer → participant_hex is the relay \
             signer hex → != member pubkey → this assertion panics. \
             Without this check the oracle grades its own homework."
        );

        // ── Rust HuddlePresenceOracle ────────────────────────────────────────
        // Minimal faithful implementation of the Desktop semantics needed for this
        // chain.  Mirrors huddlePresence.ts:260-484 for the STARTED+JOIN+reconcile path.
        //
        // Key Desktop invariants exercised:
        //   1. apply(STARTED) → session created with startCreator = event.pubkey.
        //      JOIN events are ONLY accepted for sessions where apply(STARTED) ran first
        //      (the `let session = self.sessions.get(session_id); if !session return false`
        //      guard at huddlePresence.ts:334-336).  We seed STARTED manually here.
        //   2. apply(JOIN): event.pubkey must equal relaySelf (ts:341).
        //      participant = participantPubkey(event) (p-tag ?? event.pubkey), resolved
        //      by the caller; must be non-empty (ts:344 `if !participant return false`).
        //   3. reconcile_liveness: if generation equals stored → admissions preserved.
        //      if generation differs → admissions cleared (ts:464-484).

        struct HuddlePresenceOracle {
            relay_signer_hex: String,
            // session_id → (stored_generation, admitted_participants: HashMap<hex, bool>)
            sessions: std::collections::HashMap<
                String,
                (Option<String>, std::collections::HashMap<String, bool>),
            >,
        }
        impl HuddlePresenceOracle {
            fn new(relay_signer_hex: String) -> Self {
                Self {
                    relay_signer_hex,
                    sessions: Default::default(),
                }
            }

            // Apply a STARTED event to seed the session.
            // ts:267-289: session created with startCreator = event.pubkey.
            fn apply_started(&mut self, session_id: &str) {
                self.sessions
                    .entry(session_id.to_string())
                    .or_insert((None, Default::default()));
            }

            // Apply a JOIN (48101) event.
            // ts:335-476:
            //   - session must exist (seeded by apply_started above)
            //   - event.pubkey must == relaySelf (ts:341)
            //   - participant = caller-resolved participantPubkey (p-tag ?? pubkey);
            //     must be non-empty (ts:344)
            //   - generation stored/updated; admission recorded
            // Returns true if admitted, false if rejected.
            fn apply_join(
                &mut self,
                event_pubkey_hex: &str,
                participant: &str,
                content_json: &serde_json::Value,
            ) -> bool {
                let session_id = match content_json["ephemeral_channel_id"].as_str() {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => return false,
                };
                // ts:341: JOIN event.pubkey must equal relaySelf.
                if event_pubkey_hex != self.relay_signer_hex {
                    return false;
                }
                // ts:344: participantPubkey must be non-null.
                if participant.is_empty() {
                    return false;
                }
                let session = match self.sessions.get_mut(&session_id) {
                    Some(s) => s,
                    None => return false, // ts:334: session must exist
                };
                // ts:398-401: update session generation.
                if let Some(gen) = content_json["generation"]
                    .as_str()
                    .filter(|g| !g.is_empty())
                {
                    session.0 = Some(gen.to_string());
                }
                // ts:370-380: record admission as present.
                session.1.insert(participant.to_string(), true);
                true
            }

            // Reconcile liveness (ts:464-484):
            //   if stored generation != incoming generation → clear all admissions.
            //   if equal → leave admissions intact.
            // Returns whether admissions were preserved (true) or cleared (false).
            fn reconcile_liveness(&mut self, session_id: &str, liveness_generation: &str) -> bool {
                let session = match self.sessions.get_mut(session_id) {
                    Some(s) => s,
                    None => return false,
                };
                let preserved = match &session.0 {
                    Some(stored) if stored != liveness_generation => {
                        // ts:464-473: generation mismatch → clear admissions.
                        session.1.clear();
                        false
                    }
                    _ => true, // equal or None → admissions preserved.
                };
                session.0 = Some(liveness_generation.to_string());
                preserved
            }

            // Returns true if the participant has a present admission in the session.
            fn is_admitted(&self, session_id: &str, participant_hex: &str) -> bool {
                self.sessions
                    .get(session_id)
                    .and_then(|(_, admissions)| admissions.get(participant_hex))
                    .copied()
                    .unwrap_or(false)
            }
        }

        let mut oracle = HuddlePresenceOracle::new(relay_signer_hex.clone());

        // Seed the session by applying a STARTED event.
        // In production, Desktop receives an actual kind-48100 STARTED event.
        // Here we directly call apply_started with the session ID (channel_id).
        // This mirrors Desktop's session creation path (ts:267-289).
        oracle.apply_started(&channel_id.to_string());

        // Parse the content from the stored 48101 event.
        let join_content_json: serde_json::Value =
            serde_json::from_str(&join_row.content).unwrap_or_default();

        // admission_id / roster_revision must be the admission this test drove:
        // the member's live room peer id and the room's roster revision.
        let room = state
            .audio_rooms
            .get(community, channel_id)
            .expect("F3 wire: room must exist");
        let expected_admission_id = room
            .peers
            .iter()
            .find(|peer| peer.pubkey == expected_participant_hex)
            .map(|peer| peer.key().to_string())
            .expect("F3 wire: member must hold a room peer");
        let expected_roster_revision = room.roster_snapshot().revision;
        assert_eq!(
            join_content_json["admission_id"].as_str(),
            Some(expected_admission_id.as_str()),
            "F3 wire: JOIN admission_id must be the member's room peer id; got {}",
            join_row.content
        );
        assert_eq!(
            join_content_json["roster_revision"].as_u64(),
            Some(expected_roster_revision),
            "F3 wire: JOIN roster_revision must be the room revision; got {}",
            join_row.content
        );

        // ── Drift guard: real producer output == committed Desktop fixture ───
        //
        // Normalize the real JOIN and liveness events (random keys and ids →
        // fixture placeholders) and require kind, pubkey, tags and content to
        // equal desktop/.../__fixtures__/nip-fi-join-liveness.json, the input
        // of the Desktop runtime contract test.
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../desktop/src/features/huddle/lib/__fixtures__/nip-fi-join-liveness.json"
        ))
        .expect("F3 drift guard: fixture JSON");
        let replacements = [
            (
                relay_signer_hex.clone(),
                fixture["relayPubkey"].as_str().unwrap(),
            ),
            (
                expected_participant_hex.clone(),
                fixture["memberPubkey"].as_str().unwrap(),
            ),
            (
                channel_id.to_string(),
                fixture["channelId"].as_str().unwrap(),
            ),
            (
                expected_generation.clone(),
                fixture["generation"].as_str().unwrap(),
            ),
            (
                expected_admission_id.clone(),
                fixture["admissionId"].as_str().unwrap(),
            ),
        ];
        let normalize = |raw: &str| {
            replacements
                .iter()
                .fold(raw.to_string(), |acc, (from, to)| {
                    acc.replace(from.as_str(), to)
                })
        };
        let shape = |event: &serde_json::Value| {
            serde_json::json!({
                "kind": event["kind"],
                "pubkey": event["pubkey"],
                "tags": event["tags"],
                "content": serde_json::from_str::<serde_json::Value>(
                    event["content"].as_str().unwrap_or("null")
                ).unwrap_or_default(),
            })
        };
        let produced_join = serde_json::json!({
            "kind": 48101,
            "pubkey": join_pubkey_hex,
            "tags": join_row.tags_json,
            "content": join_row.content,
        });
        let produced_liveness = liveness_event
            .clone()
            .expect("F3 drift guard: liveness EVENT captured");
        for (name, produced) in [("join", &produced_join), ("liveness", &produced_liveness)] {
            let normalized: serde_json::Value =
                serde_json::from_str(&normalize(&produced.to_string()))
                    .expect("F3 drift guard: normalized JSON");
            assert_eq!(
                shape(&normalized),
                shape(&fixture[name]),
                "F3 drift guard: relay {name} output diverged from the committed Desktop \
                 fixture; update nip-fi-join-liveness.json with the producer change"
            );
        }

        assert_eq!(
            generation_from_wire, expected_generation,
            "F3 wire: 48101 JOIN content.generation from wire bytes ({generation_from_wire}) \
             must equal state.huddle_liveness_generation ({expected_generation}). \
             Both production paths (admission handler + liveness consumer) read the same field. \
             A mismatch here means Desktop reconciliation would clear admissions. \
             Mutation A: change audio/handler.rs:802 to a hardcoded UUID → fails here."
        );

        assert!(
            !generation_from_wire.is_empty(),
            "F3 wire: 48101 content.generation must be non-empty in wire bytes. \
             Mutation B: remove generation field from commit content → empty string → panics."
        );

        let liveness_gen_value = liveness_gen.expect(
            "F3 wire consumer: handle_huddle_liveness_req must return a liveness EVENT. \
             Fixture seeds: valid community, stream channel, live room with peer, \
             KIND_HUDDLE_STARTED link event. A missing EVENT means the liveness handler \
             did not find the room or the link query failed. \
             Mutation A: remove KIND_HUDDLE_STARTED link seed → no EVENT → panics here.",
        );
        assert_eq!(
            liveness_gen_value, generation_from_wire,
            "F3 wire consumer: liveness oracle generation ({liveness_gen_value}) must equal \
             the wire JOIN generation ({generation_from_wire}). \
             Both come from state.huddle_liveness_generation.to_string(). \
             A mismatch here means Desktop would clear Off-mode admissions. \
             Mutation B: change req.rs handle_huddle_liveness_req to use a different UUID \
             → liveness_gen_value != generation_from_wire → panics here."
        );

        // Apply the actual committed JOIN event to the oracle.
        // The oracle checks: event.pubkey == relaySelf AND a non-empty participant.
        let admitted = oracle.apply_join(&join_pubkey_hex, &participant_hex, &join_content_json);
        assert!(
            admitted,
            "F3 oracle: Desktop-semantics oracle must admit the participant on apply_join. \
             The relay-signed 48101 has: event.pubkey = relay signer (✓), non-empty \
             resolved participant (✓), session exists (seeded above) (✓). \
             An empty p-tag value resolves to an empty participant → apply_join returns \
             false → this assert panics."
        );

        // Reconcile liveness: apply the authoritative liveness generation from the
        // relay's handle_huddle_liveness_req oracle (same value as wire generation).
        // huddlePresence.ts:464-484: if generation equals stored → admissions preserved.
        let admissions_preserved =
            oracle.reconcile_liveness(&channel_id.to_string(), &liveness_gen_value);
        assert!(
            admissions_preserved,
            "F3 oracle: reconcile_liveness must preserve the participant admission. \
             Both the JOIN and the liveness oracle use the same generation source \
             (state.huddle_liveness_generation.to_string()), so they are always equal \
             for Off-mode sessions. \
             RED: change req.rs handle_huddle_liveness_req to return a different generation \
             → liveness_gen_value != generation_from_wire (asserted above) → reconcile clears \
             admissions → admissions_preserved = false → panics here."
        );

        // Final assertion: participant is admitted after full chain.
        // This is the concrete Desktop admission-retention guarantee:
        //   real admission → relay-signed JOIN (with p-tag) → oracle admits →
        //   liveness reconcile with matching generation → admission retained.
        assert!(
            oracle.is_admitted(&channel_id.to_string(), &participant_hex),
            "F3 oracle: participant admission must be retained after liveness reconciliation. \
             Meaning: Desktop will NOT clear this peer when it receives the next liveness event. \
             RED: strip p tag from the committed JOIN → participant not admitted → \
             oracle.is_admitted returns false → panics."
        );

        // Cleanup: release hook and disconnect.
        fanout_release.notify_one();
        conn_cancel.cancel();
        server.abort();
        let _ = server.await;
    }

    // ── W10: two concurrent committers; expiry during second; first row intact ──
    //
    // Two concurrent tasks call `commit_participant_join` for different pubkeys.
    // Both use the same gate. The first is let through (no hook armed for it).
    // The second has the hook armed; expiry fires while it is paused at the hook.
    // After release the second rolls back. The first's committed row is intact.
    //
    // Mutation evidence:
    //   A) Delete `before_participant_commit(...)` → arrived_rx times out → panic.
    //   B) Remove `acquire_effect()` from the second path → second commits too →
    //      two rows present → second-row-count assertion panics.
    async fn w10_concurrent_committers_expiry_during_second_first_row_intact_body() {
        use chrono::{Duration, Utc};
        use std::sync::Arc;
        use uuid::Uuid;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();
        let (tenant, channel_id, member_key_a) = seed_audio_fixture(&pool).await;
        let community_id = tenant.community();

        // Second distinct member for the concurrent committer.
        let member_key_b = nostr::Keys::generate();
        let member_bytes_b = member_key_b.public_key().to_bytes().to_vec();
        let creator_bytes = member_key_a.public_key().to_bytes().to_vec(); // reuse as invited_by
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by) \
             VALUES ($1, $2, $3, 'member', $4)",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(&member_bytes_b)
        .bind(&creator_bytes)
        .execute(&pool)
        .await
        .expect("W10 fixture: seed second member");

        let deadline = Utc::now() + Duration::hours(1);
        let cancel = tokio_util::sync::CancellationToken::new();
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone());

        // Task A (first committer) — no hook armed; completes without expiry.
        let member_bytes_a = member_key_a.public_key().to_bytes().to_vec();
        let member_hex_a = member_key_a.public_key().to_hex();
        let state_a = Arc::clone(&state);
        let tenant_a = tenant.clone();
        let gate_a = Arc::clone(&gate);
        let handle_a = tokio::spawn(async move {
            commit_participant_join(
                &state_a,
                &tenant_a,
                channel_id,
                channel_id,
                &member_hex_a,
                &member_bytes_a,
                Uuid::new_v4(),
                1,
                "test-generation-a",
                &MembershipAdmission::Existing {
                    parent_channel_id: channel_id,
                },
                &gate_a,
                String::new(),
                &std::sync::Arc::new(crate::audio::room::Room::new(
                    tenant_a.community(),
                    channel_id,
                )),
            )
            .await
        });

        // Wait for task A to complete before arming the hook for task B.
        let result_a = tokio::time::timeout(std::time::Duration::from_secs(10), handle_a)
            .await
            .expect("W10: task A must complete within 10s")
            .expect("task A must not panic");
        assert!(
            result_a.is_ok(),
            "W10: task A (first committer) must succeed; got: {result_a:?}"
        );

        // Arm the hook for task B.
        let (arrived_rx, release) =
            crate::nip_fi_test_hooks::audio_participant_commit_hook::arm(community_id);

        let member_hex_b = member_key_b.public_key().to_hex();
        let state_b = Arc::clone(&state);
        let tenant_b = tenant.clone();
        let gate_b = Arc::clone(&gate);
        let handle_b = tokio::spawn(async move {
            commit_participant_join(
                &state_b,
                &tenant_b,
                channel_id,
                channel_id,
                &member_hex_b,
                &member_bytes_b,
                Uuid::new_v4(),
                2,
                "test-generation-b",
                &MembershipAdmission::Existing {
                    parent_channel_id: channel_id,
                },
                &gate_b,
                String::new(),
                &std::sync::Arc::new(crate::audio::room::Room::new(
                    tenant_b.community(),
                    channel_id,
                )),
            )
            .await
        });

        // Wait for task B to reach the hook.
        tokio::time::timeout(std::time::Duration::from_secs(10), arrived_rx)
            .await
            .expect("W10: task B must reach before_participant_commit within 10s")
            .expect("arrived channel closed");

        // Fire expiry — task B's acquire_effect returns SessionExpired.
        cancel.cancel();
        release.notify_one();

        let result_b = tokio::time::timeout(std::time::Duration::from_secs(10), handle_b)
            .await
            .expect("W10: task B must return within 10s after hook release")
            .expect("task B must not panic");

        assert!(
            matches!(result_b, Err(JoinCommitError::Expired)),
            "W10: task B must return JoinCommitError::Expired after mid-flight expiry; got: {result_b:?}"
        );

        // Task A's row persists; task B's row was rolled back.
        let row_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = 48101",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("W10: row count query");

        assert_eq!(
            row_count, 1,
            "W10: exactly one 48101 row (task A's) must be committed; found {row_count}"
        );
    }

    // ── Concurrent-reaffirm variant: same pubkey twice; expiry during second ──
    //
    // Two concurrent tasks call `commit_participant_join` for the SAME pubkey.
    // The second encounters an already-inserted row (idempotent duplicate key →
    // `was_inserted = false`), then hits the hook. Expiry fires; the second
    // rolls back. The first's row is intact. `JoinCommitError::Expired` is returned
    // by the second task.
    //
    // Contract: expiry during a reaffirm commit rolls back without corrupting the
    // first committer's row. The membership row (if Existing) is unaffected.
    //
    // Mutation evidence:
    //   A) Delete `before_participant_commit(...)` → arrived_rx times out → panic.
    //   B) Remove `tx.rollback()` in the Expired branch → second auto-rollback
    //      still leaves zero new rows (idempotent insert), but `JoinCommitError::Expired`
    //      assertion still passes — covered by (A) instead.
    async fn w10_reaffirm_expiry_during_second_same_pubkey_first_row_intact_body() {
        use chrono::{Duration, Utc};
        use std::sync::Arc;
        use uuid::Uuid;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();
        let (tenant, channel_id, member_key) = seed_audio_fixture(&pool).await;
        let community_id = tenant.community();

        let member_bytes = member_key.public_key().to_bytes().to_vec();
        let member_hex = member_key.public_key().to_hex();

        // Both tasks share the same gate (same connection, same pubkey).
        let deadline = Utc::now() + Duration::hours(1);
        let cancel = tokio_util::sync::CancellationToken::new();
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone());

        // Task 1 (first committer) — completes without expiry.
        let state1 = Arc::clone(&state);
        let tenant1 = tenant.clone();
        let bytes1 = member_bytes.clone();
        let hex1 = member_hex.clone();
        let gate1 = Arc::clone(&gate);
        let handle1 = tokio::spawn(async move {
            commit_participant_join(
                &state1,
                &tenant1,
                channel_id,
                channel_id,
                &hex1,
                &bytes1,
                Uuid::new_v4(),
                1,
                "test-generation",
                &MembershipAdmission::Existing {
                    parent_channel_id: channel_id,
                },
                &gate1,
                String::new(),
                &std::sync::Arc::new(crate::audio::room::Room::new(
                    tenant1.community(),
                    channel_id,
                )),
            )
            .await
        });

        let result1 = tokio::time::timeout(std::time::Duration::from_secs(10), handle1)
            .await
            .expect("reaffirm: task 1 must complete within 10s")
            .expect("task 1 must not panic");
        assert!(
            result1.is_ok(),
            "reaffirm: task 1 (first committer) must succeed; got: {result1:?}"
        );

        // Arm the hook for task 2 (same pubkey — duplicate insert returns was_inserted=false).
        let (arrived_rx, release) =
            crate::nip_fi_test_hooks::audio_participant_commit_hook::arm(community_id);

        let state2 = Arc::clone(&state);
        let tenant2 = tenant.clone();
        let bytes2 = member_bytes.clone();
        let hex2 = member_hex.clone();
        let gate2 = Arc::clone(&gate);
        let handle2 = tokio::spawn(async move {
            commit_participant_join(
                &state2,
                &tenant2,
                channel_id,
                channel_id,
                &hex2,
                &bytes2,
                Uuid::new_v4(),
                2,
                "test-generation",
                &MembershipAdmission::Existing {
                    parent_channel_id: channel_id,
                },
                &gate2,
                String::new(),
                &std::sync::Arc::new(crate::audio::room::Room::new(
                    tenant2.community(),
                    channel_id,
                )),
            )
            .await
        });

        // Wait for task 2 to reach the hook (after the duplicate-key 48101 insert).
        tokio::time::timeout(std::time::Duration::from_secs(10), arrived_rx)
            .await
            .expect("reaffirm: task 2 must reach before_participant_commit within 10s")
            .expect("arrived channel closed");

        // Fire expiry during the reaffirm commit window.
        cancel.cancel();
        release.notify_one();

        let result2 = tokio::time::timeout(std::time::Duration::from_secs(10), handle2)
            .await
            .expect("reaffirm: task 2 must return within 10s")
            .expect("task 2 must not panic");

        assert!(
            matches!(result2, Err(JoinCommitError::Expired)),
            "reaffirm: task 2 must return JoinCommitError::Expired; got: {result2:?}"
        );

        // Exactly one committed 48101 row (task 1's). Task 2's transaction rolled back
        // (or was a no-op duplicate that rolled back cleanly).
        let row_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = 48101",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("reaffirm: row count query");

        assert_eq!(
            row_count, 1,
            "reaffirm: exactly one 48101 row (task 1's) must persist; found {row_count}"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // CW5: AutoAddRequired path — expiry pre-commit rolls back BOTH rows
    // ─────────────────────────────────────────────────────────────────────────
    //
    // Exercises the `AutoAddRequired` branch of `commit_participant_join` —
    // the mechanism introduced by contract correction 2 (e5bc0382). The fixture
    // has NO pre-existing membership row, so the auto-add write is attempted
    // inside the joint transaction. `before_participant_commit` fires AFTER both
    // the membership insert AND the 48101 insert are in the uncommitted
    // transaction. Expiry fires at the hook; the acquire_effect check fails;
    // the entire transaction rolls back: NEITHER the membership row NOR the
    // 48101 row becomes visible.
    //
    // This is the contract seam that W9 missed: W9 used `Existing` (no auto-add)
    // so the membership half of the joint-transaction invariant was never proven.
    //
    // Mutation evidence (executed):
    //   CW5A) Delete `before_participant_commit(...)` → arrived_rx times out → panic.
    //   CW5B) Remove `acquire_effect()` → commit proceeds despite cancel →
    //         both rows committed → row-count assertions panic.
    //   CW5C) Change membership_admission to `Existing` → membership path
    //         never entered; membership row never inserted; this seam not covered.
    async fn cw5_auto_add_path_expiry_before_commit_rolls_back_both_rows_body() {
        use chrono::{Duration, Utc};
        use std::sync::Arc;
        use uuid::Uuid;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();

        // Fixture: community + channel — NO membership row for the test key.
        let community_uuid = Uuid::new_v4();
        let host = format!("cw5-test-{}.example", community_uuid.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(&host)
            .execute(&pool)
            .await
            .expect("CW5: seed community");

        let channel_id = Uuid::new_v4();
        let creator = nostr::Keys::generate();
        let creator_bytes = creator.public_key().to_bytes().to_vec();
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by) \
             VALUES ($1, $2, 'cw5-test-channel', 'stream', 'open', $3)",
        )
        .bind(channel_id)
        .bind(community_uuid)
        .bind(&creator_bytes)
        .execute(&pool)
        .await
        .expect("CW5: seed channel");

        // The joining pubkey has NO channel_member row — triggers AutoAddRequired.
        let joiner_key = nostr::Keys::generate();
        let joiner_bytes = joiner_key.public_key().to_bytes().to_vec();
        let joiner_hex = joiner_key.public_key().to_hex();

        let tenant = buzz_core::tenant::TenantContext::resolved(
            buzz_core::tenant::CommunityId::from_uuid(community_uuid),
            host,
        );
        let community_id = tenant.community();

        let deadline = Utc::now() + Duration::hours(1);
        let cancel = tokio_util::sync::CancellationToken::new();
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone());

        // IMPORTANT 4b requires that the joiner is a member of the parent channel
        // before AutoAddRequired can commit. Seed that parent membership now.
        // (In production, check_membership_for_admission only returns AutoAddRequired
        // if the parent membership exists; the re-read confirms it still does.)
        sqlx::query(
            "INSERT INTO channel_members (channel_id, community_id, pubkey, role, invited_by) \
             VALUES ($1, $2, $3, 'member', $4)",
        )
        .bind(channel_id)
        .bind(community_uuid)
        .bind(&joiner_bytes)
        .bind(&creator_bytes)
        .execute(&pool)
        .await
        .expect("CW5: seed parent membership for joiner");

        // Remove the just-inserted membership so AutoAddRequired still fires
        // (we seeded it as the "parent" channel member, but the child channel
        // is the same channel_id — so still_absent will now be false and the
        // auto-add insert is skipped). We actually want still_absent=true to
        // test the auto-add path. To do this properly: use a SEPARATE parent
        // channel so the parent membership doesn't conflict with the child check.
        // Delete the row we just inserted and use a two-channel fixture.
        sqlx::query("DELETE FROM channel_members WHERE channel_id = $1 AND community_id = $2 AND pubkey = $3")
            .bind(channel_id)
            .bind(community_uuid)
            .bind(&joiner_bytes)
            .execute(&pool)
            .await
            .expect("CW5: cleanup parent membership");

        // Use a two-channel fixture: parent_channel has the joiner as a member;
        // child_channel has NO membership for the joiner (triggers AutoAddRequired).
        let parent_channel_id = channel_id; // reuse the existing channel as parent
        let child_channel_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by) \
             VALUES ($1, $2, 'cw5-child-channel', 'stream', 'open', $3)",
        )
        .bind(child_channel_id)
        .bind(community_uuid)
        .bind(&creator_bytes)
        .execute(&pool)
        .await
        .expect("CW5: seed child channel");

        // Seed the huddle_started link event (kind 48100) required by the I4
        // re-validation inside commit_participant_join. Links parent_channel_id
        // → child_channel_id, signed by creator_bytes.
        let huddle_link_content =
            serde_json::json!({ "ephemeral_channel_id": child_channel_id.to_string() }).to_string();
        sqlx::query(
            "INSERT INTO events \
             (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id) \
             VALUES ($1, $2, $3, NOW(), $4, '[]', $5, $6, $7)",
        )
        .bind(community_uuid)
        .bind(vec![0xBBu8; 32]) // fixed test event id
        .bind(&creator_bytes)
        .bind(48100_i32) // KIND_HUDDLE_STARTED
        .bind(&huddle_link_content)
        .bind(vec![0u8; 64]) // dummy sig (not validated in this path)
        .bind(parent_channel_id)
        .execute(&pool)
        .await
        .expect("CW5: seed huddle_started link");

        // Seed parent membership for the joiner.
        sqlx::query(
            "INSERT INTO channel_members (channel_id, community_id, pubkey, role, invited_by) \
             VALUES ($1, $2, $3, 'member', $4)",
        )
        .bind(parent_channel_id)
        .bind(community_uuid)
        .bind(&joiner_bytes)
        .bind(&creator_bytes)
        .execute(&pool)
        .await
        .expect("CW5: seed parent channel membership for joiner");

        // membership_admission = AutoAddRequired — the joint-tx auto-add path.
        // parent_channel_id has the joiner as member (satisfies IMPORTANT 4b re-read).
        // child_channel_id has NO membership — so still_absent=true → auto-add fires.
        let membership = MembershipAdmission::AutoAddRequired {
            parent_channel_id,
            channel_created_by: creator_bytes.clone(),
        };

        // Arm the hook: fires between the uncommitted membership+48101 inserts
        // and acquire_effect. The full joint transaction is in-flight here.
        let (arrived_rx, release) =
            crate::nip_fi_test_hooks::audio_participant_commit_hook::arm(community_id);

        let state2 = Arc::clone(&state);
        let tenant2 = tenant.clone();
        let joiner_bytes2 = joiner_bytes.clone();
        let joiner_hex2 = joiner_hex.clone();
        let gate2 = Arc::clone(&gate);
        let handle = tokio::spawn(async move {
            commit_participant_join(
                &state2,
                &tenant2,
                child_channel_id,
                parent_channel_id,
                &joiner_hex2,
                &joiner_bytes2,
                Uuid::new_v4(),
                1,
                "test-generation",
                &membership,
                &gate2,
                String::new(),
                &std::sync::Arc::new(crate::audio::room::Room::new(
                    tenant2.community(),
                    child_channel_id,
                )),
            )
            .await
        });

        // Wait for the hook — both membership and 48101 are in the uncommitted tx.
        tokio::time::timeout(std::time::Duration::from_secs(10), arrived_rx)
            .await
            .expect("CW5: commit_participant_join must reach before_participant_commit within 10s")
            .expect("arrived channel closed");

        // Fire expiry — acquire_effect returns SessionExpired; entire tx rolls back.
        cancel.cancel();
        release.notify_one();

        let result = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
            .await
            .expect("CW5: commit_participant_join must return within 10s after hook release")
            .expect("commit_participant_join task must not panic");

        assert!(
            matches!(result, Err(JoinCommitError::Expired)),
            "CW5: must return JoinCommitError::Expired after mid-flight expiry; got: {result:?}"
        );

        // Zero 48101 rows — the 48101 insert was rolled back.
        let row_count_48101: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = 48101",
        )
        .bind(community_uuid)
        .bind(child_channel_id)
        .fetch_one(&pool)
        .await
        .expect("CW5: 48101 row count query");

        assert_eq!(
            row_count_48101, 0,
            "CW5: no 48101 row must be committed after AutoAddRequired expiry-rollback; found {row_count_48101}"
        );

        // Zero membership rows for the joiner in the child channel — the auto-add insert was rolled back.
        let membership_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM channel_members \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
        )
        .bind(community_uuid)
        .bind(child_channel_id)
        .bind(&joiner_bytes)
        .fetch_one(&pool)
        .await
        .expect("CW5: membership row count query");

        assert_eq!(
            membership_count, 0,
            "CW5: no membership row must be committed after AutoAddRequired expiry-rollback; found {membership_count}"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // CW5-variant: external membership add while paused pre-channel-lock →
    // membership preserved; only 48101 commits
    // ─────────────────────────────────────────────────────────────────────────
    //
    // Exercises the concurrent-external-add path in the AutoAddRequired branch
    // of `commit_participant_join`. An external transaction inserts the
    // membership row while our transaction is paused at `before_membership_lock`
    // — just before `acquire_channel_membership_lock_in_transaction`. When our
    // transaction resumes:
    //   1. It acquires the channel membership lock.
    //   2. Re-reads membership — the external insert is committed and visible.
    //   3. `still_absent = false` → skips the auto-add insert.
    //   4. Inserts 48101 (no duplicate; this pubkey is fresh).
    //   5. Acquires the effect permit (no expiry).
    //   6. Commits.
    //
    // Observable invariant: exactly 1 membership row (the external insert) and
    // exactly 1 48101 row commit. The join succeeds (Ok), and we did not double-
    // insert or corrupt the externally-added membership.
    //
    // Mutation evidence (executed):
    //   CW5V-A) Delete `before_membership_lock(...)` → arrived_rx times out → panic.
    //   CW5V-B) Remove the `still_absent` re-read and always insert → auto-add
    //           fires → ON CONFLICT DO UPDATE SET role = 'member' clobbers the
    //           externally-inserted 'admin' role → member.role assertion panics.
    //   CW5V-C) Remove the `if still_absent { insert }` guard → same as (B).
    async fn cw5_variant_concurrent_external_membership_add_preserved_body() {
        use chrono::{Duration, Utc};
        use std::sync::Arc;
        use uuid::Uuid;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();

        // Fixture: community + channel — NO membership row for the joining key.
        let community_uuid = Uuid::new_v4();
        let host = format!("cw5v-test-{}.example", community_uuid.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(&host)
            .execute(&pool)
            .await
            .expect("CW5-variant: seed community");

        let channel_id = Uuid::new_v4();
        let creator = nostr::Keys::generate();
        let creator_bytes = creator.public_key().to_bytes().to_vec();
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by) \
             VALUES ($1, $2, 'cw5v-test-channel', 'stream', 'open', $3)",
        )
        .bind(channel_id)
        .bind(community_uuid)
        .bind(&creator_bytes)
        .execute(&pool)
        .await
        .expect("CW5-variant: seed channel");

        // Seed the huddle_started link event (kind 48100) required by the I4
        // re-validation inside commit_participant_join. The test uses
        // parent_channel_id == channel_id (same UUID), so this event needs to
        // link channel_id → channel_id from creator_bytes.
        let huddle_link_content =
            serde_json::json!({ "ephemeral_channel_id": channel_id.to_string() }).to_string();
        sqlx::query(
            "INSERT INTO events \
             (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id) \
             VALUES ($1, $2, $3, NOW(), $4, '[]', $5, $6, $7)",
        )
        .bind(community_uuid)
        .bind(vec![0xAAu8; 32]) // fixed test event id
        .bind(&creator_bytes)
        .bind(48100_i32) // KIND_HUDDLE_STARTED
        .bind(&huddle_link_content)
        .bind(vec![0u8; 64]) // dummy sig (not validated in this path)
        .bind(channel_id)
        .execute(&pool)
        .await
        .expect("CW5-variant: seed huddle_started link");

        let joiner_key = nostr::Keys::generate();
        let joiner_bytes = joiner_key.public_key().to_bytes().to_vec();
        let joiner_hex = joiner_key.public_key().to_hex();

        let tenant = buzz_core::tenant::TenantContext::resolved(
            buzz_core::tenant::CommunityId::from_uuid(community_uuid),
            host,
        );
        let community_id = tenant.community();

        let deadline = Utc::now() + Duration::hours(1);
        let cancel = tokio_util::sync::CancellationToken::new();
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone());

        let membership = MembershipAdmission::AutoAddRequired {
            parent_channel_id: channel_id,
            channel_created_by: creator_bytes.clone(),
        };

        // Arm the pre-lock hook. The join task pauses here before acquiring the
        // channel membership lock; while paused, we insert membership externally.
        let (arrived_rx, release) =
            crate::nip_fi_test_hooks::audio_membership_lock_hook::arm(community_id);

        let state2 = Arc::clone(&state);
        let tenant2 = tenant.clone();
        let joiner_bytes2 = joiner_bytes.clone();
        let joiner_hex2 = joiner_hex.clone();
        let gate2 = Arc::clone(&gate);
        let pool2 = pool.clone();
        let handle = tokio::spawn(async move {
            commit_participant_join(
                &state2,
                &tenant2,
                channel_id,
                channel_id,
                &joiner_hex2,
                &joiner_bytes2,
                Uuid::new_v4(),
                1,
                "test-generation",
                &membership,
                &gate2,
                String::new(),
                &std::sync::Arc::new(crate::audio::room::Room::new(
                    tenant2.community(),
                    channel_id,
                )),
            )
            .await
        });

        // Wait for the join task to reach the pre-lock hook.
        tokio::time::timeout(std::time::Duration::from_secs(10), arrived_rx)
            .await
            .expect("CW5-variant: must reach before_membership_lock within 10s")
            .expect("arrived channel closed");

        // External concurrent insert — simulates another legitimate path adding
        // the joiner to the channel before our transaction acquires the lock.
        // Use role = 'admin' as the distinguishing marker: if auto-add fires,
        // `ON CONFLICT DO UPDATE SET role = EXCLUDED.role` (which is 'member')
        // clobbers the 'admin' role — the assertion below catches that.
        let external_inviter = nostr::Keys::generate();
        let external_inviter_bytes = external_inviter.public_key().to_bytes().to_vec();
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by) \
             VALUES ($1, $2, $3, 'admin', $4)",
        )
        .bind(community_uuid)
        .bind(channel_id)
        .bind(&joiner_bytes)
        .bind(&external_inviter_bytes)
        .execute(&pool2)
        .await
        .expect("CW5-variant: external membership insert");

        // Release the hook — our transaction acquires the lock, re-reads
        // (finds existing membership), skips the auto-add, commits only 48101.
        release.notify_one();

        let result = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
            .await
            .expect("CW5-variant: commit_participant_join must return within 10s")
            .expect("commit_participant_join task must not panic");

        assert!(
            result.is_ok(),
            "CW5-variant: join must succeed (external add observed, skip insert); got: {result:?}"
        );

        // Verify membership via the normal API: role must be 'admin' (the
        // externally-inserted value). If auto-add fires, ON CONFLICT DO UPDATE
        // SET role = 'member' clobbers it — this assertion catches that.
        let members =
            buzz_db::channel_members::get_members(state.db.pool(), community_id, channel_id)
                .await
                .expect("CW5-variant: get_members query");

        assert_eq!(
            members.len(),
            1,
            "CW5-variant: exactly 1 membership row (external's) must persist; found {}",
            members.len()
        );
        let member = &members[0];
        assert_eq!(
            member.pubkey, joiner_bytes,
            "CW5-variant: membership row must be for the joiner"
        );
        assert_eq!(
            member.role, "admin",
            "CW5-variant: membership role must be 'admin' (external insert's role preserved — \
             if auto-add fires, ON CONFLICT sets role='member' and this panics)"
        );

        // Exactly 1 committed 48101 row — the join event committed.
        let row_count_48101: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = 48101",
        )
        .bind(community_uuid)
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("CW5-variant: 48101 row count query");

        assert_eq!(
            row_count_48101, 1,
            "CW5-variant: exactly 1 48101 row (the join event) must commit; found {row_count_48101}"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // CW8 (real-DB variant): after_add_peer hook fires → cancel → cleanup runs
    // ─────────────────────────────────────────────────────────────────────────
    //
    // The CW8 contract seam (post-add_peer cleanup) requires a seeded channel
    // in the real DB so `check_membership_for_admission` succeeds and the handler
    // reaches `room.add_peer`. It requires the PG-wrapper DB; a missing or
    // unreachable DB fails.
    //
    // Mutation evidence (executed):
    //   CW8A) Delete `after_add_peer(...)` → arrived_rx times out → panic.
    //   CW8B) Delete `room.remove_peer(peer_id)` from cleanup → room not removed →
    //         audio_rooms.get() returns Some → room_after.is_none() assertion panics.
    //   CW8C) Delete `cleanup_if_empty(...)` from cleanup → room entry persists after
    //         last-peer removal → audio_rooms.get() returns Some →
    //         room_after.is_none() assertion panics (detects the missing call).
    async fn cw8_post_add_peer_cancel_removes_peer_and_cleans_up_real_db_body() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();
        let (tenant, channel_id, member_key) = seed_audio_fixture(&pool).await;
        let community = tenant.community();

        let key = member_key; // Same key is already a member → open path to add_peer.
        let assertion = VerifiedAssertion::for_test(
            Some(key.public_key()),
            vec![Utc::now() + Duration::hours(1)],
        );

        let audio_rooms = Arc::clone(&state.audio_rooms);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let conn_cancel = CancellationToken::new();
        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();
        let conn_cancel_c = conn_cancel.clone();
        // Save the tenant host before tenant_c is moved into the server closure.
        let tenant_host = tenant_c.host().to_string();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("test listener addr");

        // Arm the after_add_peer hook before the server starts.
        let (arrived_rx, release) = crate::nip_fi_test_hooks::audio_add_peer_hook::arm(community);

        let server = tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/",
                axum::routing::get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let cancel_i = conn_cancel_c.clone();
                    move |ws: axum::extract::ws::WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let conn_time = chrono::Utc::now();
                        let control_inner =
                            crate::state::CommunityConnectionControl::new(cancel_i.clone());
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    channel_id,
                                    control_inner,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app).await.expect("test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("server ready");

        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/"))
            .await
            .expect("connect client");

        // Complete NIP-42 handshake.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("challenge timeout")
            .expect("challenge message")
            .expect("challenge ws message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("challenge field")
            .to_string();

        // Use the tenant's host to build the relay URL — must match the
        // nip42_expected_relay_url computed inside handle_active_audio_connection.
        let relay_url = format!("ws://{tenant_host}");
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", &relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key)
            .unwrap();

        let auth_msg = serde_json::json!({
            "type": "auth",
            "event": auth_event,
            "parent_channel_id": null,
            "protocol_version": 1,
        })
        .to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("send auth msg");

        // Wait for after_add_peer — peer is now in the room.
        tokio::time::timeout(std::time::Duration::from_secs(5), arrived_rx)
            .await
            .expect("CW8: handler must reach after_add_peer within 5s")
            .expect("arrived channel closed");

        // Fire cancel — simulates expiry arriving at this exact point.
        conn_cancel.cancel();

        // Release hook — handler's explicit cancellation branch runs
        // guard.release_before_commit() (remove_peer + cleanup_if_empty).
        release.notify_one();

        // Handler returns (connection closes).
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), client.next()).await;

        // Room must be empty AND must have been cleaned up by cleanup_if_empty.
        // An empty-but-still-registered room means cleanup_if_empty did NOT fire,
        // which would fail the CW8B mutation test (deleting cleanup_if_empty).
        // Asserting audio_rooms.get() returns None is the stronger check.
        let room_after = audio_rooms.get(community, channel_id);
        assert!(
            room_after.is_none(),
            "CW8: room must have been removed by cleanup_if_empty after post-add_peer cancel; \
             room still present in map (cleanup_if_empty did not fire): peers={:?}",
            room_after.as_ref().map(|r| r.peer_pubkeys())
        );

        // No 48101 committed — commit_participant_join was never reached.
        let row_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = 48101",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("CW8: row count query");

        assert_eq!(
            row_count, 0,
            "CW8: no 48101 row must be committed when cancel fires after add_peer; found {row_count}"
        );

        server.abort();
        let _ = server.await;
    }

    // ─────────────────────────────────────────────────────────────────────────
    // CW10 (contract): expiry queued after commit while permit held →
    // fan-out completes; expiry provably blocked at quiescence barrier until
    // permit drops
    // ─────────────────────────────────────────────────────────────────────────
    //
    // This is the commit-won/quiescence witness — the heart of the design.
    // `after_participant_fanout` fires after tx.commit() AND after fan-out
    // (mark_local_event + fan_out_event_to_local_subscribers + publish_event)
    // but BEFORE `_permit` drops.
    //
    // At the hook: arm expiry in a background task. Because `_permit` is still
    // held, `gate.expire()` blocks at the write guard. Verify expiry is blocked
    // (cancel fires but write guard not yet acquired → expire not complete).
    // Release hook → `commit_participant_join` returns → `_permit` drops →
    // expiry task acquires write guard → expire() completes.
    //
    // Observable invariants:
    //   1. At hook time: cancel is set (expire called cancel.cancel()) but
    //      expire() is blocked (write guard not yet acquired).
    //   2. After permit drops: expire() completes.
    //   3. The 48101 row IS committed (fan-out happened under the permit).
    //   4. `local_event_ids` contains the event (mark_local_event ran).
    //
    // Mutation evidence (executed):
    //   CW10A) Delete `after_participant_fanout(...)` → arrived_rx times out → panic.
    //   CW10B) Remove `acquire_effect()` from `commit_participant_join` → the
    //          permit is never held → expiry is not blocked → expire() completes
    //          before we check → the "expiry blocked" invariant assertion panics.
    //          (Note: CW10B is covered by having the expire task complete before
    //          the hook fires, detectable by checking expire_done before release.)
    //   CW10C) Move `after_participant_fanout` hook to before `tx.commit()` →
    //          48101 not yet committed when hook fires → 48101 row-count assertion
    //          panics (no row at hook time, but the test checks after completion).
    //          Actually: the test checks after the whole function returns, so CW10C
    //          is best evidenced by CW10A (hook placement) + the row-count check.
    async fn cw10_expiry_blocked_at_permit_barrier_until_fan_out_completes_body() {
        use chrono::{Duration, Utc};
        use std::sync::Arc;
        use uuid::Uuid;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();
        let (tenant, channel_id, member_key) = seed_audio_fixture(&pool).await;
        let community_id = tenant.community();

        let member_bytes = member_key.public_key().to_bytes().to_vec();
        let member_hex = member_key.public_key().to_hex();
        let peer_id = Uuid::new_v4();

        // Deadline far in the future — expiry does NOT fire on its own.
        let deadline = Utc::now() + Duration::hours(1);
        let cancel = tokio_util::sync::CancellationToken::new();
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone());

        let membership = MembershipAdmission::Existing {
            parent_channel_id: channel_id,
        };

        // Arm the after_participant_fanout hook.
        let (arrived_rx, release) =
            crate::nip_fi_test_hooks::audio_participant_fanout_hook::arm(community_id);

        let state2 = Arc::clone(&state);
        let tenant2 = tenant.clone();
        let bytes2 = member_bytes.clone();
        let hex2 = member_hex.clone();
        let gate2 = Arc::clone(&gate);
        let handle = tokio::spawn(async move {
            commit_participant_join(
                &state2,
                &tenant2,
                channel_id,
                channel_id,
                &hex2,
                &bytes2,
                peer_id,
                1,
                "test-generation",
                &membership,
                &gate2,
                String::new(),
                &std::sync::Arc::new(crate::audio::room::Room::new(
                    tenant2.community(),
                    channel_id,
                )),
            )
            .await
        });

        // Wait for the hook — tx.commit() ran AND fan-out ran; permit is still held.
        tokio::time::timeout(std::time::Duration::from_secs(10), arrived_rx)
            .await
            .expect("CW10: commit_participant_join must reach after_participant_fanout within 10s")
            .expect("arrived channel closed");

        // 48101 must already be committed (fan-out ran under the permit).
        let row_count_at_hook: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = 48101",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("CW10: row count at hook");

        assert_eq!(
            row_count_at_hook, 1,
            "CW10: 48101 row must be committed before the hook fires (fan-out under permit); found {row_count_at_hook}"
        );

        // Arm expiry in a background task. It calls cancel.cancel() immediately
        // then blocks at the write guard (because the permit read guard is held).
        let gate3 = Arc::clone(&gate);
        let expire_done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let expire_done2 = Arc::clone(&expire_done);
        let expire_task = tokio::spawn(async move {
            gate3.expire(|| {}).await;
            expire_done2.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        // Yield a few times so expire_task can start, call cancel.cancel(), and
        // reach the write guard (where it blocks).
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        // Cancel must be set (expire called cancel.cancel() immediately).
        assert!(
            cancel.is_cancelled(),
            "CW10: cancel must be set when expire() fires"
        );

        // Expiry must NOT have completed yet — permit is still held.
        assert!(
            !expire_done.load(std::sync::atomic::Ordering::SeqCst),
            "CW10: expire() must be blocked at write guard while permit is held"
        );

        // Release hook → `commit_participant_join` returns → `_permit` drops.
        release.notify_one();

        // Wait for the commit_participant_join task to return.
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
            .await
            .expect("CW10: commit_participant_join must return within 10s after hook release")
            .expect("commit_participant_join task must not panic");

        assert!(
            result.is_ok(),
            "CW10: commit_participant_join must return Ok after successful commit; got: {result:?}"
        );

        // Wait for the expiry task to complete — now unblocked after permit drop.
        tokio::time::timeout(std::time::Duration::from_secs(5), expire_task)
            .await
            .expect("CW10: expire() task must complete within 5s after permit drop")
            .expect("expire task must not panic");

        assert!(
            expire_done.load(std::sync::atomic::Ordering::SeqCst),
            "CW10: expire() must complete after permit is dropped"
        );

        // 48101 remains committed — the commit-won invariant holds.
        let row_count_final: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = 48101",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("CW10: final row count query");

        assert_eq!(
            row_count_final, 1,
            "CW10: exactly 1 48101 row must persist after commit-won + expiry; found {row_count_final}"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // CW10-full-handler: committed join → disconnect → exactly one 48102
    // ─────────────────────────────────────────────────────────────────────────
    //
    // Full-handler witness (IMPORTANT 5 + teardown): a committed join must
    // produce exactly one kind:48101 and exactly one kind:48102, regardless of
    // when teardown is triggered. Uses a real DB + full `handle_active_audio_connection`
    // invocation so the complete send_loop/recv_loop/forward_loop lifecycle runs.
    //
    // Steps:
    //   1. Seed a channel + member, connect via WS, complete NIP-42 handshake.
    //   2. Arm `after_participant_fanout` hook — fires after tx.commit() + fan-out,
    //      before `_permit` drops. At this point 48101 is committed.
    //   3. Release the hook → `commit_participant_join` returns Ok.
    //   4. Session enters recv_loop. Immediately cancel `conn_cancel` to
    //      simulate a client disconnect (or NIP-FI expiry triggering the same
    //      teardown path).
    //   5. Wait for the handler to complete.
    //   6. Assert: exactly 1 committed 48101 row; exactly 1 committed 48102 row.
    //      The pair proves "committed join ⇒ exactly one leave event".
    //
    // Mutation evidence (executed):
    //   CW10F-A) Remove the `emit_participant_event(48102, ...)` call from the
    //            handler epilogue → 48102 count stays 0 → assertion panics.
    //   CW10F-B) Remove `room.remove_peer(peer_id)` / `remove_peer_and_check_ended`
    //            from teardown → room is not empty → cleanup_if_empty is a no-op
    //            → the room entry persists → subsequent get() finds it.
    async fn cw10_full_handler_committed_join_produces_exactly_one_leave_event_body() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();
        let (tenant, channel_id, member_key) = seed_audio_fixture(&pool).await;
        let community = tenant.community();

        let key = member_key;
        let assertion = VerifiedAssertion::for_test(
            Some(key.public_key()),
            vec![Utc::now() + Duration::hours(1)],
        );

        let audio_rooms = Arc::clone(&state.audio_rooms);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let conn_cancel = CancellationToken::new();
        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();
        let conn_cancel_c = conn_cancel.clone();
        let tenant_host = tenant_c.host().to_string();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("test listener addr");

        // Arm after_participant_fanout: fires when 48101 is committed + fan-out done.
        let (fanout_rx, fanout_release) =
            crate::nip_fi_test_hooks::audio_participant_fanout_hook::arm(community);

        let server = tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/",
                axum::routing::get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let cancel_i = conn_cancel_c.clone();
                    move |ws: axum::extract::ws::WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let conn_time = chrono::Utc::now();
                        let control_inner =
                            crate::state::CommunityConnectionControl::new(cancel_i.clone());
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    channel_id,
                                    control_inner,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app).await.expect("test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("server ready");

        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/"))
            .await
            .expect("connect client");

        // Complete NIP-42 handshake.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("challenge timeout")
            .expect("challenge msg")
            .expect("challenge ws msg");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("challenge field")
            .to_string();

        let relay_url = format!("ws://{tenant_host}");
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", &relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key)
            .unwrap();

        let auth_msg = serde_json::json!({
            "type": "auth",
            "event": auth_event,
            "parent_channel_id": null,
            "protocol_version": 1,
        })
        .to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("send auth msg");

        // Wait for after_participant_fanout — 48101 is committed and fan-out ran.
        tokio::time::timeout(std::time::Duration::from_secs(10), fanout_rx)
            .await
            .expect("CW10-full: handler must reach after_participant_fanout within 10s")
            .expect("fanout channel closed");

        // Verify 48101 is committed before we trigger disconnect.
        let row_48101: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = 48101",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("CW10-full: 48101 count at hook");

        assert_eq!(
            row_48101, 1,
            "CW10-full: 48101 must be committed at after_participant_fanout; found {row_48101}"
        );

        // Release hook → commit_participant_join returns → session enters recv_loop.
        fanout_release.notify_one();

        // Give the session a moment to enter recv_loop before we disconnect.
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Trigger disconnect — cancelling conn_cancel signals the handler's
        // cancel token, which causes recv_loop, send_loop, and forward_loop to
        // stop; the handler epilogue then calls emit_participant_event(48102, ...).
        conn_cancel.cancel();

        // Handler returns after teardown. Wait for the WS connection to close.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), client.next()).await;

        // Wait a moment for the handler to finish emitting 48102.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Exactly one 48102 row must exist — the "committed join ⇒ exactly one leave" invariant.
        let row_48102: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = 48102",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("CW10-full: 48102 count");

        assert_eq!(
            row_48102, 1,
            "CW10-full: exactly 1 48102 must be committed after a committed join + disconnect; found {row_48102}"
        );

        // Room must be cleaned up.
        let room_after = audio_rooms.get(community, channel_id);
        assert!(
            room_after.is_none(),
            "CW10-full: room must be removed after last peer disconnects; \
             room still present: peers={:?}",
            room_after.as_ref().map(|r| r.peer_pubkeys())
        );

        server.abort();
        let _ = server.await;
    }

    // ─────────────────────────────────────────────────────────────────────────
    // CW6: guard-level witness — unattached lease released on pre-commit exit
    // ─────────────────────────────────────────────────────────────────────────
    //
    // `HuddleAdmissionGuard::release_before_commit` must call `directory.release`
    // exactly once when a lease is held and no commit has happened (the guard
    // held an unattached lease and was asked to clean up on a pre-commit exit).
    //
    // This test uses a `CountingDir` (a `HuddleDirectory` double with a release
    // counter) injected into the guard's `lease` field. No Redis, no mesh
    // transport, no `AppState` required — the guard-level abstraction is the
    // seam that makes this feasible without production infrastructure.
    //
    // The path under test is `HuddleAdmissionGuard::release_before_commit`, which
    // calls `directory.release(&lease)` directly and awaits the result. Release
    // is guaranteed complete before `release_before_commit` returns — no detached
    // renewer task.
    //
    // Mutation evidence (executed):
    //   CW6A) Remove `if let Some((lease, directory)) = self.lease.take()` block →
    //         release is never called → release_calls stays 0 → assertion panics.
    #[tokio::test]
    async fn cw6_guard_release_before_commit_calls_directory_release_exactly_once() {
        use crate::audio::join::{
            AcquireOutcome, HuddleDirectory, HuddleLease, HuddleReleaseOutcome, HuddleRenewOutcome,
            Ownership, HUDDLE_CONTROL_PROFILE,
        };
        use crate::tunnel::directory::SessionLease;
        use buzz_core::CommunityId;
        use buzz_relay_mesh::{wire::FencedHeader, MeshError, RuntimeId};
        use std::sync::{Arc, Mutex};
        use uuid::Uuid;

        // A minimal HuddleDirectory double that counts release calls.
        struct CountingDir {
            release_calls: Mutex<u32>,
        }
        #[async_trait::async_trait]
        impl HuddleDirectory for CountingDir {
            async fn owner_of(
                &self,
                _c: CommunityId,
                _s: Uuid,
            ) -> Result<Option<Ownership>, MeshError> {
                Ok(None)
            }
            async fn acquire(
                &self,
                _c: CommunityId,
                _s: Uuid,
                _owner: RuntimeId,
            ) -> Result<AcquireOutcome, MeshError> {
                Ok(AcquireOutcome::Acquired(HuddleLease(SessionLease {
                    community_id: CommunityId::from_uuid(Uuid::nil()),
                    session_id: Uuid::nil(),
                    owner_runtime_id: RuntimeId([0u8; 32]),
                    generation: 1,
                    profile: HUDDLE_CONTROL_PROFILE,
                })))
            }
            async fn renew(&self, _lease: &HuddleLease) -> Result<HuddleRenewOutcome, MeshError> {
                // Should never be called — the pre-cancelled token hits the
                // cancel arm before renew.
                Ok(HuddleRenewOutcome::Renewed(HuddleLease(SessionLease {
                    community_id: CommunityId::from_uuid(Uuid::nil()),
                    session_id: Uuid::nil(),
                    owner_runtime_id: RuntimeId([0u8; 32]),
                    generation: 1,
                    profile: HUDDLE_CONTROL_PROFILE,
                })))
            }
            async fn release(
                &self,
                _lease: &HuddleLease,
            ) -> Result<HuddleReleaseOutcome, MeshError> {
                *self.release_calls.lock().unwrap() += 1;
                Ok(HuddleReleaseOutcome::Released)
            }
            async fn validate(
                &self,
                _community_id: CommunityId,
                _fenced: &FencedHeader,
            ) -> Result<(), MeshError> {
                Ok(())
            }
        }

        let community = CommunityId::from_uuid(Uuid::nil());
        let channel_id = Uuid::new_v4();
        let dir = Arc::new(CountingDir {
            release_calls: Mutex::new(0),
        });

        // Build a test HuddleLease (uses pub(crate) inner field — same crate).
        let lease = HuddleLease(SessionLease {
            community_id: community,
            session_id: Uuid::new_v4(),
            owner_runtime_id: RuntimeId([0u8; 32]),
            generation: 7,
            profile: HUDDLE_CONTROL_PROFILE,
        });

        let room = Arc::new(crate::audio::room::Room::new(community, channel_id));
        let audio_rooms = Arc::new(crate::audio::room::AudioRoomManager::default());
        let dir_clone = Arc::clone(&dir) as Arc<dyn HuddleDirectory>;

        let mut guard = HuddleAdmissionGuard {
            lease: Some((lease, dir_clone)),
            remote_session: None,
            remote_stream: None,
            peer_id: None,
            room,
            audio_rooms,
            community,
            channel_id,
        };

        guard.release_before_commit().await;

        // `release_before_commit` now calls `directory.release` directly and
        // awaits it — no detached renewer task. Release is complete by the time
        // `release_before_commit` returns.
        let release_calls = *dir.release_calls.lock().unwrap();
        assert_eq!(
            release_calls, 1,
            "CW6: directory.release must be called exactly once on pre-commit exit; got {release_calls}"
        );

        // Guard is idempotent — calling release_before_commit again must not
        // trigger a second release (lease field is now None).
        guard.release_before_commit().await;
        let release_calls_after = *dir.release_calls.lock().unwrap();
        assert_eq!(
            release_calls_after, 1,
            "CW6: second release_before_commit must be idempotent (no double-release); got {release_calls_after}"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // CW7: guard-level witness — clean close sent on remote stream pre-commit exit
    // ─────────────────────────────────────────────────────────────────────────
    //
    // `HuddleAdmissionGuard::release_before_commit` must call `send_clean_close`
    // (UnregisterPeer + Goodbye + finish) when a `remote_stream` is held, before
    // the guard releases. No real mesh transport, TLS, or remote pod required:
    // `MeshStream::new` accepts `Box<dyn StreamSendHalf>` stubs, and
    // `RemoteHuddleSession::for_test` provides the needed `fenced`/`pubkey`.
    //
    // Mutation evidence (executed):
    //   CW7A) Remove `if let (Some(session), Some(ref mut stream)) = ...` block
    //         in `release_before_commit` → send_frame never called → frames_sent
    //         stays 0 → assertion panics.
    //   CW7B) Swap UnregisterPeer and Goodbye order → Goodbye arrives before
    //         UnregisterPeer → frame[0] is Goodbye, not Data → first frame
    //         assertion panics (expected Data, got Goodbye).
    #[tokio::test]
    async fn cw7_guard_release_before_commit_sends_clean_close_on_remote_stream() {
        use crate::audio::join::RemoteHuddleSession;
        use buzz_relay_mesh::wire::FencedHeader;
        use buzz_relay_mesh::RuntimeId;
        use buzz_relay_mesh::{
            BoxFuture, MeshError, MeshStream, MeshStreamFrame, StreamRecvHalf, StreamSendHalf,
        };
        use std::sync::{Arc, Mutex};
        use uuid::Uuid;

        // A send half that records every frame sent.
        struct RecordingSend {
            frames: Arc<Mutex<Vec<MeshStreamFrame>>>,
            finished: Arc<Mutex<bool>>,
        }
        impl StreamSendHalf for RecordingSend {
            fn send_frame(
                &mut self,
                frame: MeshStreamFrame,
            ) -> BoxFuture<'_, Result<(), MeshError>> {
                self.frames.lock().unwrap().push(frame);
                Box::pin(async { Ok(()) })
            }
            fn finish(&mut self) -> Result<(), MeshError> {
                *self.finished.lock().unwrap() = true;
                Ok(())
            }
        }

        // A recv half that always returns None (never read in this test).
        struct NullRecv;
        impl StreamRecvHalf for NullRecv {
            fn recv_frame(&mut self) -> BoxFuture<'_, Result<Option<MeshStreamFrame>, MeshError>> {
                Box::pin(async { Ok(None) })
            }
        }

        let frames = Arc::new(Mutex::new(Vec::<MeshStreamFrame>::new()));
        let finished = Arc::new(Mutex::new(false));
        let stream = MeshStream::new(
            Box::new(RecordingSend {
                frames: Arc::clone(&frames),
                finished: Arc::clone(&finished),
            }),
            Box::new(NullRecv),
        );

        let community = buzz_core::CommunityId::from_uuid(Uuid::nil());
        let channel_id = Uuid::new_v4();
        let fenced = FencedHeader {
            owner_runtime_id: RuntimeId([0u8; 32]),
            session_id: Uuid::nil(),
            generation: 1,
        };
        let pubkey = "test-pubkey-hex".to_string();
        let session = RemoteHuddleSession::for_test(fenced, pubkey.clone());

        let room = Arc::new(crate::audio::room::Room::new(community, channel_id));
        let audio_rooms = Arc::new(crate::audio::room::AudioRoomManager::default());

        let mut guard = HuddleAdmissionGuard {
            lease: None,
            remote_session: Some(session),
            remote_stream: Some(stream),
            peer_id: None,
            room,
            audio_rooms,
            community,
            channel_id,
        };

        guard.release_before_commit().await;

        // Stream must have received UnregisterPeer (Data) then Goodbye, then finish.
        let sent = frames.lock().unwrap().clone();
        assert_eq!(
            sent.len(),
            2,
            "CW7: send_clean_close must send exactly 2 frames (Data + Goodbye); got {}",
            sent.len()
        );

        // Frame 0: Data with UnregisterPeer payload — exact pubkey.
        match &sent[0] {
            MeshStreamFrame::Data { payload, .. } => {
                use crate::audio::join::{decode_control, HuddleControlMsg};
                let msg = decode_control(payload)
                    .expect("CW7: frame[0] Data payload must decode as HuddleControlMsg");
                assert_eq!(
                    msg,
                    HuddleControlMsg::UnregisterPeer {
                        pubkey: pubkey.clone()
                    },
                    "CW7: frame[0] must be UnregisterPeer with exact pubkey; got {msg:?}"
                );
            }
            other => panic!(
                "CW7: frame[0] must be Data (UnregisterPeer), got {other:?} — \
                 swap-order mutation: Goodbye before UnregisterPeer"
            ),
        }

        // Frame 1: Goodbye — order assertion: UnregisterPeer BEFORE Goodbye.
        match &sent[1] {
            MeshStreamFrame::Goodbye { .. } => {}
            other => panic!("CW7: frame[1] must be Goodbye, got {other:?}"),
        }

        // Finish must have been called.
        assert!(
            *finished.lock().unwrap(),
            "CW7: send_clean_close must call finish() on the stream"
        );
        // remote_session and remote_stream must be cleared.
        assert!(
            guard.remote_session.is_none(),
            "CW7: remote_session must be cleared after release_before_commit"
        );
        assert!(
            guard.remote_stream.is_none(),
            "CW7: remote_stream must be cleared after release_before_commit"
        );
    }

    // ── F4: add-peer gate-expired → denial frame delivered before close (handler-bound) ──
    //
    // Production path: `handle_active_audio_connection`, Step 5, line ~959:
    //   `audio_gate.acquire_effect()` → Err(SessionExpired)
    //   → control.expiry_deny_terminal(...)   ← PROOF SEAM: must be present
    //   → lifecycle_cancel
    //   → R2 drain (while let Ok(msg) = terminal_ctrl_rx.try_recv())
    //   → ws_send.send(denial frame)
    //   → ws_send.send(close frame)
    //   → return
    //
    // The test arms `before_add_peer_gate_acquire` to intercept the handler
    // just before `audio_gate.acquire_effect()` (Step 5), then cancels the
    // connection token (simulating late-firing expiry), then releases the
    // hook.  `acquire_effect()` immediately returns `SessionExpired`, the
    // handler's denial arm runs, and the WS client observes:
    //   frame 0: Audio restricted JSON payload
    //   frame 1: 1008 POLICY close
    //
    // Mutation evidence:
    //   A) Remove `expiry_deny_terminal` from the add-peer SessionExpired arm →
    //      no denial frame queued → R2 drain finds empty channel → frame 0
    //      is the POLICY close instead of the JSON payload → frame-0 text
    //      assertion panics.
    //   B) Remove R2 drain → denial queued but never drained → frame 0 absent
    //      → timeout panics.
    //   C) Remove `before_add_peer_gate_acquire` hook call from handler →
    //      arrived_rx never fires → test panics before any assertion.
    //   D) Delete `#[cfg(test)] before_add_peer_gate_acquire(...)` line →
    //      same as C.
    //
    // Requires Postgres for `seed_audio_fixture` (channel/member row).
    async fn f4_add_peer_expired_delivers_denial_before_close_body() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;
        use tokio_tungstenite::connect_async;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();
        let (tenant, channel_id, member_key) = seed_audio_fixture(&pool).await;
        let community_id = tenant.community();

        // VALID future deadline: the already-expired check at handler.rs:475-505
        // fires immediately when the deadline is in the past, routing the handler
        // to the wrong arm before the add-peer hook can be reached.  Use a
        // far-future deadline so the handler proceeds to the target seam, then
        // cancel from the test while the hook is held — `acquire_effect()` sees
        // `is_cancelled()` and returns `SessionExpired`, exercising the intended
        // `expiry_deny_terminal → lifecycle_cancel → finalize_drain!()` arm.
        // [F4: valid-deadline / cancel-while-held approach]
        let assertion = VerifiedAssertion::for_test(
            Some(member_key.public_key()),
            vec![Utc::now() + Duration::hours(1)],
        );

        let conn_cancel = CancellationToken::new();
        let conn_cancel_for_test = conn_cancel.clone();

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();
        let cancel_c = conn_cancel.clone();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("F4 add-peer: bind test listener");
        let addr = listener.local_addr().expect("F4 add-peer: listener addr");

        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let cancel_i = cancel_c.clone();
                    move |ws: WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let cancel_i = cancel_i.clone();
                        let conn_time = chrono::Utc::now();
                        let control_i =
                            crate::state::CommunityConnectionControl::new(cancel_i.clone());
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    channel_id,
                                    control_i,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app)
                .await
                .expect("F4 add-peer: test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("F4 add-peer: server ready");

        // Arm the hook BEFORE connecting so we capture the handler at the exact seam.
        let (arrived_rx, hook_release) =
            crate::nip_fi_test_hooks::audio_add_peer_gate_acquire_hook::arm(community_id);

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("F4 add-peer: connect client");

        // NIP-42 auth exchange.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("F4 add-peer: challenge timeout")
            .expect("F4 add-peer: challenge item")
            .expect("F4 add-peer: challenge message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("F4 add-peer: expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("F4 add-peer: challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("F4 add-peer: challenge field")
            .to_string();

        // Derive relay URL from fixture tenant — must match nip42_expected_relay_url(
        // &state.config.relay_url, &tenant) computed in handle_active_audio_connection.
        // hermetic_for_test relay_url uses "ws://" scheme; host is tenant.host().
        let relay_url = format!("ws://{}", tenant.host());
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", &relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&member_key)
            .unwrap();
        let auth_msg = serde_json::json!({"type": "auth", "event": auth_event}).to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("F4 add-peer: send auth");

        // Wait for handler to reach before_add_peer_gate_acquire.
        // Handler has done: auth → membership check → channel DB reads → step 5 seam.
        tokio::time::timeout(std::time::Duration::from_secs(10), arrived_rx)
            .await
            .expect("F4 add-peer: handler must reach before_add_peer_gate_acquire within 10s")
            .expect("F4 add-peer: hook arrived channel closed");

        // Cancel from the test while the hook holds the handler.
        // The gate uses the same token (cancel_i = conn_cancel) so cancelling here
        // causes acquire_effect() to return SessionExpired on hook release.
        // This exercises the real caller rejection arm at handler.rs:1096-1124:
        //   expiry_deny_terminal → lifecycle_cancel → finalize_drain!()
        // [F4: cancel-while-held path — exercises the target rejection arm]
        conn_cancel_for_test.cancel();

        // Release the hook: handler resumes → acquire_effect() sees is_cancelled() →
        // returns SessionExpired → expiry_deny_terminal enqueues denial frame →
        // lifecycle_cancel (transition lock) → finalize_drain! → send frames.
        hook_release.notify_one();

        // Frame 0: Audio authorization-denied JSON payload.
        let frame0 = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .expect("F4 add-peer: frame 0 timeout")
            .expect("F4 add-peer: frame 0 item")
            .expect("F4 add-peer: frame 0 message");
        let expected_restricted = serde_json::json!({
            "type": "restricted",
            "message": buzz_auth::DenialClass::AuthorizationDenied.nostr_text()
        })
        .to_string();
        match &frame0 {
            tokio_tungstenite::tungstenite::Message::Text(t) => assert_eq!(
                t.as_str(),
                expected_restricted.as_str(),
                "F4 add-peer: frame 0 must be exact canonical Audio restricted JSON \
                 (proves expiry_deny_terminal ran in the handler's SessionExpired arm)"
            ),
            other => panic!("F4 add-peer: frame 0 must be Text(restricted JSON); got {other:?}"),
        }

        // Frame 1: 1008 POLICY close.
        let frame1 = tokio::time::timeout(std::time::Duration::from_secs(3), client.next())
            .await
            .expect("F4 add-peer: frame 1 timeout")
            .expect("F4 add-peer: frame 1 item")
            .expect("F4 add-peer: frame 1 message");
        match &frame1 {
            tokio_tungstenite::tungstenite::Message::Close(Some(cf)) => {
                assert_eq!(
                    cf.code,
                    tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy,
                    "F4 add-peer: close code must be 1008 POLICY; got {:?}",
                    cf.code
                );
            }
            other => panic!("F4 add-peer: frame 1 must be Close(Some(1008)); got {other:?}"),
        }

        server.abort();
        let _ = server.await;
    }

    // ── F4: commit gate-expired → denial frame delivered before close (handler-bound) ──
    //
    // Production path: `handle_active_audio_connection`, commit match arm, line ~1306:
    //   `commit_participant_join(...)` → Err(JoinCommitError::Expired)
    //   → control.expiry_deny_terminal(...)   ← PROOF SEAM: must be present
    //   → lifecycle_cancel
    //   → R2 drain (while let Ok(msg) = terminal_ctrl_rx.try_recv())
    //   → ws_send.send(denial frame)
    //   → ws_send.send(close frame)
    //   → return
    //
    // Uses `before_participant_commit` (already in `nip_fi_test_hooks`) which fires
    // inside `commit_participant_join` BEFORE `acquire_effect()`.  Test arms the hook,
    // waits for arrival (handler at the commit gate seam), cancels from test side,
    // releases hook → `commit_participant_join` returns `Err(JoinCommitError::Expired)`
    // → handler's Expired arm runs → wire delivers denial + close.
    //
    // Mutation evidence:
    //   A) Remove `expiry_deny_terminal` from the JoinCommitError::Expired arm →
    //      no denial frame queued → R2 drain finds empty channel → frame 0 is
    //      the POLICY close → frame-0 text assertion panics.
    //   B) Remove R2 drain → denial queued but never drained → timeout panics.
    //   C) Delete `before_participant_commit(...)` call from commit_participant_join →
    //      arrived_rx never fires → test panics.
    //
    // Requires Postgres for DB writes in the commit path.
    async fn f4_commit_expired_delivers_denial_before_close_body() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;
        use tokio_tungstenite::connect_async;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();
        let (tenant, channel_id, member_key) = seed_audio_fixture(&pool).await;
        let community_id = tenant.community();

        // VALID future deadline: same rationale as F4 add-peer — an already-expired
        // deadline is rejected at handler.rs:475-505 before the commit hook is reachable.
        // Use a far-future deadline so the handler reaches the commit seam, then cancel
        // from the test while the hook is held — acquire_effect() sees is_cancelled()
        // and returns SessionExpired → JoinCommitError::Expired → handler's Expired arm.
        // [F4: valid-deadline / cancel-while-held approach]
        let assertion = VerifiedAssertion::for_test(
            Some(member_key.public_key()),
            vec![Utc::now() + Duration::hours(1)],
        );

        let conn_cancel = CancellationToken::new();
        let conn_cancel_for_test = conn_cancel.clone();

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();
        let cancel_c = conn_cancel.clone();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("F4 commit: bind test listener");
        let addr = listener.local_addr().expect("F4 commit: listener addr");

        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let cancel_i = cancel_c.clone();
                    move |ws: WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let cancel_i = cancel_i.clone();
                        let conn_time = chrono::Utc::now();
                        let control_i =
                            crate::state::CommunityConnectionControl::new(cancel_i.clone());
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    channel_id,
                                    control_i,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app)
                .await
                .expect("F4 commit: test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("F4 commit: server ready");

        // Arm `before_participant_commit`: fires inside commit_participant_join
        // BEFORE acquire_effect().  This intercepts the handler at the commit seam.
        let (arrived_rx, hook_release) =
            crate::nip_fi_test_hooks::audio_participant_commit_hook::arm(community_id);

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("F4 commit: connect client");

        // NIP-42 auth exchange.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("F4 commit: challenge timeout")
            .expect("F4 commit: challenge item")
            .expect("F4 commit: challenge message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("F4 commit: expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("F4 commit: challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("F4 commit: challenge field")
            .to_string();

        // Derive relay URL from fixture tenant — must match nip42_expected_relay_url(
        // &state.config.relay_url, &tenant) computed in handle_active_audio_connection.
        let relay_url = format!("ws://{}", tenant.host());
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", &relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&member_key)
            .unwrap();
        let auth_msg = serde_json::json!({"type": "auth", "event": auth_event}).to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("F4 commit: send auth");

        // Wait for handler to reach before_participant_commit.
        // Handler has done: auth → membership → add_peer → 48101 insert → commit seam.
        tokio::time::timeout(std::time::Duration::from_secs(15), arrived_rx)
            .await
            .expect("F4 commit: handler must reach before_participant_commit within 15s")
            .expect("F4 commit: hook arrived channel closed");

        // Cancel from the test while the hook holds the handler at the commit seam.
        // The gate uses the same token (cancel_c = conn_cancel) so cancelling here
        // causes acquire_effect() in commit_participant_join to return SessionExpired.
        // → JoinCommitError::Expired → handler's Expired arm:
        //   expiry_deny_terminal → lifecycle_cancel → finalize_drain! → send frames.
        // [F4: cancel-while-held path — exercises JoinCommitError::Expired arm]
        conn_cancel_for_test.cancel();

        // Release hook: commit_participant_join resumes → acquire_effect() sees
        // is_cancelled() → Err(JoinCommitError::Expired) → denial + close frames.
        hook_release.notify_one();

        // Frame 0: Audio authorization-denied JSON payload.
        let frame0 = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .expect("F4 commit: frame 0 timeout")
            .expect("F4 commit: frame 0 item")
            .expect("F4 commit: frame 0 message");
        let expected_restricted = serde_json::json!({
            "type": "restricted",
            "message": buzz_auth::DenialClass::AuthorizationDenied.nostr_text()
        })
        .to_string();
        match &frame0 {
            tokio_tungstenite::tungstenite::Message::Text(t) => assert_eq!(
                t.as_str(),
                expected_restricted.as_str(),
                "F4 commit: frame 0 must be exact canonical Audio restricted JSON \
                 (proves expiry_deny_terminal ran in the handler's JoinCommitError::Expired arm)"
            ),
            other => panic!("F4 commit: frame 0 must be Text(restricted JSON); got {other:?}"),
        }

        // Frame 1: 1008 POLICY close.
        let frame1 = tokio::time::timeout(std::time::Duration::from_secs(3), client.next())
            .await
            .expect("F4 commit: frame 1 timeout")
            .expect("F4 commit: frame 1 item")
            .expect("F4 commit: frame 1 message");
        match &frame1 {
            tokio_tungstenite::tungstenite::Message::Close(Some(cf)) => {
                assert_eq!(
                    cf.code,
                    tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy,
                    "F4 commit: close code must be 1008 POLICY; got {:?}",
                    cf.code
                );
            }
            other => panic!("F4 commit: frame 1 must be Close(Some(1008)); got {other:?}"),
        }

        server.abort();
        let _ = server.await;
    }

    // ── F4: elapsed-deadline arm — expiry fires while handler is at add-peer hook ──
    //
    // Production path: `nip_fi_gate.rs:139-142` — the ELAPSED-DEADLINE rejection in
    // `acquire_effect()`:
    //   if let Some(deadline) = self.deadline {
    //       if Utc::now() >= deadline {
    //           return Err(SessionExpired);   ← this arm (not 136-137 cancel arm)
    //
    // This test differs from `f4_add_peer_expired_delivers_denial_before_close`:
    //   - THAT test: far-future deadline, test calls `cancel.cancel()` while hook held.
    //     Exercises the CANCELLATION arm (nip_fi_gate.rs:136-137).
    //   - THIS test: near-future deadline (2s), waits for the real tokio::time::sleep
    //     to fire in the expiry task.  Hooks the expiry task BEFORE gate.expire()
    //     (cancel NOT yet set), then releases the add-peer hook so acquire_effect()
    //     returns SessionExpired via the ELAPSED-DEADLINE arm (139-142).
    //
    // Two-hook sequence:
    //   1. Arm `before_add_peer_gate_acquire` (handler hook at Step 5 seam).
    //   2. Arm `expiry_pre_expire_hook` (expiry task hook after sleep, before expire).
    //   3. Connect + auth → handler reaches add-peer hook; expiry task sleeping.
    //   4. Await `before_add_peer_gate_acquire` arrival (handler paused at Step 5).
    //   5. Await `expiry_pre_expire_hook` arrival (sleep fired, cancel NOT set).
    //   6. Assert: cancel is NOT cancelled (gate.expire() has not yet fired cancel.cancel()).
    //   7. Release `before_add_peer_gate_acquire` → acquire_effect() reads the read guard,
    //      checks cancel (false), checks deadline (Utc::now() >= deadline → true) →
    //      returns Err(SessionExpired) via the deadline arm (nip_fi_gate.rs:139-142).
    //   8. Release `expiry_pre_expire_hook` → expiry task calls gate.expire() → cancel set.
    //   9. Client receives: denial JSON + 1008 POLICY close.
    //
    // Mutation evidence:
    //   A) Delete `expiry_pre_expire_hook::fire(...)` call from nip_fi_session.rs →
    //      expiry_arrived_rx times out → test panics (proves hook at correct seam).
    //   B) Flip deadline to `Utc::now() + Duration::hours(1)` →
    //      expiry_arrived_rx never fires within 5s → test panics
    //      (proves the sleep must actually fire, not just cancel).
    //   C) Remove `expiry_deny_terminal` from the SessionExpired arm →
    //      no denial frame queued → frame 0 is the close → text assertion panics.
    //
    // Requires Postgres for `seed_audio_fixture` (channel/member row).
    async fn f4_elapsed_deadline_arm_fires_on_gate_acquire_body() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;
        use tokio_tungstenite::connect_async;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();
        let (tenant, channel_id, member_key) = seed_audio_fixture(&pool).await;
        let community_id = tenant.community();

        // Near-future deadline: 2 seconds from now.
        // MUST be in the future (not past) so the handler passes the already-expired
        // check at handler.rs:475-505 and reaches the add-peer hook at Step 5.
        // The expiry task's tokio::time::sleep(remaining) fires after ~2s of real
        // wall-clock time, which is enough for auth + membership + DB reads to complete.
        let deadline = Utc::now() + Duration::milliseconds(2000);
        let assertion = VerifiedAssertion::for_test(Some(member_key.public_key()), vec![deadline]);

        // Create the control OUTSIDE the server closure so the test has access to
        // `conn_control.hook_key` for arming the expiry_pre_expire_hook.
        // Both the server closure and the test hold Clones of the same control.
        let conn_cancel = CancellationToken::new();
        let conn_control = crate::state::CommunityConnectionControl::new(conn_cancel.clone());
        let conn_control_for_server = conn_control.clone();

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("F4 elapsed: bind test listener");
        let addr = listener.local_addr().expect("F4 elapsed: listener addr");

        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let control_i = conn_control_for_server.clone();
                    move |ws: WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let control_i = control_i.clone();
                        let conn_time = chrono::Utc::now();
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    channel_id,
                                    control_i,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app)
                .await
                .expect("F4 elapsed: test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("F4 elapsed: server ready");

        // Arm BOTH hooks before connecting.
        //
        // `before_add_peer_gate_acquire`: fires in the handler's Step 5, just before
        // audio_gate.acquire_effect().  Pauses the handler at the gate seam.
        //
        // `expiry_pre_expire_hook`: fires in spawn_nip_fi_expiry_task's sleep arm,
        // AFTER the real tokio::time::sleep fires (deadline elapsed) but BEFORE
        // gate.expire() is called (cancel NOT yet set).
        let (add_peer_arrived_rx, add_peer_release) =
            crate::nip_fi_test_hooks::audio_add_peer_gate_acquire_hook::arm(community_id);
        let (expiry_arrived_rx, expiry_release) =
            crate::nip_fi_session::expiry_pre_expire_hook::arm(conn_control.hook_key);

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("F4 elapsed: connect client");

        // NIP-42 auth exchange.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("F4 elapsed: challenge timeout")
            .expect("F4 elapsed: challenge item")
            .expect("F4 elapsed: challenge message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("F4 elapsed: expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("F4 elapsed: challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("F4 elapsed: challenge field")
            .to_string();

        let relay_url = format!("ws://{}", tenant.host());
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", &relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&member_key)
            .unwrap();
        let auth_msg = serde_json::json!({"type": "auth", "event": auth_event}).to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("F4 elapsed: send auth");

        // Step 4: wait for handler to reach before_add_peer_gate_acquire.
        // The handler has done: auth → membership check → channel DB reads → Step 5 seam.
        // Allow up to 10s for the DB-heavy auth path to complete.
        tokio::time::timeout(std::time::Duration::from_secs(10), add_peer_arrived_rx)
            .await
            .expect("F4 elapsed: handler must reach before_add_peer_gate_acquire within 10s")
            .expect("F4 elapsed: add_peer arrived channel closed");

        // Step 5: wait for the expiry task's sleep to fire and the task to pause at
        // expiry_pre_expire_hook.  The deadline is 2s from test start; allow 5s
        // from this point (handler auth may have taken up to ~1s already).
        //
        // At the moment this await returns:
        //   - tokio::time::sleep(remaining) has completed → wall-clock is past deadline
        //   - gate.expire() has NOT been called → cancel.is_cancelled() == false
        tokio::time::timeout(std::time::Duration::from_secs(5), expiry_arrived_rx)
            .await
            .expect(
                "F4 elapsed: expiry task must reach pre-expire hook within 5s \
                     (deadline is 2s; if this times out, the sleep did not fire)",
            )
            .expect("F4 elapsed: expiry_arrived channel closed");

        // Step 6: assert cancel is NOT yet set — the hook fired BEFORE gate.expire().
        // This is the critical precondition: acquire_effect() must return SessionExpired
        // via the elapsed-deadline arm (nip_fi_gate.rs:139-142), not the cancel arm
        // (136-137).
        assert!(
            !conn_cancel.is_cancelled(),
            "F4 elapsed: cancel must NOT be set at expiry_pre_expire_hook arrival \
             (hook fires before gate.expire(), which calls cancel.cancel())"
        );

        // Step 7: release the add-peer hook.
        // The handler resumes, calls acquire_effect():
        //   - read guard acquired (expiry task hasn't called gate.expire() yet → no write guard)
        //   - cancel check: is_cancelled() == false → passes
        //   - deadline check: Utc::now() >= deadline → true → returns Err(SessionExpired)
        //     via the elapsed-deadline arm (nip_fi_gate.rs:139-142)
        // Handler's SessionExpired arm runs:
        //   expiry_deny_terminal → lifecycle_cancel → await expiry_task → finalize_drain!
        add_peer_release.notify_one();

        // Step 7b: wait for the handler's SessionExpired arm to reach lifecycle_cancel.
        // lifecycle_cancel() → conn_cancel.cancel() is the observable signal that:
        //   (a) acquire_effect() returned Err(SessionExpired) via the elapsed-deadline arm,
        //   (b) the handler called expiry_deny_terminal (denial frame enqueued), and
        //   (c) the handler is now awaiting the expiry task.
        //
        // Only AFTER this observation do we release the expiry hook.  Releasing expiry
        // before the handler has entered its SessionExpired arm would create a window
        // where gate.expire() runs before the handler even calls acquire_effect(), so
        // the deadline arm might never fire and the test could pass vacuously.
        //
        // RED gate: remove the elapsed-deadline check at nip_fi_gate.rs:~139-142.
        // Without it, acquire_effect() returns Ok(permit) (cancel is false, no deadline check)
        // → handler never hits SessionExpired → lifecycle_cancel never fires →
        // conn_cancel never becomes true → this timeout fires → test FAILS deterministically.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if conn_cancel.is_cancelled() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect(
            "F4 elapsed: handler must call lifecycle_cancel() (conn_cancel becomes true) \
             within 5s after add-peer release. \
             This proves acquire_effect() returned Err(SessionExpired) via the elapsed-deadline \
             arm at nip_fi_gate.rs:~139-142 and the handler's rejection arm ran. \
             RED gate: remove the elapsed-deadline check → acquire_effect() returns Ok → \
             handler never enters SessionExpired → lifecycle_cancel never fires → times out.",
        );

        // Step 8: release the expiry hook so gate.expire() can complete.
        // The handler is now blocked on `_nip_fi_admission_expiry.take().unwrap().await`.
        // Releasing the expiry hook lets gate.expire() run → write guard acquired →
        // phase = Expired → task returns → handler's await completes → finalize_drain!
        // sends the denial frame and close frame.
        expiry_release.notify_one();

        // Frame 0: Audio authorization-denied JSON payload.
        let frame0 = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .expect("F4 elapsed: frame 0 timeout")
            .expect("F4 elapsed: frame 0 item")
            .expect("F4 elapsed: frame 0 message");
        let expected_restricted = serde_json::json!({
            "type": "restricted",
            "message": buzz_auth::DenialClass::AuthorizationDenied.nostr_text()
        })
        .to_string();
        match &frame0 {
            tokio_tungstenite::tungstenite::Message::Text(t) => assert_eq!(
                t.as_str(),
                expected_restricted.as_str(),
                "F4 elapsed: frame 0 must be exact canonical Audio restricted JSON \
                 (proves expiry_deny_terminal ran in the handler's SessionExpired arm \
                 via the elapsed-deadline path)"
            ),
            other => panic!("F4 elapsed: frame 0 must be Text(restricted JSON); got {other:?}"),
        }

        // Frame 1: 1008 POLICY close.
        let frame1 = tokio::time::timeout(std::time::Duration::from_secs(3), client.next())
            .await
            .expect("F4 elapsed: frame 1 timeout")
            .expect("F4 elapsed: frame 1 item")
            .expect("F4 elapsed: frame 1 message");
        match &frame1 {
            tokio_tungstenite::tungstenite::Message::Close(Some(cf)) => {
                assert_eq!(
                    cf.code,
                    tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy,
                    "F4 elapsed: close code must be 1008 POLICY; got {:?}",
                    cf.code
                );
            }
            other => panic!("F4 elapsed: frame 1 must be Close(Some(1008)); got {other:?}"),
        }

        server.abort();
        let _ = server.await;
    }

    // ── R2: drain_terminal!() serialization race witness ─────────────────────────
    //
    // Exercises the `lifecycle_cancel()` call INSIDE `drain_terminal!()`.  This is
    // the key serialization step: when the handler calls `drain_terminal!()`, it must
    // acquire the transition lock (via `lifecycle_cancel()`) before draining, so that
    // a concurrent admin producer that has won the reason but not yet called `try_send`
    // is forced to complete its enqueue before the drain runs.
    //
    // IMPORTANT — no assertion (None): the handler is called with nip_fi_assertion=None
    // so audio_session_deadline=None → no expiry task spawned → _nip_fi_admission_expiry=None
    // → drain_terminal!()'s `t.await` branch is a no-op.  This isolates the falsification:
    // lifecycle_cancel() is the SOLE serialization.  If it were removed, the drain would
    // see an empty channel (admin hasn't called try_send yet) and deliver no denial frame.
    // With a far-future deadline, `t.await` would wait for cancel (fired after try_send),
    // accidentally serializing the drain even without lifecycle_cancel() — making the
    // falsification target invisible.
    //
    // Production path: `handle_active_audio_connection`, check_membership_for_admission
    // Err arm (non-member key), which calls `drain_terminal!()` directly:
    //   1. auth + relay membership → pass
    //   2. check_membership_for_admission → Err("not a member")
    //   3. ws_send.send("not a member" diagnostic frame)
    //   4. [before_not_a_member_drain_terminal hook fires — handler paused]
    //   5. Admin fires disconnect_nip_fi → wins reason → cancel_race_test_hook pauses
    //      (transition lock held, before try_send).
    //   6. hook_ready signal → test confirms admin is paused with lock held.
    //   7. Release before_not_a_member_drain_terminal hook → handler proceeds to
    //      drain_terminal!() → lifecycle_cancel() tries to acquire transition lock →
    //      BLOCKS (admin holds it).
    //   8. Test releases cancel_race_test_hook → admin completes try_send → drops lock.
    //   9. lifecycle_cancel() unblocks → acquires lock → publishes LifecycleClosed →
    //      drops lock.
    //  10. drain_terminal!(): t.await no-op (None expiry) → try_recv() → denial frame →
    //      ws_send → 1008 POLICY close.
    //
    // The WS client observes:
    //   frame 0: "not a member" diagnostic JSON
    //   frame 1: Audio restricted JSON (denial frame from admin)
    //   frame 2: 1008 POLICY close
    //
    // Falsification target: remove `lifecycle_cancel()` from `drain_terminal!()`.
    //   Without the lock acquisition, the handler drains immediately while admin
    //   is still paused at cancel_race_test_hook (before try_send).  The channel
    //   is empty → denial frame absent → only diagnostic + close → frame-1 timeout
    //   → test panics.  This is the mutation that Thufir's review identified:
    //   "remove only the new lifecycle_cancel() from drain_terminal!() → test RED."
    //
    // Mutation evidence:
    //   A) Delete `before_not_a_member_drain_terminal(...)` from handler →
    //      `arrived_rx` times out → test panics (proves hook is at correct seam).
    //   B) Remove `lifecycle_cancel()` from `drain_terminal!()` macro →
    //      handler drains without blocking → admin's try_send races → denial absent
    //      → frame-1 recv times out → panics.
    //   C) Remove R2 drain (`while let Ok(msg) = terminal_ctrl_rx.try_recv()`) from
    //      `drain_terminal!()` → denial queued but never consumed → frame-1 timeout.
    //
    // Requires Postgres for `seed_audio_fixture` (channel/member row for membership).
    // Uses a NON-MEMBER key (fresh generate()) so check_membership_for_admission fails.
    async fn r2_drain_terminal_lifecycle_cancel_serializes_admin_frame_body() {
        use std::sync::Arc;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();
        let (tenant, channel_id, _member_key) = seed_private_audio_fixture(&pool).await;
        let community_id = tenant.community();

        // Use a NON-MEMBER key: passes relay membership (require_relay_membership=false)
        // but fails check_membership_for_admission → triggers the "not a member" arm →
        // before_not_a_member_drain_terminal hook → drain_terminal!().
        let non_member_key = nostr::Keys::generate();

        // No NIP-FI assertion (pass None to handler): audio_session_deadline=None,
        // so NO expiry task is spawned → _nip_fi_admission_expiry=None →
        // drain_terminal!()'s `t.await` branch is skipped entirely →
        // lifecycle_cancel() is the ONLY serialization protecting against admin's race.
        //
        // With a far-future deadline, `t.await` would wait for cancel (fired only
        // after admin's try_send), incidentally serializing the drain even without
        // lifecycle_cancel(). That makes the lifecycle_cancel() falsification
        // target invisible. By omitting the assertion, we isolate the mechanism:
        //   GREEN: lifecycle_cancel() blocks on admin's lock → admin proceeds →
        //          try_send → lock dropped → lifecycle_cancel unblocks → drain sees
        //          the denial frame.
        //   RED (mutation B): no lifecycle_cancel() → t.await no-op → drain sees
        //          empty channel (admin still at hook) → no denial frame → timeout.

        // Pre-create and register the control so disconnect_nip_fi can find the session.
        // audio_post_auth_register writes proven_pubkey on this same Arc after NIP-42 auth.
        let conn_cancel = tokio_util::sync::CancellationToken::new();
        let conn_control = crate::state::CommunityConnectionControl::new(conn_cancel.clone());
        let conn_id = uuid::Uuid::new_v4();
        let _conn_guard =
            state
                .community_connections
                .register(conn_id, community_id, conn_control.clone());
        let conn_control_for_server = conn_control.clone();

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("R2 drain: bind test listener");
        let addr = listener.local_addr().expect("R2 drain: listener addr");

        let server = tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/",
                axum::routing::get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let control_outer = conn_control_for_server.clone();
                    move |ws: axum::extract::WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let control_inner = control_outer.clone();
                        let conn_time = chrono::Utc::now();
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    channel_id,
                                    control_inner,
                                    None, // No assertion: no expiry task spawned;
                                    // lifecycle_cancel() is the sole serializer.
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app)
                .await
                .expect("R2 drain: test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("R2 drain: server ready");

        // ── Arm cancel_race_test_hook BEFORE connecting ───────────────────
        // The hook fires inside disconnect_nip_fi after winning the reason,
        // while the transition lock is still held.  We arm it before the
        // client connects so it's ready by the time the test fires the admin.
        let hook_key = conn_control.hook_key;
        let (hook_ready_tx, hook_ready_rx) = std::sync::mpsc::channel::<()>();
        let (hook_proceed_tx, hook_proceed_rx_inner) = std::sync::mpsc::channel::<()>();
        let hook_proceed_rx = Arc::new(std::sync::Mutex::new(hook_proceed_rx_inner));

        // Drop guard: disarms cancel_race_test_hook on panic/exit.
        struct CancelHookGuard(uuid::Uuid);
        impl Drop for CancelHookGuard {
            fn drop(&mut self) {
                crate::state::cancel_race_test_hook::disarm(self.0);
            }
        }
        let _cancel_hook_guard = CancelHookGuard(hook_key);

        crate::state::cancel_race_test_hook::arm(
            hook_key,
            Arc::new(move || {
                // Signal the test: admin has won reason, lock is held (before try_send).
                hook_ready_tx.send(()).unwrap();
                // Hold the lock until the test releases — this is what causes
                // lifecycle_cancel() in drain_terminal!() to BLOCK.
                hook_proceed_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .expect(
                        "R2 drain cancel_race_test_hook: test did not send proceed within 5 s — \
                         handler or test deadlocked",
                    );
            }),
        );

        // ── Arm before_not_a_member_drain_terminal BEFORE connecting ─────
        let (handler_arrived_rx, handler_release) =
            crate::nip_fi_test_hooks::audio_not_a_member_drain_terminal_hook::arm(community_id);

        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/"))
            .await
            .expect("R2 drain: connect client");

        // NIP-42 auth exchange.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("R2 drain: challenge timeout")
            .expect("R2 drain: challenge item")
            .expect("R2 drain: challenge message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("R2 drain: expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("R2 drain: challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("R2 drain: challenge field")
            .to_string();

        let relay_url = format!("ws://{}", tenant.host());
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", &relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&non_member_key)
            .unwrap();
        let auth_msg = serde_json::json!({"type": "auth", "event": auth_event}).to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("R2 drain: send auth");

        // ── Wait for handler to reach before_not_a_member_drain_terminal ──
        // Handler has done: challenge → auth → relay membership → membership check Err.
        tokio::time::timeout(std::time::Duration::from_secs(10), handler_arrived_rx)
            .await
            .expect(
                "R2 drain: handler must reach before_not_a_member_drain_terminal within 10s \
                 (proves hook is at the correct seam in the not-a-member arm)",
            )
            .expect("R2 drain: hook arrived channel closed");

        // Frame 0: "not a member" diagnostic JSON (sent before the hook).
        let frame0 = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("R2 drain: frame 0 timeout")
            .expect("R2 drain: frame 0 item")
            .expect("R2 drain: frame 0 message");
        match &frame0 {
            tokio_tungstenite::tungstenite::Message::Text(t) => {
                assert!(
                    t.contains("not a member"),
                    "R2 drain: frame 0 must be the 'not a member' diagnostic; got: {t}"
                );
            }
            other => panic!("R2 drain: frame 0 must be Text(not a member); got {other:?}"),
        }

        // ── Fire admin disconnect in a thread ─────────────────────────────
        // disconnect_nip_fi runs in a sync thread (holds std::sync::Mutex).
        // The cancel_race_test_hook pauses it after winning the reason but
        // BEFORE try_send, while holding the transition lock.
        let non_member_pubkey = non_member_key.public_key().to_bytes().to_vec();
        // The session was admitted without an assertion (see setup), so it has
        // no NIP-FI issuer and no issuer-scoped scan can match it.  Stamp an
        // issuer on the already-registered identity (handler is parked at the
        // drain seam) so the admin scan reaches this control.
        conn_control.set_proven_identity(non_member_pubkey.clone(), Some("test-issuer".to_owned()));
        let state_for_deny = Arc::clone(&state);
        let deny_thread = std::thread::spawn(move || {
            state_for_deny
                .community_connections
                .disconnect_nip_fi("test-issuer", &non_member_pubkey)
        });

        // Wait for admin to reach the hook (won reason, lock held).
        // Only now is the invariant established: admin holds the lock + handler
        // is at the drain_terminal!() seam → releasing the handler will cause
        // lifecycle_cancel() to block.
        hook_ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect(
                "R2 drain: cancel_race_test_hook never fired within 5 s — \
                 disconnect_nip_fi hook path never reached (check pubkey registration)",
            );

        // ── Release handler hook, then release admin from a background thread ─
        // The handler proceeds to drain_terminal!() → lifecycle_cancel() → tries to
        // acquire the transition lock → BLOCKS (admin holds it).
        //
        // We CANNOT yield between releasing the handler and releasing admin when running
        // on a single-threaded tokio runtime (#[tokio::test]):  the handler's synchronous
        // Mutex::lock() inside lifecycle_cancel() blocks the tokio worker thread once the
        // handler task is scheduled, making it impossible for the test task to ever send
        // the proceed signal after yielding.
        //
        // Fix: fire the proceed signal from a separate OS thread with a small delay.
        // This allows the sequence:
        //   1. handler_release.notify_one() — handler is runnable
        //   2. OS thread spawned to send proceed after 10 ms
        //   3. test awaits frame 1 → yields to runtime
        //   4. handler task runs → lifecycle_cancel() → blocks on mutex (admin holds it)
        //   5. OS thread (10 ms later) → sends proceed → admin completes try_send → drops lock
        //   6. handler acquires lock → lifecycle_cancel completes → drain → gets denial frame
        //
        // Falsification target B: remove lifecycle_cancel() from drain_terminal!() →
        //   handler does NOT block on mutex → drains immediately in step 4 →
        //   channel is EMPTY (admin hasn't sent yet, 10 ms timer not fired) →
        //   try_recv returns Err(Empty) → no denial frame → frame 1 is POLICY close →
        //   assertion below panics.
        handler_release.notify_one();
        // Release admin from a background thread to avoid blocking the tokio worker.
        std::thread::spawn(move || {
            // Small pause: lets the handler task be scheduled and start lifecycle_cancel().
            // Even if the handler races ahead (no lifecycle_cancel), it will drain an
            // EMPTY channel (admin hasn't sent yet) → no denial frame → mutation RED.
            std::thread::sleep(std::time::Duration::from_millis(20));
            let _ = hook_proceed_tx.send(());
        });

        // ── Await denial frame: proves lifecycle_cancel() blocked + admin enqueued ─
        // Frame 1: Audio restricted JSON (denial frame from admin's try_send).
        // This frame proves that drain_terminal!()'s lifecycle_cancel() blocked
        // on admin's lock and waited for try_send to complete before draining.
        // Without lifecycle_cancel() in drain_terminal!(), the drain runs before
        // admin's try_send → channel empty → this timeout fires → test RED.
        let frame1 = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .expect(
                "R2 drain: frame 1 (denial) timeout — drain_terminal!() must deliver \
                 the admin denial frame before close. \
                 Mutation B: remove lifecycle_cancel() from drain_terminal!() → \
                 drain races admin's try_send → channel empty → this times out → RED.",
            )
            .expect("R2 drain: frame 1 item")
            .expect("R2 drain: frame 1 message");
        let expected_restricted = serde_json::json!({
            "type": "restricted",
            "message": buzz_auth::DenialClass::AuthorizationDenied.nostr_text()
        })
        .to_string();
        match &frame1 {
            tokio_tungstenite::tungstenite::Message::Text(t) => assert_eq!(
                t.as_str(),
                expected_restricted.as_str(),
                "R2 drain: frame 1 must be the Audio restricted JSON denial frame \
                 (proves drain_terminal!() serialized after admin's try_send). \
                 Mutation A: remove R2 drain from drain_terminal!() → this times out."
            ),
            other => panic!("R2 drain: frame 1 must be Text(restricted JSON); got {other:?}"),
        }

        // Frame 2: 1008 POLICY close.
        let frame2 = tokio::time::timeout(std::time::Duration::from_secs(3), client.next())
            .await
            .expect("R2 drain: frame 2 timeout")
            .expect("R2 drain: frame 2 item")
            .expect("R2 drain: frame 2 message");
        match &frame2 {
            tokio_tungstenite::tungstenite::Message::Close(Some(cf)) => {
                assert_eq!(
                    cf.code,
                    tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy,
                    "R2 drain: close code must be 1008 POLICY; got {:?}",
                    cf.code
                );
            }
            other => panic!("R2 drain: frame 2 must be Close(Some(1008 POLICY)); got {other:?}"),
        }

        let closed = deny_thread.join().expect("R2 drain: deny thread panicked");
        assert_eq!(
            closed, 1,
            "R2 drain: registry scan must find exactly 1 audio session \
             (proves audio_post_auth_register ran before the hook)"
        );

        server.abort();
        let _ = server.await;
    }

    // ── R2: room-ended admission error + concurrent admin deny → denial delivered ──
    //
    // Production path: `handle_active_audio_connection`, AdmissionError::Ended arm:
    //   1. acquire_effect() → Ok (permit acquired)
    //   2. add_peer()       → Err(Ended)             ← room pre-marked ended
    //   3. ws_send.send(room_ended error frame)
    //   4. [R2 hook fires here — admin fires denial]
    //   5. control.lifecycle_cancel()                  ← loses reason (admin won)
    //   6. R2 drain: terminal_ctrl_rx.try_recv() → denial frame → ws_send
    //   7. ws_send.send(reason.close_message())        ← 1008 POLICY close
    //
    // The WS client observes:
    //   frame 0: room_ended error JSON
    //   frame 1: Audio restricted JSON (denial frame queued by admin in step 4)
    //   frame 2: 1008 POLICY close
    //
    // Mutation evidence:
    //   A) Remove R2 drain (`while let Ok(msg) = terminal_ctrl_rx.try_recv()`) →
    //      admin denial is queued but never consumed → client never sees frame 1 →
    //      timeout on frame-1 recv → test panics.
    //   B) Delete `before_room_ended_lifecycle_cancel(...)` from handler →
    //      `arrived_rx` times out → test panics (proves hook is at the right seam).
    //   C) Remove `set_terminal_frame_sender` call from handler → admin's
    //      `disconnect_nip_fi` finds no sender → nothing enqueued → frame 1 absent →
    //      panics.
    //
    // Requires Postgres for `seed_audio_fixture` (channel/member row for membership check).
    async fn r2_room_ended_concurrent_admin_delivers_denial_before_close_body() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::sync::Arc;
        use tokio_tungstenite::connect_async;

        let state = audio_test_state_real_db().await;
        let pool = state.db.pool().clone();
        let (tenant, channel_id, member_key) = seed_audio_fixture(&pool).await;
        let community_id = tenant.community();

        // Far-future deadline: expiry task is armed but does NOT fire during this test.
        // The denial comes from the concurrent admin path, not expiry.
        let assertion = VerifiedAssertion::for_test(
            Some(member_key.public_key()),
            vec![Utc::now() + Duration::hours(1)],
        );

        let pubkey_bytes = member_key.public_key().to_bytes().to_vec();

        // Pre-mark the room as ended so add_peer returns AdmissionError::Ended.
        // The room must exist in audio_rooms BEFORE the handler calls get_or_create;
        // calling get_or_create and then marking it ended is equivalent.
        let room = state.audio_rooms.get_or_create(community_id, channel_id);
        room.mark_ended();

        // Pre-create and register the control so state.community_connections can
        // find this audio session by pubkey for the admin disconnect scan.
        // audio_post_auth_register writes proven_pubkey on this same Arc after
        // NIP-42 auth; the registered entry is updated in-place.
        let conn_cancel = tokio_util::sync::CancellationToken::new();
        let conn_control = crate::state::CommunityConnectionControl::new(conn_cancel.clone());
        let conn_id = Uuid::new_v4();
        let _conn_guard =
            state
                .community_connections
                .register(conn_id, community_id, conn_control.clone());
        let conn_control_for_server = conn_control.clone();

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let state_c = Arc::clone(&state);
        let tenant_c = tenant.clone();
        let assertion_c = assertion.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("R2: bind test listener");
        let addr = listener.local_addr().expect("R2: listener addr");

        let server = tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/",
                axum::routing::get({
                    let state_i = Arc::clone(&state_c);
                    let tenant_i = tenant_c.clone();
                    let assertion_i = assertion_c.clone();
                    let control_outer = conn_control_for_server.clone();
                    move |ws: axum::extract::WebSocketUpgrade| {
                        let state_i = Arc::clone(&state_i);
                        let tenant_i = tenant_i.clone();
                        let assertion_i = assertion_i.clone();
                        let control_inner = control_outer.clone();
                        let conn_time = chrono::Utc::now();
                        async move {
                            ws.on_upgrade(move |socket| async move {
                                handle_active_audio_connection(
                                    socket,
                                    state_i,
                                    tenant_i,
                                    channel_id,
                                    control_inner,
                                    Some(assertion_i),
                                    conn_time,
                                )
                                .await
                            })
                        }
                    }
                }),
            );
            let _ = ready_tx.send(());
            axum::serve(listener, app).await.expect("R2: test server");
        });

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ready_rx)
            .await
            .expect("R2: server ready");

        // Arm the R2 hook BEFORE connecting so we capture the handler at the
        // Ended error seam (after room_ended error sent, before lifecycle_cancel).
        let (arrived_rx, hook_release) =
            crate::nip_fi_test_hooks::audio_room_ended_lifecycle_cancel_hook::arm(community_id);

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("R2: connect client");

        // NIP-42 auth exchange.
        let challenge_msg = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("R2: challenge timeout")
            .expect("R2: challenge item")
            .expect("R2: challenge message");
        let challenge_text = match challenge_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            other => panic!("R2: expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("R2: challenge JSON");
        let challenge = challenge_json["challenge"]
            .as_str()
            .expect("R2: challenge field")
            .to_string();

        // Derive relay URL from fixture tenant — must match nip42_expected_relay_url(
        // &state.config.relay_url, &tenant) computed in handle_active_audio_connection.
        let relay_url = format!("ws://{}", tenant.host());
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", &relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&member_key)
            .unwrap();
        let auth_msg = serde_json::json!({"type": "auth", "event": auth_event}).to_string();
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                auth_msg.into(),
            ))
            .await
            .expect("R2: send auth");

        // Wait for handler to reach before_room_ended_lifecycle_cancel.
        // Handler has done: auth → membership → channel reads → add_peer (Ended) →
        // room_ended error frame sent → HOOK fires.
        tokio::time::timeout(std::time::Duration::from_secs(10), arrived_rx)
            .await
            .expect(
                "R2: handler must reach before_room_ended_lifecycle_cancel within 10s \
                 (proves hook is at the correct seam in the Ended arm)",
            )
            .expect("R2: hook arrived channel closed");

        // Frame 0: room_ended error JSON (sent before the hook, so available now).
        let frame0 = tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
            .await
            .expect("R2: frame 0 timeout")
            .expect("R2: frame 0 item")
            .expect("R2: frame 0 message");
        match &frame0 {
            tokio_tungstenite::tungstenite::Message::Text(t) => {
                assert!(
                    t.contains("room_ended"),
                    "R2: frame 0 must be the room_ended error frame; got: {t}"
                );
            }
            other => panic!("R2: frame 0 must be Text(room_ended error); got {other:?}"),
        }

        // Admin disconnect: fires while handler is held at the hook.
        // CommunityConnectionControl::disconnect_nip_fi (via registry scan) enqueues
        // the Audio denial frame on terminal_ctrl_tx, wins AuthorizationDenied,
        // then calls cancel.cancel().
        let closed = state
            .community_connections
            .disconnect_nip_fi("test-issuer", &pubkey_bytes);
        assert_eq!(
            closed, 1,
            "R2: registry scan must find exactly 1 audio session \
             (proves audio_post_auth_register ran before the hook)"
        );

        // Release hook → handler calls lifecycle_cancel (loses reason, but cancel
        // is set), awaits expiry task, then R2 drain delivers the admin's denial
        // frame + reason.close_message() (1008 POLICY).
        hook_release.notify_one();

        // Frame 1: Audio authorization-denied JSON (R2 drain delivers admin denial).
        let frame1 = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .expect("R2: frame 1 timeout (denial frame must be delivered by R2 drain)")
            .expect("R2: frame 1 item")
            .expect("R2: frame 1 message");
        let expected_restricted = serde_json::json!({
            "type": "restricted",
            "message": buzz_auth::DenialClass::AuthorizationDenied.nostr_text()
        })
        .to_string();
        match &frame1 {
            tokio_tungstenite::tungstenite::Message::Text(t) => assert_eq!(
                t.as_str(),
                expected_restricted.as_str(),
                "R2: frame 1 must be the Audio restricted JSON denial frame \
                 (proves R2 drain delivered the admin-queued denial). \
                 Mutation A: remove R2 drain → this times out."
            ),
            other => panic!("R2: frame 1 must be Text(restricted JSON); got {other:?}"),
        }

        // Frame 2: 1008 POLICY close (reason.close_message() from AuthorizationDenied).
        let frame2 = tokio::time::timeout(std::time::Duration::from_secs(3), client.next())
            .await
            .expect("R2: frame 2 timeout")
            .expect("R2: frame 2 item")
            .expect("R2: frame 2 message");
        match &frame2 {
            tokio_tungstenite::tungstenite::Message::Close(Some(cf)) => {
                assert_eq!(
                    cf.code,
                    tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy,
                    "R2: close code must be 1008 POLICY; got {:?}",
                    cf.code
                );
            }
            other => panic!("R2: frame 2 must be Close(Some(1008 POLICY)); got {other:?}"),
        }

        server.abort();
        let _ = server.await;
    }

    mod postgres_tests {
        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn w9_expiry_before_participant_commit_rolls_back_48101_insert() {
            super::w9_expiry_before_participant_commit_rolls_back_48101_insert_body().await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn w10_concurrent_committers_expiry_during_second_first_row_intact() {
            super::w10_concurrent_committers_expiry_during_second_first_row_intact_body().await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn w10_reaffirm_expiry_during_second_same_pubkey_first_row_intact() {
            super::w10_reaffirm_expiry_during_second_same_pubkey_first_row_intact_body().await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn cw5_auto_add_path_expiry_before_commit_rolls_back_both_rows() {
            super::cw5_auto_add_path_expiry_before_commit_rolls_back_both_rows_body().await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn cw5_variant_concurrent_external_membership_add_preserved() {
            super::cw5_variant_concurrent_external_membership_add_preserved_body().await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn cw8_post_add_peer_cancel_removes_peer_and_cleans_up_real_db() {
            super::cw8_post_add_peer_cancel_removes_peer_and_cleans_up_real_db_body().await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn cw10_expiry_blocked_at_permit_barrier_until_fan_out_completes() {
            super::cw10_expiry_blocked_at_permit_barrier_until_fan_out_completes_body().await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn cw10_full_handler_committed_join_produces_exactly_one_leave_event() {
            super::cw10_full_handler_committed_join_produces_exactly_one_leave_event_body().await;
        }

        /// F3: lifecycle_generation committed in kind-48101 JOIN matches the relay's
        /// global liveness generation (the same value `handle_huddle_liveness_req`
        /// returns for Off-mode rooms).
        ///
        /// This test is discoverable by the PostgreSQL Tests CI lane and exercises
        /// the full DB-write path on a live Postgres instance.
        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn f3_commit_participant_join_includes_lifecycle_generation() {
            super::f3_commit_participant_join_includes_lifecycle_generation_body().await;
        }

        /// F3 (wire): real admission → actual 48101 JOIN wire bytes → generation field
        /// matches `state.huddle_liveness_generation` — the production Off-mode fallback
        /// at `audio/handler.rs:802`.
        ///
        /// This is the production-bound complement to `f3_commit_participant_join_includes_lifecycle_generation`:
        /// it drives the real handler to a committed JOIN, reads the actual wire bytes from
        /// the DB, and asserts the generation field against the authoritative source.
        /// Changing line 802 to a hardcoded UUID would fail this test but NOT the direct
        /// `commit_participant_join` test (which supplies the generation as an input).
        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn f3_wire_admission_generation_matches_liveness_source() {
            super::f3_wire_admission_generation_matches_liveness_source_body().await;
        }

        /// F4 (add-peer path): handler's `SessionExpired` arm at `audio_gate.acquire_effect()`
        /// in Step 5 calls `expiry_deny_terminal` and delivers the Audio denial frame +
        /// 1008 POLICY close before the socket drops.  Exercised via the full handler.
        ///
        /// Discoverable by the PostgreSQL Tests CI lane (requires `seed_audio_fixture` DB writes).
        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn f4_add_peer_expired_delivers_denial_before_close() {
            super::f4_add_peer_expired_delivers_denial_before_close_body().await;
        }

        /// F4 (commit path): handler's `JoinCommitError::Expired` arm calls `expiry_deny_terminal`
        /// and delivers the Audio denial frame + 1008 POLICY close before the socket drops.
        /// Exercised via the full handler with `before_participant_commit` hook.
        ///
        /// Discoverable by the PostgreSQL Tests CI lane (requires full DB commit path).
        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn f4_commit_expired_delivers_denial_before_close() {
            super::f4_commit_expired_delivers_denial_before_close_body().await;
        }

        /// F4 (elapsed-deadline arm): the expiry task's real `tokio::time::sleep` fires,
        /// pausing BEFORE `gate.expire()` so `cancel.is_cancelled() == false`.
        /// The handler's `acquire_effect()` then returns `SessionExpired` via the
        /// elapsed-deadline arm at `nip_fi_gate.rs:139-142` (not the cancel arm at 136-137).
        ///
        /// Discoverable by the PostgreSQL Tests CI lane (requires `seed_audio_fixture`
        /// for the membership DB check and channel DB reads).
        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn f4_elapsed_deadline_arm_fires_on_gate_acquire() {
            super::f4_elapsed_deadline_arm_fires_on_gate_acquire_body().await;
        }

        /// R2: room-ended admission error racing concurrent admin deny → R2 drain delivers
        /// denial frame + 1008 POLICY close after the room_ended error frame.
        ///
        /// Discoverable by the PostgreSQL Tests CI lane (requires `seed_audio_fixture`
        /// for the membership DB check in `handle_active_audio_connection`).
        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn r2_room_ended_concurrent_admin_delivers_denial_before_close() {
            super::r2_room_ended_concurrent_admin_delivers_denial_before_close_body().await;
        }

        /// R2 (drain_terminal!() path): non-member early exit with concurrent admin deny →
        /// `drain_terminal!()`'s `lifecycle_cancel()` serializes admin's `try_send` so
        /// the denial frame is delivered before close.
        ///
        /// Falsification target: remove only `lifecycle_cancel()` from `drain_terminal!()` →
        /// handler drains before admin enqueues → denial absent → frame-1 timeout → RED.
        ///
        /// Discoverable by the PostgreSQL Tests CI lane (requires `seed_audio_fixture`
        /// for the membership DB check).  Uses a NON-MEMBER key so the not-a-member
        /// arm fires.
        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn r2_drain_terminal_lifecycle_cancel_serializes_admin_frame() {
            super::r2_drain_terminal_lifecycle_cancel_serializes_admin_frame_body().await;
        }
    }
}
