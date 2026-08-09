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

use super::jitter::{PeerJitterBuffer, SAMPLE_RATE_HZ};
use super::relay_api::{WsStream, REMOTE_SPEECH_THRESHOLD};
use super::wire::{FrameHeader, FLAG_DTX, V2_HEADER_LEN};

/// Speaker-tick window for emitting `huddle-active-speakers`. Active set is
/// cleared each tick — peers that didn't send a frame in the last window are
/// considered silent.
const SPEAKER_TICK_MS: u64 = 500;
/// UI cadence for per-speaker waveform levels.
const SPEAKER_LEVEL_TICK_MS: u64 = 50;
/// Per-peer arrival window for the TTS interrupt frame counter.
const FRAME_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);
/// Playout clock: NetEq emits 10 ms frames, so we tick at 10 ms.
const PLAYOUT_TICK_MS: u64 = 10;

/// How long after the last received packet we keep pulling frames out of a
/// peer's NetEq into its rodio Player. NetEq always emits a frame on every
/// `get_audio` call — silent/Expand when there are no packets — so without
/// this bound an idle peer (one that disconnected without sending `left`)
/// would have its Player queue 100 silence buffers/sec forever. We pull for
/// a short grace window past the last packet so brief DTX gaps still feed
/// NetEq's PLC/expand path normally.
const IDLE_PEER_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// Queue-depth thresholds for smooth producer/device clock-drift recovery.
///
/// The playout pipeline has two clocks: the producer is a `tokio` 10 ms
/// interval (this loop) that pulls from NetEq and appends to each peer's
/// `Player`; the consumer is the `cpal` audio callback that pulls samples
/// from the device `Mixer` at hardware sample rate. NetEq does rate-adapt
/// (accelerate / expand) but only to its own input pattern — it cannot
/// see the actual device-side consumption rate.
///
/// In steady state producer ≈ consumer, but scheduler jitter or small
/// clock skew can leave the producer slightly ahead, and rodio's
/// `Player` queue is an unbounded MPSC under the hood. Over a long call
/// that drift would accumulate as monotonic added latency (and eventually
/// memory).
///
/// Dropping a whole 10 ms buffer at a shallow queue depth creates a waveform
/// discontinuity that is audible as a click or static. Once the queue grows
/// beyond the recovery threshold, play it 2% faster until it returns to the
/// target. A hard drop remains only as an emergency bound at 300 ms.
const PLAYOUT_QUEUE_RECOVERY_START: usize = 10;
const PLAYOUT_QUEUE_RECOVERY_END: usize = 4;
const PLAYOUT_QUEUE_EMERGENCY_HIGH_WATER: usize = 30;
const PLAYOUT_RECOVERY_SPEED: f32 = 1.02;

/// Map sender-authored dBov into a useful UI range. Normal conversational
/// speech generally sits between roughly -60 dBov and -12 dBov.
fn normalized_speaker_level(level_dbov: i8) -> f32 {
    ((f32::from(level_dbov) + 60.0) / 48.0).clamp(0.0, 1.0)
}

fn should_recover_playout(depth: usize, currently_recovering: bool) -> bool {
    if currently_recovering {
        depth > PLAYOUT_QUEUE_RECOVERY_END
    } else {
        depth >= PLAYOUT_QUEUE_RECOVERY_START
    }
}

/// One remote peer's slot: jitter buffer + dedicated rodio Player.
///
/// Per-frame seq/timestamp come from the v2 wire header (sender-authored).
/// The relay forwards `peer_index | header | opus_bytes` opaquely; we parse
/// the header here and pass the sender's own monotonic seq + 48 kHz media
/// timestamp into NetEq.
struct PeerSlot {
    jitter: PeerJitterBuffer,
    player: rodio::Player,
    /// Wall-clock time of the most recent inbound packet for this peer. Read
    /// by the playout tick to decide whether to keep draining NetEq into the
    /// Player. Updated on every successful `insert_packet`.
    last_packet_at: tokio::time::Instant,
    recovering_playout: bool,
}

impl PeerSlot {
    fn new(peer_idx: u8, sink_mixer: &rodio::mixer::Mixer) -> Option<Self> {
        match PeerJitterBuffer::new(peer_idx) {
            Ok(jitter) => Some(Self {
                jitter,
                player: rodio::Player::connect_new(sink_mixer),
                last_packet_at: tokio::time::Instant::now(),
                recovering_playout: false,
            }),
            Err(e) => {
                eprintln!("buzz-desktop: jitter buffer init peer {peer_idx}: {e}");
                None
            }
        }
    }

