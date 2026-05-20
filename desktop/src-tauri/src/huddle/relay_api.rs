//! Relay HTTP helpers for huddle operations.
//!
//! Thin wrappers around the relay REST API for channel membership queries,
//! human participant counting, and the audio relay WebSocket connection.
//!
//! ```text
//! connect_audio_relay(channel_id)
//!   → WS /huddle/{id}/audio → challenge → NIP-42 auth → joined
//!   → send loop: pcm_rx → Opus encode → WS binary frame
//!   → recv loop: WS binary frame → Opus decode (per-peer) → rodio playback
//! ```

use futures_util::{SinkExt, StreamExt};
use std::sync::{atomic::AtomicBool, Arc};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMsg};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::relay::query_relay;

/// Maximum number of agents that can be invited to a single huddle.
pub(crate) const MAX_HUDDLE_AGENTS: usize = 20;

/// Per-peer frame threshold: speech ≈ 25 frames/500ms, DTX noise ≈ 1.
pub(crate) const REMOTE_SPEECH_THRESHOLD: u16 = 5;

/// Validate that a string looks like a Nostr pubkey hex (64 hex chars).
pub(crate) fn validate_pubkey_hex(pubkey: &str) -> Result<(), String> {
    if pubkey.len() != 64 || !pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
        let preview: String = pubkey.chars().take(16).collect();
        return Err(format!("invalid pubkey hex: {preview}"));
    }
    Ok(())
}

pub(crate) fn parse_channel_uuid(channel_id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(channel_id).map_err(|_| format!("invalid channel UUID: {channel_id}"))
}

/// Handshake timeout — matches the server's AUTH_TIMEOUT (5 s).
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Connect to the relay's audio WebSocket and run the Opus encode/decode pipeline.
///
/// Returns `(cancel_token, pcm_sender)` — caller stores both in `HuddleState`.
/// Dropping the sender or calling `cancel.cancel()` shuts down the relay task.
pub(crate) async fn connect_audio_relay(
    channel_id: &str,
    parent_channel_id: Option<&str>,
    state: &AppState,
) -> Result<(CancellationToken, tokio::sync::mpsc::Sender<Vec<u8>>), String> {
    use nostr::JsonUtil;

    let relay_url = crate::relay::relay_ws_url_with_override(state);
    let ws_url = format!("{relay_url}/huddle/{channel_id}/audio");

    let keys = state.keys.lock().map_err(|e| e.to_string())?.clone();

    // TTS interrupt flags — recv task cancels TTS when remote humans speak.
    let (tts_cancel, tts_active) = {
        let hs = state.huddle()?;
        (Arc::clone(&hs.tts_cancel), Arc::clone(&hs.tts_active))
    };

    let app_handle = state.app_handle.lock().ok().and_then(|g| g.clone());

    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .map_err(|e| format!("audio WS connect failed: {e}"))?;
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    let challenge = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        loop {
            match ws_rx.next().await {
                Some(Ok(WsMsg::Text(text))) => {
                    let v: serde_json::Value = serde_json::from_str(&text)
                        .map_err(|e| format!("bad challenge JSON: {e}"))?;
                    if v["type"] == "challenge" {
                        break v["challenge"]
                            .as_str()
                            .ok_or_else(|| "missing challenge string".to_string())
                            .map(|s| s.to_string());
                    }
                }
                Some(Ok(WsMsg::Close(_))) | None => {
                    break Err("connection closed before challenge".into());
                }
                _ => continue,
            }
        }
    })
    .await
    .map_err(|_| "timeout waiting for challenge from relay".to_string())?
    .map_err(|e: String| e)?;

    let tags = vec![
        nostr::Tag::parse(["relay", &relay_url]).map_err(|e| format!("tag relay: {e}"))?,
        nostr::Tag::parse(["challenge", &challenge]).map_err(|e| format!("tag challenge: {e}"))?,
    ];
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(22242), "")
        .tags(tags)
        .sign_with_keys(&keys)
        .map_err(|e| format!("sign: {e}"))?;

    let event_json: serde_json::Value = serde_json::from_str(&event.as_json())
        .map_err(|e| format!("failed to serialize auth event: {e}"))?;
    let auth_msg = serde_json::json!({
        "type": "auth",
        "event": event_json,
        "parent_channel_id": parent_channel_id,
        // Negotiate huddle audio protocol v2 (8-byte sender-authored header
        // per Opus frame: seq | ts_48k | level_dbov | flags). See
        // huddle::wire for the layout. The relay pins the first joiner's
        // version per-room and rejects mismatched joiners with
        // `upgrade_required`.
        "protocol_version": super::wire::PROTOCOL_VERSION,
    });
    ws_tx
        .send(WsMsg::Text(auth_msg.to_string().into()))
        .await
        .map_err(|e| format!("send auth: {e}"))?;

    let initial_peers: Vec<(u8, String)> = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        loop {
            match ws_rx.next().await {
                Some(Ok(WsMsg::Text(text))) => {
                    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                    match v["type"].as_str() {
                        Some("joined") => {
                            let peers = v["peers"]
                                .as_array()
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|p| {
                                            Some((
                                                p["peer_index"].as_u64()? as u8,
                                                p["pubkey"].as_str()?.to_string(),
                                            ))
                                        })
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default();
                            break Ok(peers);
                        }
                        Some("error") => {
                            break Err(format!("audio relay auth error: {}", v["message"]));
                        }
                        _ => continue,
                    }
                }
                Some(Ok(WsMsg::Close(_))) | None => {
                    break Err("connection closed before joined".into());
                }
                _ => continue,
            }
        }
    })
    .await
    .map_err(|_| "timeout waiting for joined from relay".to_string())?
    .map_err(|e: String| e)?;

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let (pcm_tx, pcm_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(50);
    let output_device_name = state
        .audio_output_device
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    tokio::spawn(async move {
        if let Err(e) = audio_relay_pipeline(
            ws_tx,
            ws_rx,
            pcm_rx,
            cancel_clone.clone(),
            app_handle.clone(),
            initial_peers,
            tts_cancel,
            tts_active,
            output_device_name,
        )
        .await
        {
            eprintln!("sprout-desktop: audio relay pipeline exited: {e}");
        }

        // Only emit the disconnect event for UNEXPECTED exits.
        // Skip if already cancelled (teardown_huddle in progress).
        if !cancel_clone.is_cancelled() {
            cancel_clone.cancel();
            if let Some(ref app) = app_handle {
                use tauri::Emitter;
                let _ = app.emit("huddle-audio-disconnected", ());
            }
        }
    });

    Ok((cancel, pcm_tx))
}

