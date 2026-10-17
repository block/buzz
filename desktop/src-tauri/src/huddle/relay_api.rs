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

use buzz_audio_client::encoder::{AudioEncoder, FRAME_SAMPLES};
use futures_util::{SinkExt, StreamExt};
use std::sync::{atomic::AtomicBool, Arc};
use tokio_tungstenite::tungstenite::Message as WsMsg;
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

fn build_audio_auth_event(
    keys: &nostr::Keys,
    relay_url: &str,
    challenge: &str,
    auth_tag_json: Option<&str>,
) -> Result<nostr::Event, String> {
    let mut tags = vec![
        nostr::Tag::parse(["relay", relay_url]).map_err(|e| format!("tag relay: {e}"))?,
        nostr::Tag::parse(["challenge", challenge]).map_err(|e| format!("tag challenge: {e}"))?,
    ];
    if let Some(auth_tag_json) = auth_tag_json {
        let compat_pubkey = nostr::PublicKey::from_hex(&keys.public_key().to_hex())
            .map_err(|e| format!("agent pubkey conversion failed: {e}"))?;
        buzz_sdk_pkg::nip_oa::verify_auth_tag(auth_tag_json, &compat_pubkey)
            .map_err(|e| format!("agent auth tag verification failed: {e}"))?;
        let compat_tag = buzz_sdk_pkg::nip_oa::parse_auth_tag(auth_tag_json)
            .map_err(|e| format!("agent auth tag parse failed: {e}"))?;
        tags.push(
            nostr::Tag::parse(compat_tag.as_slice())
                .map_err(|e| format!("agent auth tag conversion failed: {e}"))?,
        );
    }
    nostr::EventBuilder::new(nostr::Kind::Custom(22242), "")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| format!("sign: {e}"))
}

async fn connect_authenticated_audio_socket(
    channel_id: &str,
    parent_channel_id: Option<&str>,
    relay_url: &str,
    keys: &nostr::Keys,
    auth_tag_json: Option<&str>,
) -> Result<(WsSink, WsReceiver, u8, Vec<(u8, String, u8)>), String> {
    use nostr::JsonUtil;

    let ws_url = format!("{relay_url}/huddle/{channel_id}/audio");
    let joined = buzz_audio_client::connection::connect(
        &ws_url,
        parent_channel_id,
        None,
        |challenge| async move {
            let event = build_audio_auth_event(keys, relay_url, &challenge, auth_tag_json)?;
            serde_json::from_str(&event.as_json())
                .map_err(|e| format!("failed to serialize auth event: {e}"))
        },
    )
    .await?;
    let (tx, rx) = joined.socket.split();
    let peers = joined
        .peers
        .into_iter()
        .map(|p| (p.index, p.pubkey, p.epoch))
        .collect();
    Ok((tx, rx, joined.index, peers))
}

