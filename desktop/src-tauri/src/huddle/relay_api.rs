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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AudioRelayConnectError {
    code: Option<String>,
    message: String,
}

impl AudioRelayConnectError {
    pub(crate) fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    pub(crate) fn from_relay_payload(value: &serde_json::Value) -> Self {
        Self {
            code: value["code"].as_str().map(str::to_string),
            message: value["message"]
                .as_str()
                .unwrap_or("unknown relay error")
                .to_string(),
        }
    }
}

impl From<String> for AudioRelayConnectError {
    fn from(message: String) -> Self {
        Self {
            code: None,
            message,
        }
    }
}

impl From<&str> for AudioRelayConnectError {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

impl std::fmt::Display for AudioRelayConnectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.code.as_deref() {
            Some(code) => write!(
                formatter,
                "audio relay auth error [{code}]: {}",
                self.message
            ),
            None => formatter.write_str(&self.message),
        }
    }
}

fn format_audio_relay_error(value: &serde_json::Value) -> AudioRelayConnectError {
    AudioRelayConnectError::from_relay_payload(value)
}

fn parse_audio_roster_peer(peer: &serde_json::Value) -> Option<(u8, String, u8)> {
    Some((
        u8::try_from(peer["peer_index"].as_u64()?).ok()?,
        peer["pubkey"].as_str()?.to_string(),
        // Legacy relays omit the occupancy epoch.
        u8::try_from(peer["epoch"].as_u64().unwrap_or(0)).ok()?,
    ))
}

