//! Receive-side playout loop for the huddle audio relay.
//!
//! Owns the per-peer state map (one `NetEq` + one `rodio::Player` per remote
//! peer), the 10 ms playout clock, and the 500 ms active-speaker tick. Sibling
//! to [`relay_api`](super::relay_api), which keeps the encode/send half.
//!
//! ## Architecture
//!
//! ```text
//!   WS binary frame ──► insert_packet ──► NetEq jitter buffer
//!                                              │
//!                       playout_tick (10 ms) ──┘──► get_audio ─► per-peer
//!                                                                rodio::Player
//!                                                                    │
//!                                                                    ▼
//!                                                            device mixer (sums
//!                                                            concurrent peers)
//! ```
//!
//! The pre-fix shape used a single `rodio::Player` shared across every peer.
//! `Player` is a FIFO queue, so 3+ simultaneous speakers serialized into one
//! voice flipping speakers every 20 ms with unbounded queue growth. See
//! `desktop/src-tauri/tests/rodio_mixer_diagnostic.rs` for the deterministic
//! repro that pins this diagnosis in CI.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tokio_util::sync::CancellationToken;

use super::jitter::{PeerJitterBuffer, FRAME_TIMESTAMP_DELTA, SAMPLE_RATE_HZ};
use super::relay_api::{WsStream, REMOTE_SPEECH_THRESHOLD};

/// Speaker-tick window for emitting `huddle-active-speakers`. Active set is
/// cleared each tick — peers that didn't send a frame in the last window are
/// considered silent.
const SPEAKER_TICK_MS: u64 = 500;
/// Per-peer arrival window for the TTS interrupt frame counter.
const FRAME_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);
/// Playout clock: NetEq emits 10 ms frames, so we tick at 10 ms.
const PLAYOUT_TICK_MS: u64 = 10;

/// One remote peer's slot: jitter buffer, dedicated rodio Player, and the
/// synthesized seq/timestamp pair we feed NetEq for v1 wire frames.
///
/// On v1 wire (this commit) the protocol carries no per-frame seq/ts, so we
/// generate them locally. The WebSocket is over TCP — frames arrive in order
/// end-to-end — so monotonic-on-arrival is a safe approximation. Protocol v2
/// (next commit) replaces these with sender-authored values.
struct PeerSlot {
    jitter: PeerJitterBuffer,
    player: rodio::Player,
    seq: u16,
    ts_48k: u32,
}

impl PeerSlot {
    fn new(peer_idx: u8, sink_mixer: &rodio::mixer::Mixer) -> Option<Self> {
        match PeerJitterBuffer::new(peer_idx) {
            Ok(jitter) => Some(Self {
                jitter,
                player: rodio::Player::connect_new(sink_mixer),
                seq: 0,
                ts_48k: 0,
            }),
            Err(e) => {
                eprintln!("sprout-desktop: jitter buffer init peer {peer_idx}: {e}");
                None
            }
        }
    }
}