/// Connect to the relay's audio WebSocket and run the Opus encode/decode pipeline.
///
/// Returns `(cancel_token, pcm_sender)` — caller stores both in `HuddleState`.
/// Dropping the sender or calling `cancel.cancel()` shuts down the relay task.
pub(crate) async fn connect_audio_relay(
    channel_id: &str,
    parent_channel_id: Option<&str>,
    state: &AppState,
) -> Result<(CancellationToken, tokio::sync::mpsc::Sender<Vec<u8>>), String> {
    let relay_url = crate::relay::relay_ws_url_with_override(state);
    let keys = state.keys.lock().map_err(|e| e.to_string())?.clone();

    // TTS interrupt flags — recv task cancels TTS when remote humans speak.
    let (
        tts_cancel,
        tts_active,
        local_tts_publishers,
        remote_stt_pipeline,
        agent_pubkeys,
        human_floor,
    ) = {
        let hs = state.huddle()?;
        (
            Arc::clone(&hs.tts_cancel),
            Arc::clone(&hs.tts_active),
            Arc::clone(&hs.local_tts_publishers),
            Arc::clone(&hs.remote_stt_pipeline),
            Arc::clone(&hs.agent_pubkeys),
            hs.human_floor.clone(),
        )
    };

    let app_handle = state.app_handle.lock().ok().and_then(|g| g.clone());

    let (ws_tx, ws_rx, _peer_index, initial_peers) =
        connect_authenticated_audio_socket(channel_id, parent_channel_id, &relay_url, &keys, None)
            .await?;

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let (pcm_tx, pcm_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(50);
    let output_device_name = state
        .huddle_audio
        .output_device
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    tokio::spawn(async move {
        if let Err(e) = audio_relay_pipeline(AudioRelayPipelineArgs {
            ws_tx,
            ws_rx,
            pcm_rx,
            cancel: cancel_clone.clone(),
            app_handle: app_handle.clone(),
            initial_peers,
            tts_cancel,
            tts_active,
            local_tts_publishers,
            remote_stt_pipeline,
            agent_pubkeys,
            human_floor,
            output_device_name,
        })
        .await
        {
            eprintln!("buzz-desktop: audio relay pipeline exited: {e}");
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
type WsSink = futures_util::stream::SplitSink<WsStream, WsMsg>;
type WsReceiver = futures_util::stream::SplitStream<WsStream>;

const TTS_BROADCAST_QUEUE_DEPTH: usize = 8;
const TTS_BROADCAST_MAX_FRAMES: usize = 1_500; // 30 seconds at 20 ms/frame.

struct QueuedTtsFrame {
    epoch: u64,
    speaker_generation: u64,
    samples_48k: Vec<f32>,
}

fn upsample_tts_24k_to_48k(samples_24k: &[f32]) -> Vec<f32> {
    let mut samples_48k = Vec::with_capacity(samples_24k.len().saturating_mul(2));
    for (index, sample) in samples_24k.iter().copied().enumerate() {
        let next = samples_24k.get(index + 1).copied().unwrap_or(sample);
        samples_48k.push(sample);
        samples_48k.push((sample + next) * 0.5);
    }
    samples_48k
}

fn queue_tts_broadcast_packet(
    queue: &mut std::collections::VecDeque<QueuedTtsFrame>,
    packet: super::tts::TtsBroadcastPacket,
    current_epoch: u64,
    current_speaker_generation: u64,
) {
    if packet.epoch != current_epoch
        || packet.speaker_generation != current_speaker_generation
        || packet.samples_24k.is_empty()
    {
        return;
    }
    let samples_48k = upsample_tts_24k_to_48k(&packet.samples_24k);
    for chunk in samples_48k.chunks(960) {
        if queue.len() >= TTS_BROADCAST_MAX_FRAMES {
            eprintln!("buzz-desktop: tts broadcast status=dropped reason=queue_duration_limit");
            break;
        }
        let mut frame = chunk.to_vec();
        frame.resize(960, 0.0);
        queue.push_back(QueuedTtsFrame {
            epoch: packet.epoch,
            speaker_generation: packet.speaker_generation,
            samples_48k: frame,
        });
    }
}

/// Open a send-only v2 Huddle audio peer authenticated as a locally managed
/// agent. The relay therefore assigns the synthesized stream to that agent's
/// existing pubkey; no backend or wire-protocol extension is required.
pub(crate) async fn connect_tts_audio_publisher(
    channel_id: &str,
    parent_channel_id: Option<&str>,
    state: &AppState,
    keys: &nostr::Keys,
    auth_tag_json: Option<&str>,
    local_tts_publishers: super::tts::LocalTtsPublishers,
) -> Result<super::tts::TtsAudioPublisher, String> {
    let relay_url = crate::relay::relay_ws_url_with_override(state);
    let (ws_tx, ws_rx, peer_index, _) = connect_authenticated_audio_socket(
        channel_id,
        parent_channel_id,
        &relay_url,
        keys,
        auth_tag_json,
    )
    .await?;

    let cancel = CancellationToken::new();
    let publisher_cancel = cancel.clone();
    let (tx, rx) = tokio::sync::mpsc::channel(TTS_BROADCAST_QUEUE_DEPTH);
    let publisher = super::tts::TtsAudioPublisher::new(tx, cancel);
    let (epoch, speaker_generation) = publisher.version_state();
    let local_publisher = super::tts::LocalTtsPublisherLease::new(peer_index, local_tts_publishers);
    tokio::spawn(async move {
        let _local_publisher = local_publisher;
        if let Err(error) = run_tts_audio_publisher(
            ws_tx,
            ws_rx,
            rx,
            publisher_cancel.clone(),
            epoch,
            speaker_generation,
        )
        .await
        {
            eprintln!("buzz-desktop: tts broadcast status=disconnected error={error}");
        }
        publisher_cancel.cancel();
    });
    Ok(publisher)
}

async fn run_tts_audio_publisher(
    mut ws_tx: WsSink,
    mut ws_rx: WsReceiver,
    mut audio_rx: tokio::sync::mpsc::Receiver<super::tts::TtsBroadcastPacket>,
    cancel: CancellationToken,
    epoch: Arc<std::sync::atomic::AtomicU64>,
    speaker_generation: Arc<std::sync::atomic::AtomicU64>,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    let mut encoder = AudioEncoder::new()?;
    let mut queue = std::collections::VecDeque::<QueuedTtsFrame>::new();
    let mut send_tick = tokio::time::interval(std::time::Duration::from_millis(20));
    send_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = send_tick.tick() => {
                let current_epoch = epoch.load(Ordering::Acquire);
                let current_generation = speaker_generation.load(Ordering::Acquire);
                while queue.front().is_some_and(|frame| {
                    frame.epoch != current_epoch
                        || frame.speaker_generation != current_generation
                }) {
                    queue.pop_front();
                }
                let Some(frame) = queue.pop_front() else { continue };
                let payload = encoder.encode(&frame.samples_48k)?;
                ws_tx
                    .send(WsMsg::Binary(payload.into()))
                    .await
                    .map_err(|error| format!("tts audio send: {error}"))?;
            }
            message = ws_rx.next() => {
                match message {
                    Some(Ok(WsMsg::Ping(data))) => {
                        ws_tx.send(WsMsg::Pong(data)).await
                            .map_err(|error| format!("tts audio pong: {error}"))?;
                    }
                    Some(Ok(WsMsg::Close(_))) | None => break,
                    Some(Err(error)) => return Err(format!("tts audio receive: {error}")),
                    Some(Ok(_)) => {}
                }
            }
            packet = audio_rx.recv() => {
                let Some(packet) = packet else { break };
                queue_tts_broadcast_packet(
                    &mut queue,
                    packet,
                    epoch.load(Ordering::Acquire),
                    speaker_generation.load(Ordering::Acquire),
                );
            }
        }
    }
    let _ = ws_tx.send(WsMsg::Close(None)).await;
    Ok(())
}

struct AudioRelayPipelineArgs {
    ws_tx: futures_util::stream::SplitSink<WsStream, WsMsg>,
    ws_rx: futures_util::stream::SplitStream<WsStream>,
    pcm_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    cancel: CancellationToken,
    app_handle: Option<tauri::AppHandle>,
    initial_peers: Vec<(u8, String, u8)>,
    tts_cancel: Arc<AtomicBool>,
    tts_active: Arc<AtomicBool>,
    local_tts_publishers: super::tts::LocalTtsPublishers,
    remote_stt_pipeline: Arc<std::sync::Mutex<Option<std::sync::Weak<super::stt::SttPipeline>>>>,
    agent_pubkeys: Arc<std::sync::Mutex<Vec<String>>>,
    human_floor: super::human_floor::HumanFloor,
    output_device_name: Option<String>,
}

async fn audio_relay_pipeline(args: AudioRelayPipelineArgs) -> Result<(), String> {
    let AudioRelayPipelineArgs {
        ws_tx,
        ws_rx,
        mut pcm_rx,
        cancel,
        app_handle,
        initial_peers,
        tts_cancel,
        tts_active,
        local_tts_publishers,
        remote_stt_pipeline,
        agent_pubkeys,
        human_floor,
        output_device_name,
    } = args;

    let encoder = AudioEncoder::new()?;

    let sink_handle = super::audio_output::open_output_sink_by_name(output_device_name.as_deref())?;

    use std::sync::Arc as StdArc;
    let ws_tx = StdArc::new(tokio::sync::Mutex::new(ws_tx));
    let ws_tx_send = StdArc::clone(&ws_tx);
    let cancel_send = cancel.clone();

    let send_task = tokio::spawn(async move {
        let mut encoder = encoder; // Move encoder into task.
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
                let frame = match encoder.encode(chunk) {
                    Ok(frame) => frame,
                    Err(error) => {
                        eprintln!("buzz-desktop: opus encode error: {error}");
                        continue;
                    }
                };
                if tx.send(WsMsg::Binary(frame.into())).await.is_err() {
                    return; // WS closed.
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
        local_tts_publishers,
        remote_stt_pipeline,
        agent_pubkeys,
        human_floor,
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
            eprintln!("buzz-desktop: fetch channel members failed: {e}");
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
        .filter(|(_, role)| role_filter.is_none_or(|r| role.as_deref() == Some(r)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tts_upsampling_doubles_rate_with_linear_midpoints() {
        assert_eq!(
            upsample_tts_24k_to_48k(&[0.0, 1.0, -1.0]),
            vec![0.0, 0.5, 1.0, 0.0, -1.0, -1.0]
        );
    }

    #[test]
    fn tts_queue_rejects_cancelled_versions_and_pads_twenty_ms_frames() {
        let mut queue = std::collections::VecDeque::new();
        queue_tts_broadcast_packet(
            &mut queue,
            super::super::tts::TtsBroadcastPacket {
                epoch: 1,
                speaker_generation: 7,
                samples_24k: vec![0.25; 480],
            },
            1,
            7,
        );
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].samples_48k.len(), 960);

        queue_tts_broadcast_packet(
            &mut queue,
            super::super::tts::TtsBroadcastPacket {
                epoch: 1,
                speaker_generation: 7,
                samples_24k: vec![0.5; 480],
            },
            2,
            7,
        );
        assert_eq!(queue.len(), 1, "cancelled epoch must not enqueue");
    }
}