/// Background Opus encode/decode pipeline spawned by `connect_audio_relay`.
pub(crate) type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn audio_relay_pipeline(
    ws_tx: futures_util::stream::SplitSink<WsStream, WsMsg>,
    ws_rx: futures_util::stream::SplitStream<WsStream>,
    mut pcm_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    cancel: CancellationToken,
    app_handle: Option<tauri::AppHandle>,
    initial_peers: Vec<(u8, String)>,
    tts_cancel: Arc<AtomicBool>,
    tts_active: Arc<AtomicBool>,
    output_device_name: Option<String>,
) -> Result<(), String> {
    let mut encoder = opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Voip)
        .map_err(|e| format!("opus encoder: {e}"))?;
    encoder
        .set_bitrate(opus::Bitrate::Bits(32000))
        .map_err(|e| format!("opus bitrate: {e}"))?;
    encoder
        .set_dtx(true)
        .map_err(|e| format!("opus dtx: {e}"))?;

    let sink_handle = super::audio_output::open_output_sink_by_name(output_device_name.as_deref())?;

    use std::sync::Arc as StdArc;
    let ws_tx = StdArc::new(tokio::sync::Mutex::new(ws_tx));
    let ws_tx_send = StdArc::clone(&ws_tx);
    let cancel_send = cancel.clone();

    let send_task = tokio::spawn(async move {
        use super::wire::{audio_level_dbov, FrameHeader, V2_HEADER_LEN};
        let mut encoder = encoder; // Move encoder into task.
        const FRAME_SAMPLES: usize = 960;
        let mut out_buf = vec![0u8; 4000];
        // Per-frame wire-protocol state. We send v2 frames now: each Opus
        // payload is preceded by an 8-byte header carrying our own seq +
        // 48 kHz timestamp + audio level + flags.
        let mut seq: u16 = 0;
        let mut ts_48k: u32 = 0;

        loop {
            let pcm_bytes = {
                use futures_util::future::Either;
                let cancelled = std::pin::pin!(cancel_send.cancelled());
                let recv = std::pin::pin!(pcm_rx.recv());
                match futures_util::future::select(cancelled, recv).await {
                    Either::Left(_) => break, // Cancelled.
                    Either::Right((Some(b), _)) => b,
                    Either::Right((None, _)) => break, // Sender dropped.
                }
            };

            if pcm_bytes.len() % 4 != 0 {
                continue; // Malformed batch.
            }
            let samples: Vec<f32> = pcm_bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();

            let mut tx = ws_tx_send.lock().await;
            for chunk in samples.chunks(FRAME_SAMPLES) {
                // dBov is computed from the pre-encode PCM. Opus DTX may
                // produce a 1-2 byte comfort packet; computing level from
                // the encoded payload would be meaningless.
                let level = audio_level_dbov(chunk);
                let encode_result = if chunk.len() == FRAME_SAMPLES {
                    encoder.encode_float(chunk, &mut out_buf)
                } else {
                    let mut padded = chunk.to_vec();
                    padded.resize(FRAME_SAMPLES, 0.0);
                    encoder.encode_float(&padded, &mut out_buf)
                };
                let n = match encode_result {
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("sprout-desktop: opus encode error: {e}");
                        continue;
                    }
                };
                if n > 0 {
                    // Opus DTX packets are very small (≤2 bytes). Flag them
                    // explicitly so the receiver can elide DTX from speaker
                    // detection without re-parsing the Opus payload.
                    let flags = if n <= 2 { super::wire::FLAG_DTX } else { 0 };
                    let header = FrameHeader {
                        seq,
                        ts_48k,
                        level_dbov: level,
                        flags,
                    }
                    .encode();

                    // Build the v2 wire frame: 8-byte header + Opus payload.
                    let mut frame = Vec::with_capacity(V2_HEADER_LEN + n);
                    frame.extend_from_slice(&header);
                    frame.extend_from_slice(&out_buf[..n]);
                    if tx.send(WsMsg::Binary(frame.into())).await.is_err() {
                        return; // WS closed.
                    }

                    seq = seq.wrapping_add(1);
                    ts_48k = ts_48k.wrapping_add(super::jitter::FRAME_TIMESTAMP_DELTA);
                }
            }
        }
        let mut tx = ws_tx_send.lock().await;
        let _ = tx.send(WsMsg::Close(None)).await;
    });

    let recv_task = tokio::spawn(super::playout::run_playout_recv_loop(
        ws_rx,
        ws_tx,
        sink_handle,
        cancel,
        app_handle,
        initial_peers,
        tts_active,
        tts_cancel,
    ));

    // Wait for either task to finish, then abort the survivor.
    use futures_util::future::Either;
    match futures_util::future::select(std::pin::pin!(send_task), std::pin::pin!(recv_task)).await {
        Either::Left((_, recv_handle)) => recv_handle.abort(),
        Either::Right((_, send_handle)) => send_handle.abort(),
    }

    Ok(())
}