async fn connect_authenticated_audio_socket(
    channel_id: &str,
    parent_channel_id: Option<&str>,
    relay_url: &str,
    keys: &nostr::Keys,
    auth_tag_json: Option<&str>,
) -> Result<(WsSink, WsReceiver, u8, Vec<(u8, String, u8)>), AudioRelayConnectError> {
    use nostr::JsonUtil;

    let ws_url = format!("{relay_url}/huddle/{channel_id}/audio");
    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .map_err(|e| format!("audio WS connect failed: {e}"))?;
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    let challenge = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        loop {
            match ws_rx.next().await {
                Some(Ok(WsMsg::Text(text))) => {
                    let value: serde_json::Value = serde_json::from_str(&text)
                        .map_err(|e| format!("bad challenge JSON: {e}"))?;
                    if value["type"] == "challenge" {
                        break value["challenge"]
                            .as_str()
                            .ok_or_else(|| "missing challenge string".to_string())
                            .map(str::to_string);
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
    .map_err(|_| "timeout waiting for challenge from relay".to_string())??;

    let event = build_audio_auth_event(keys, relay_url, &challenge, auth_tag_json)?;
    let event_json: serde_json::Value = serde_json::from_str(&event.as_json())
        .map_err(|e| format!("failed to serialize auth event: {e}"))?;
    let auth_msg = serde_json::json!({
        "type": "auth",
        "event": event_json,
        "parent_channel_id": parent_channel_id,
        // Use the released v2 contract while deployed relays remain capped at
        // v2. Relay-to-client media therefore has a one-byte peer-index prefix;
        // see huddle::wire for the compatibility tradeoff.
        "protocol_version": super::wire::PROTOCOL_VERSION,
    });
    ws_tx
        .send(WsMsg::Text(auth_msg.to_string().into()))
        .await
        .map_err(|e| format!("send auth: {e}"))?;

    let (peer_index, initial_peers) = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        loop {
            match ws_rx.next().await {
                Some(Ok(WsMsg::Text(text))) => {
                    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                    match value["type"].as_str() {
                        Some("joined") => {
                            let peers = value["peers"]
                                .as_array()
                                .map(|peers| {
                                    peers.iter().filter_map(parse_audio_roster_peer).collect()
                                })
                                .unwrap_or_default();
                            let peer_index = value["peer_index"]
                                .as_u64()
                                .and_then(|index| u8::try_from(index).ok())
                                .ok_or_else(|| "joined message missing peer index".to_string())?;
                            break Ok((peer_index, peers));
                        }
                        Some("error") => {
                            break Err(format_audio_relay_error(&value));
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
    .map_err(|_| "timeout waiting for joined from relay".to_string())??;

    Ok((ws_tx, ws_rx, peer_index, initial_peers))
}

/// Connect to the relay's audio WebSocket and run the Opus encode/decode pipeline.
///
/// Returns `(cancel_token, pcm_sender)` — caller stores both in `HuddleState`.
/// Dropping the sender or calling `cancel.cancel()` shuts down the relay task.
pub(crate) async fn connect_audio_relay(
    channel_id: &str,
    parent_channel_id: Option<&str>,
    state: &AppState,
) -> Result<(CancellationToken, tokio::sync::mpsc::Sender<Vec<u8>>), AudioRelayConnectError> {
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
        reconnect_target,
    ) = {
        let hs = state.huddle()?;
        (
            Arc::clone(&hs.tts_cancel),
            Arc::clone(&hs.tts_active),
            Arc::clone(&hs.local_tts_publishers),
            Arc::clone(&hs.remote_stt_pipeline),
            Arc::clone(&hs.agent_pubkeys),
            hs.human_floor.for_audio_connection(),
            super::reconnect::ReconnectTarget::capture(&hs, channel_id, parent_channel_id),
        )
    };

    let app_handle = state.app_handle.lock().ok().and_then(|g| g.clone());

    let (ws_tx, ws_rx, _peer_index, initial_peers) =
        connect_authenticated_audio_socket(channel_id, parent_channel_id, &relay_url, &keys, None)
            .await?;

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let (pcm_tx, pcm_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(50);
    let output_device_changes = state.huddle_audio.output_device_changes.subscribe();
    let output_device_name = output_device_changes.borrow().clone();

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
            output_device_changes,
        })
        .await
        {
            eprintln!("buzz-desktop: audio relay pipeline exited: {e}");
        }

        // Only UNEXPECTED exits reconnect. An already-cancelled token means
        // teardown_huddle is in progress and the huddle is going away.
        // Cancelling before the reconnect call is load-bearing: if this
        // pipeline dies before a running reconnect loop has installed its
        // handles, the loop reads the cancelled token as a failed dial.
        if !cancel_clone.is_cancelled() {
            cancel_clone.cancel();
            if let Some(app) = app_handle {
                super::reconnect::after_unexpected_disconnect(app, reconnect_target).await;
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
const AUDIO_SEND_QUEUE_DEPTH: usize = 4;

#[derive(Default)]
struct AudioSendQueueState {
    frames: std::collections::VecDeque<Vec<u8>>,
    closed: bool,
}

#[derive(Default)]
struct AudioSendQueue {
    state: std::sync::Mutex<AudioSendQueueState>,
    ready: tokio::sync::Notify,
}

impl AudioSendQueue {
    fn push_latest(&self, frame: Vec<u8>) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return;
        }
        if state.frames.len() == AUDIO_SEND_QUEUE_DEPTH {
            state.frames.pop_front();
        }
        state.frames.push_back(frame);
        drop(state);
        self.ready.notify_one();
    }

    fn close(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closed = true;
        self.ready.notify_waiters();
    }

    async fn pop(&self) -> Option<Vec<u8>> {
        loop {
            let notified = self.ready.notified();
            {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                if let Some(frame) = state.frames.pop_front() {
                    return Some(frame);
                }
                if state.closed {
                    return None;
                }
            }
            notified.await;
        }
    }
}

async fn wire_send_loop<S>(
    queue: std::sync::Arc<AudioSendQueue>,
    sink: std::sync::Arc<tokio::sync::Mutex<S>>,
) -> Result<(), String>
where
    S: futures_util::Sink<WsMsg> + Unpin,
    S::Error: std::fmt::Display,
{
    while let Some(frame) = queue.pop().await {
        let mut sink = sink.lock().await;
        sink.send(WsMsg::Binary(frame.into()))
            .await
            .map_err(|error| format!("audio send: {error}"))?;
    }
    Ok(())
}

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
    .await
    .map_err(|error| error.to_string())?;

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
    use super::wire::{audio_level_dbov, FrameHeader, V2_HEADER_LEN};
    use std::sync::atomic::Ordering;

    let mut encoder = opus::Encoder::new(48_000, opus::Channels::Mono, opus::Application::Voip)
        .map_err(|error| format!("tts opus encoder: {error}"))?;
    encoder
        .set_bitrate(opus::Bitrate::Bits(32_000))
        .map_err(|error| format!("tts opus bitrate: {error}"))?;
    encoder
        .set_dtx(true)
        .map_err(|error| format!("tts opus dtx: {error}"))?;

    let mut sequence = 0_u16;
    let mut timestamp_48k = 0_u32;
    let mut encoded = vec![0_u8; 4_000];
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
                let level = audio_level_dbov(&frame.samples_48k);
                let encoded_len = encoder
                    .encode_float(&frame.samples_48k, &mut encoded)
                    .map_err(|error| format!("tts opus encode: {error}"))?;
                if encoded_len == 0 {
                    continue;
                }
                let flags = if encoded_len <= 2 { super::wire::FLAG_DTX } else { 0 };
                let header = FrameHeader {
                    seq: sequence,
                    ts_48k: timestamp_48k,
                    level_dbov: level,
                    flags,
                }
                .encode();
                let mut payload = Vec::with_capacity(V2_HEADER_LEN + encoded_len);
                payload.extend_from_slice(&header);
                payload.extend_from_slice(&encoded[..encoded_len]);
                ws_tx
                    .send(WsMsg::Binary(payload.into()))
                    .await
                    .map_err(|error| format!("tts audio send: {error}"))?;
                sequence = sequence.wrapping_add(1);
                timestamp_48k = timestamp_48k.wrapping_add(super::jitter::FRAME_TIMESTAMP_DELTA);
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
    output_device_changes: tokio::sync::watch::Receiver<Option<String>>,
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
        output_device_changes,
    } = args;

    let mut encoder = opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Voip)
        .map_err(|e| format!("opus encoder: {e}"))?;
    encoder
        .set_bitrate(opus::Bitrate::Bits(32_000))
        .map_err(|e| format!("opus bitrate: {e}"))?;
    encoder
        .set_dtx(true)
        .map_err(|e| format!("opus dtx: {e}"))?;

    let sink_handle = super::audio_output::open_output_sink_by_name(output_device_name.as_deref())?;

    use std::sync::Arc as StdArc;
    let ws_tx = StdArc::new(tokio::sync::Mutex::new(ws_tx));
    let ws_tx_send = StdArc::clone(&ws_tx);
    let cancel_send = cancel.clone();
    let send_queue = StdArc::new(AudioSendQueue::default());
    let wire_queue = StdArc::clone(&send_queue);

    let wire_send_task = tokio::spawn(wire_send_loop(wire_queue, ws_tx_send));

    let encode_queue = StdArc::clone(&send_queue);
    let encode_task = tokio::spawn(async move {
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
                        eprintln!("buzz-desktop: opus encode error: {e}");
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
                    encode_queue.push_latest(frame);

                    seq = seq.wrapping_add(1);
                    ts_48k = ts_48k.wrapping_add(super::jitter::FRAME_TIMESTAMP_DELTA);
                }
            }
        }
        encode_queue.close();
    });

    let recv_task = tokio::spawn(super::playout::run_playout_recv_loop(
        ws_rx,
        ws_tx,
        sink_handle,
        cancel.clone(),
        app_handle,
        initial_peers,
        tts_active,
        tts_cancel,
        local_tts_publishers,
        remote_stt_pipeline,
        agent_pubkeys,
        human_floor.clone(),
        output_device_changes,
    ));

    supervise_audio_tasks(
        encode_task,
        wire_send_task,
        recv_task,
        &send_queue,
        &cancel,
        &human_floor,
    )
    .await
}

/// Every exit, including a child panic, joins the remaining children before
/// clearing this connection's remote floor or handing recovery to a successor.
async fn supervise_audio_tasks(
    mut encode: tokio::task::JoinHandle<()>,
    mut send: tokio::task::JoinHandle<Result<(), String>>,
    mut recv: tokio::task::JoinHandle<()>,
    queue: &AudioSendQueue,
    cancel: &CancellationToken,
    floor: &super::human_floor::HumanFloor,
) -> Result<(), String> {
    let (completed, result) = tokio::select! {
        result = &mut encode => (0, result.map_err(|error| format!("audio encode task: {error}"))),
        result = &mut send => (1, result.map_err(|error| format!("audio send task: {error}")).and_then(|result| result)),
        result = &mut recv => (2, result.map_err(|error| format!("audio receive task: {error}"))),
        _ = cancel.cancelled() => (3, Ok(())),
    };
    queue.close();
    encode.abort();
    send.abort();
    recv.abort();
    // A completed JoinHandle must not be polled a second time.
    if completed != 0 {
        log_audio_child_join("encode", encode.await);
    }
    if completed != 1 {
        log_audio_child_join("send", send.await);
    }
    if completed != 2 {
        log_audio_child_join("receive", recv.await);
    }
    floor.clear_remote();
    result
}

fn log_audio_child_join<T>(name: &str, result: Result<T, tokio::task::JoinError>) {
    if let Err(error) = result {
        if !error.is_cancelled() {
            eprintln!("buzz-desktop: audio {name} task failed during teardown: {error}");
        }
    }
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
#[path = "relay_api_tests.rs"]
mod tests;