    /// Whether this peer is still actively sending — used by the playout tick
    /// to gate the rodio append so disconnected peers don't pump silence
    /// indefinitely.
    ///
    /// The gate is `recent packet OR jitter buffer not empty`. The recent-
    /// packet half covers the common case: brief speech gaps and DTX cadence
    /// (≤400 ms) stay inside the [`IDLE_PEER_GRACE`] window so PLC/expand
    /// frames keep flowing. The buffer-not-empty half is a safety net for
    /// the edge case Mari called out: a peer who sends a burst then
    /// disconnects has real audio queued in NetEq that should still play
    /// out, even if `last_packet_at` ages past the grace before the buffer
    /// finishes draining. The grace alone is enough today because NetEq's
    /// `max_delay_ms` (200 ms) is well inside the grace (500 ms), but the
    /// OR keeps the invariant robust against future config tuning.
    fn is_active(&self) -> bool {
        self.last_packet_at.elapsed() < IDLE_PEER_GRACE || !self.jitter.is_empty()
    }

    fn update_playout_recovery(&mut self) {
        let should_recover = should_recover_playout(self.player.len(), self.recovering_playout);
        if should_recover == self.recovering_playout {
            return;
        }
        self.recovering_playout = should_recover;
        self.player.set_speed(if should_recover {
            PLAYOUT_RECOVERY_SPEED
        } else {
            1.0
        });
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
    let mut speaker_levels: std::collections::HashMap<u8, f32> = std::collections::HashMap::new();
    let mut frame_counts: std::collections::HashMap<u8, u16> = std::collections::HashMap::new();
    let mut last_frame_reset = tokio::time::Instant::now();
    let mut tts_was_active = false;

    let mut speaker_tick = tokio::time::interval(std::time::Duration::from_millis(SPEAKER_TICK_MS));
    speaker_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut speaker_level_tick =
        tokio::time::interval(std::time::Duration::from_millis(SPEAKER_LEVEL_TICK_MS));
    speaker_level_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut playout_tick = tokio::time::interval(std::time::Duration::from_millis(PLAYOUT_TICK_MS));
    // `Delay` (not `Skip`) so a brief stall in another select arm — e.g. the
    // ws_tx_for_pongs mutex contending with the encode-side task on a Ping —
    // doesn't drop a playout tick outright. Dropped ticks would leave the
    // per-peer Player queues empty for 10 ms and the device mixer would
    // produce audible silence. `Delay` catches up immediately when the loop
    // returns to the select.
    playout_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = playout_tick.tick() => {
                // Drain one 10 ms frame from each *active* peer's NetEq into
                // its Player. NetEq always emits a frame (Expand/silence when
                // empty), so for peers that recently sent we keep the device
                // mixer fed without starving; for peers that have stopped
                // sending — disconnected without a `left`, or simply quiet —
                // we skip the append so we don't pump 100 silence buffers/sec
                // per idle peer into rodio forever. `is_active` is a 500 ms
                // grace past the last received packet, far longer than typical
                // DTX comfort-noise cadence.
                for (peer_idx, slot) in peers.iter_mut() {
                    if !slot.is_active() {
                        // Still drain the frame to keep NetEq's internal clock
                        // advancing; just don't enqueue it for playback.
                        let _ = slot.jitter.get_audio();
                        continue;
                    }
                    match slot.jitter.get_audio() {
                        Ok((samples, _vad)) => {
                            // Smooth out producer-vs-device clock drift. A
                            // shallow hard drop used to remove entire 10 ms
                            // chunks and create audible discontinuities.
                            slot.update_playout_recovery();
                            if slot.player.len() >= PLAYOUT_QUEUE_EMERGENCY_HIGH_WATER {
                                eprintln!(
                                    "buzz-desktop: playout queue emergency high-water for peer \
                                     {peer_idx} (depth={}) — dropping oldest frame",
                                    slot.player.len(),
                                );
                                slot.player.skip_one();
                            }
                            slot.player.append(SamplesBuffer::new(channels, rate, samples));
                        }
                        Err(e) => {
                            eprintln!(
                                "buzz-desktop: jitter get_audio peer {peer_idx}: {e}"
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
            _ = speaker_level_tick.tick() => {
                if let Some(ref app) = app_handle {
                    use tauri::Emitter;
                    let levels: std::collections::HashMap<String, f32> = speaker_levels
                        .iter()
                        .filter_map(|(idx, level)| {
                            index_to_pubkey.get(idx).cloned().map(|pubkey| (pubkey, *level))
                        })
                        .collect();
                    let _ = app.emit("huddle-speaker-levels", &levels);
                }
                for level in speaker_levels.values_mut() {
                    *level *= 0.55;
                }
                speaker_levels.retain(|_, level| *level > 0.015);
            }
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(WsMsg::Binary(data))) => {
                        // Wire shape (v2): [peer_index: u8][header: 8 bytes][opus payload...]
                        // The minimum size is 1 (peer_index) + 8 (header) + ≥1 Opus byte.
                        if data.len() <= 1 + V2_HEADER_LEN {
                            continue;
                        }
                        let peer_idx = data[0];
                        let after_idx = &data[1..];
                        let Some((header, opus_bytes)) = FrameHeader::parse(after_idx)
                        else {
                            // Malformed v2 frame: header parse only fails when
                            // the slice is too short, which `if data.len() <= ...`
                            // already guards. Defensive log + drop.
                            eprintln!(
                                "buzz-desktop: dropping malformed audio frame from peer {peer_idx} ({} bytes)",
                                data.len(),
                            );
                            continue;
                        };
                        if opus_bytes.is_empty() {
                            continue;
                        }
                        let is_dtx = (header.flags & FLAG_DTX) != 0;
                        // Only count non-DTX arrivals toward the UI's
                        // active-speaker set. DTX/comfort packets are emitted
                        // by an idle peer to keep the codec alive — they
                        // don't mean the peer is speaking, and shouldn't
                        // make their tile flash for the 500 ms speaker tick.
                        if !is_dtx {
                            active_indices.insert(peer_idx);
                            let level = normalized_speaker_level(header.level_dbov);
                            speaker_levels
                                .entry(peer_idx)
                                .and_modify(|current| *current = current.max(level))
                                .or_insert(level);
                        }

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

                        // Sender-authored seq/ts: NetEq can detect real
                        // packet reordering & loss, not just arrival jitter.
                        if let Err(err) =
                            slot.jitter
                                .insert_packet(header.seq, header.ts_48k, opus_bytes)
                        {
                            eprintln!(
                                "buzz-desktop: jitter insert peer {peer_idx}: {err}"
                            );
                        } else {
                            // Heartbeat for the playout tick's idle-peer
                            // guard — only on successful insert so a stream
                            // of bad packets can't keep a dead peer "active".
                            slot.last_packet_at = tokio::time::Instant::now();
                        }

                        // Count remote-speech frame arrivals for the TTS
                        // interrupt. DTX/comfort frames don't count — they
                        // mean the peer is silent, just keeping the codec
                        // state alive.
                        if tts_now && !is_dtx {
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
                                                    speaker_levels.remove(&key);
                                                }
                                                index_to_pubkey.insert(key, pk.to_string());
                                            }
                                        }
                                    }
                                }
                                Some("roster") => {
                                    if let Some(peer_list) = v["peers"].as_array() {
                                        let mut replacement = std::collections::HashMap::new();
                                        for p in peer_list {
                                            if let (Some(pk), Some(idx)) = (
                                                p["pubkey"].as_str(),
                                                p["peer_index"].as_u64(),
                                            ) {
                                                replacement.insert(idx as u8, pk.to_string());
                                            }
                                        }
                                        let identity_unchanged = |idx: &u8| {
                                            replacement.get(idx) == index_to_pubkey.get(idx)
                                        };
                                        peers.retain(|idx, _| identity_unchanged(idx));
                                        frame_counts.retain(|idx, _| identity_unchanged(idx));
                                        active_indices.retain(identity_unchanged);
                                        speaker_levels.retain(|idx, _| identity_unchanged(idx));
                                        index_to_pubkey = replacement;
                                    }
                                }
                                Some("left") => {
                                    if let Some(idx) = v["peer_index"].as_u64() {
                                        let key = idx as u8;
                                        index_to_pubkey.remove(&key);
                                        frame_counts.remove(&key);
                                        active_indices.remove(&key);
                                        speaker_levels.remove(&key);
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

    if let Some(ref app) = app_handle {
        use tauri::Emitter;
        let _ = app.emit(
            "huddle-speaker-levels",
            &std::collections::HashMap::<String, f32>::new(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaker_level_maps_conversational_range() {
        assert_eq!(normalized_speaker_level(-127), 0.0);
        assert_eq!(normalized_speaker_level(-60), 0.0);
        assert!((normalized_speaker_level(-36) - 0.5).abs() < f32::EPSILON);
        assert_eq!(normalized_speaker_level(-12), 1.0);
        assert_eq!(normalized_speaker_level(0), 1.0);
    }

    #[test]
    fn playout_recovery_uses_hysteresis() {
        assert!(!should_recover_playout(9, false));
        assert!(should_recover_playout(10, false));
        assert!(should_recover_playout(5, true));
        assert!(!should_recover_playout(4, true));
    }
}