/// Drive the receive loop until cancelled or the WS closes.
///
/// `ws_tx_for_pongs` is shared with the encode-side task and only used here to
/// reply to Pings; it is locked briefly per Ping and never held across the
/// audio fast path.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_playout_recv_loop(
    mut ws_rx: futures_util::stream::SplitStream<WsStream>,
    ws_tx_for_pongs: Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WsStream, WsMsg>>>,
    sink_handle: rodio::MixerDeviceSink,
    cancel: CancellationToken,
    app_handle: Option<tauri::AppHandle>,
    initial_peers: Vec<(u8, String)>,
    tts_active: Arc<AtomicBool>,
    tts_cancel: Arc<AtomicBool>,
) {
    use rodio::buffer::SamplesBuffer;
    use std::num::NonZero;

    let mut peers: std::collections::HashMap<u8, PeerSlot> = std::collections::HashMap::new();
    let channels = NonZero::new(1u16).expect("1 is non-zero");
    let rate = NonZero::new(SAMPLE_RATE_HZ).expect("48k is non-zero");

    let mut index_to_pubkey: std::collections::HashMap<u8, String> =
        initial_peers.into_iter().collect();
    let mut active_indices: std::collections::HashSet<u8> = std::collections::HashSet::new();
    let mut frame_counts: std::collections::HashMap<u8, u16> = std::collections::HashMap::new();
    let mut last_frame_reset = tokio::time::Instant::now();
    let mut tts_was_active = false;

    let mut speaker_tick = tokio::time::interval(std::time::Duration::from_millis(SPEAKER_TICK_MS));
    speaker_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut playout_tick = tokio::time::interval(std::time::Duration::from_millis(PLAYOUT_TICK_MS));
    playout_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = playout_tick.tick() => {
                // Drain one 10 ms frame from each peer's NetEq into its Player.
                // NetEq's contract is to always emit a frame (Expand/silence
                // when empty), so the audio device's pull from the mixer never
                // starves.
                for (peer_idx, slot) in peers.iter_mut() {
                    match slot.jitter.get_audio() {
                        Ok((samples, _vad)) => {
                            slot.player.append(SamplesBuffer::new(channels, rate, samples));
                        }
                        Err(e) => {
                            eprintln!(
                                "sprout-desktop: jitter get_audio peer {peer_idx}: {e}"
                            );
                        }
                    }
                }
            }
            _ = speaker_tick.tick() => {
                if let Some(ref app) = app_handle {
                    use tauri::Emitter;
                    let pubkeys: Vec<String> = active_indices
                        .iter()
                        .filter_map(|idx| index_to_pubkey.get(idx).cloned())
                        .collect();
                    let _ = app.emit("huddle-active-speakers", &pubkeys);
                }
                active_indices.clear();
            }
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(WsMsg::Binary(data))) => {
                        if data.len() < 2 {
                            continue;
                        }
                        let peer_idx = data[0];
                        let opus_bytes = &data[1..];
                        active_indices.insert(peer_idx);

                        // TTS interrupt frame counter — reset on TTS rising edge.
                        let tts_now = tts_active.load(Ordering::Acquire);
                        if tts_now && !tts_was_active {
                            frame_counts.clear();
                            last_frame_reset = tokio::time::Instant::now();
                        }
                        tts_was_active = tts_now;

                        let slot = match peers.entry(peer_idx) {
                            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                            std::collections::hash_map::Entry::Vacant(e) => {
                                let Some(slot) = PeerSlot::new(peer_idx, sink_handle.mixer())
                                else {
                                    continue;
                                };
                                e.insert(slot)
                            }
                        };

                        if let Err(err) =
                            slot.jitter.insert_packet(slot.seq, slot.ts_48k, opus_bytes)
                        {
                            eprintln!(
                                "sprout-desktop: jitter insert peer {peer_idx}: {err}"
                            );
                        } else {
                            slot.seq = slot.seq.wrapping_add(1);
                            slot.ts_48k = slot.ts_48k.wrapping_add(FRAME_TIMESTAMP_DELTA);
                        }

                        if tts_now {
                            if last_frame_reset.elapsed() >= FRAME_WINDOW {
                                frame_counts.clear();
                                last_frame_reset = tokio::time::Instant::now();
                            }
                            let count = frame_counts.entry(peer_idx).or_insert(0);
                            *count = count.saturating_add(1);
                            if *count >= REMOTE_SPEECH_THRESHOLD {
                                tts_cancel.store(true, Ordering::Release);
                            }
                        }
                    }
                    Some(Ok(WsMsg::Text(text))) => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                            match v["type"].as_str() {
                                Some("joined") => {
                                    if let Some(peer_list) = v["peers"].as_array() {
                                        for p in peer_list {
                                            if let (Some(pk), Some(idx)) = (
                                                p["pubkey"].as_str(),
                                                p["peer_index"].as_u64(),
                                            ) {
                                                let key = idx as u8;
                                                // peer_index reuse with a new pubkey:
                                                // flush the old peer's NetEq + Player so
                                                // the next frame starts clean.
                                                if index_to_pubkey
                                                    .get(&key)
                                                    .map(|s| s.as_str())
                                                    != Some(pk)
                                                {
                                                    peers.remove(&key);
                                                    frame_counts.remove(&key);
                                                    active_indices.remove(&key);
                                                }
                                                index_to_pubkey.insert(key, pk.to_string());
                                            }
                                        }
                                    }
                                }
                                Some("left") => {
                                    if let Some(idx) = v["peer_index"].as_u64() {
                                        let key = idx as u8;
                                        index_to_pubkey.remove(&key);
                                        frame_counts.remove(&key);
                                        // Dropping Player detaches its queue from the
                                        // device mixer, freeing the per-peer slot.
                                        peers.remove(&key);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(WsMsg::Ping(data))) => {
                        let mut tx = ws_tx_for_pongs.lock().await;
                        let _ = tx.send(WsMsg::Pong(data)).await;
                    }
                    Some(Ok(WsMsg::Close(_))) | None => break,
                    Some(Ok(_)) => {}    // non-binary/text frame
                    Some(Err(_)) => break,
                }
            }
        }
    }
}