/// Fetch channel members with roles from the relay. Returns (pubkey, role) tuples.
///
/// Queries kind:39002 (NIP-29 members) by `#d` channel id and extracts
/// `["p", pubkey, relay_url?, role?]` tags from the most recent event.
pub(crate) async fn fetch_channel_members_with_roles(
    channel_id: &str,
    state: &AppState,
) -> Result<Vec<(String, Option<String>)>, String> {
    let filter = serde_json::json!({
        "kinds": [39002],
        "#d": [channel_id],
        "limit": 1,
    });
    let events = query_relay(state, std::slice::from_ref(&filter))
        .await
        .map_err(|e| {
            eprintln!("sprout-desktop: fetch channel members failed: {e}");
            e
        })?;

    let Some(event) = events.first() else {
        return Ok(Vec::new());
    };

    let mut seen = std::collections::BTreeSet::new();
    let mut members = Vec::new();
    for tag in event.tags.iter() {
        let slice = tag.as_slice();
        if slice.first().map(String::as_str) != Some("p") {
            continue;
        }
        let Some(pubkey) = slice.get(1) else { continue };
        if pubkey.is_empty() || !seen.insert(pubkey.clone()) {
            continue;
        }
        let role = slice.get(3).filter(|s| !s.is_empty()).cloned();
        members.push((pubkey.clone(), role));
    }
    Ok(members)
}

/// Fetch channel members, optionally filtered by role (e.g., "bot" for agents).
pub(crate) async fn fetch_channel_members(
    channel_id: &str,
    role_filter: Option<&str>,
    state: &AppState,
) -> Result<Vec<String>, String> {
    let all = fetch_channel_members_with_roles(channel_id, state).await?;
    Ok(all
        .into_iter()
        .filter(|(_, role)| role_filter.map_or(true, |r| role.as_deref() == Some(r)))
        .map(|(pubkey, _)| pubkey)
        .collect())
}

/// Count human (non-bot) members remaining in a channel.
pub(crate) async fn count_human_members(
    channel_id: &str,
    state: &AppState,
) -> Result<usize, String> {
    let all = fetch_channel_members_with_roles(channel_id, state).await?;
    Ok(all
        .iter()
        .filter(|(_, role)| role.as_deref() != Some("bot"))
        .count())
}
