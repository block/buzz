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

use super::human_floor::HumanFloor;
use super::jitter::{PeerJitterBuffer, SAMPLE_RATE_HZ};
use super::relay_api::{WsStream, REMOTE_SPEECH_THRESHOLD};
use super::wire::{parse_relay_frame, FLAG_DTX};

/// Speaker-tick window for emitting `huddle-active-speakers`. Active set is
/// cleared each tick — peers that didn't send a frame in the last window are
/// considered silent.
const SPEAKER_TICK_MS: u64 = 500;
/// UI cadence for per-speaker waveform levels.
const SPEAKER_LEVEL_TICK_MS: u64 = 50;
/// Per-peer arrival window for the TTS interrupt frame counter.
const FRAME_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);
const REMOTE_RELEASE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);
/// Match Mobile's speaking treatment: an open microphone can emit continuous
/// non-DTX Opus for room tone, so packet type alone is not evidence of speech.
const REMOTE_SPEECH_LEVEL_DBOV: i8 = -55;
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

fn is_remote_speech_frame(is_dtx: bool, level_dbov: i8) -> bool {
    !is_dtx && level_dbov >= REMOTE_SPEECH_LEVEL_DBOV
}

fn update_remote_release_deadline(
    peer: u8,
    is_speech: bool,
    remote_floor_owners: &std::collections::HashSet<u8>,
    deadlines: &mut std::collections::HashMap<u8, tokio::time::Instant>,
    now: tokio::time::Instant,
) {
    if remote_floor_owners.contains(&peer) {
        if is_speech {
            // Refresh from audible speech itself. Some mobile capture paths
            // stop producing packets once speech ends, so waiting for a DTX
            // or quiet packet can otherwise hold the human floor forever.
            deadlines.insert(peer, now + REMOTE_RELEASE_DEBOUNCE);
        } else {
            // Preserve the deadline from the last audible frame. Continuous
            // room-tone packets must not keep extending the human floor.
            deadlines
                .entry(peer)
                .or_insert(now + REMOTE_RELEASE_DEBOUNCE);
        }
    }
}

fn release_expired_remote_floors(
    now: tokio::time::Instant,
    owners: &mut std::collections::HashSet<u8>,
    deadlines: &mut std::collections::HashMap<u8, tokio::time::Instant>,
    human_floor: &HumanFloor,
) {
    let released: Vec<u8> = deadlines
        .iter()
        .filter_map(|(peer, deadline)| (*deadline <= now).then_some(*peer))
        .collect();
    for peer in released {
        deadlines.remove(&peer);
        owners.remove(&peer);
        human_floor.leave_remote(peer);
    }
}

fn should_recover_playout(depth: usize, currently_recovering: bool) -> bool {
    if currently_recovering {
        depth > PLAYOUT_QUEUE_RECOVERY_END
    } else {
        depth >= PLAYOUT_QUEUE_RECOVERY_START
    }
}

fn is_locally_synthesized_peer(
    peer_idx: u8,
    local_tts_publishers: &super::tts::LocalTtsPublishers,
) -> bool {
    local_tts_publishers
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains_key(&peer_idx)
}

fn is_agent_peer(
    peer_idx: u8,
    index_to_pubkey: &std::collections::HashMap<u8, String>,
    agent_pubkeys: &[String],
) -> bool {
    index_to_pubkey.get(&peer_idx).is_some_and(|pubkey| {
        agent_pubkeys
            .iter()
            .any(|agent| agent.eq_ignore_ascii_case(pubkey))
    })
}

/// Whether `peer_idx` is currently occupied per the authoritative roster.
/// Protocol v2 media carries only the peer index, so roster presence is the
/// strongest routing boundary available until the relay supports v3 epochs.
fn is_current_occupant(peer_idx: u8, index_to_epoch: &std::collections::HashMap<u8, u8>) -> bool {
    index_to_epoch.contains_key(&peer_idx)
}

fn same_occupancy(
    peer_idx: u8,
    pubkey: &str,
    epoch: u8,
    index_to_pubkey: &std::collections::HashMap<u8, String>,
    index_to_epoch: &std::collections::HashMap<u8, u8>,
) -> bool {
    index_to_pubkey
        .get(&peer_idx)
        .is_some_and(|current| current == pubkey)
        && index_to_epoch.get(&peer_idx) == Some(&epoch)
}

fn mix_remote_stt_samples(mix: &mut Vec<f32>, samples: &[f32]) {
    if mix.len() < samples.len() {
        mix.resize(samples.len(), 0.0);
    }
    for (mixed, sample) in mix.iter_mut().zip(samples) {
        *mixed = (*mixed + *sample).clamp(-1.0, 1.0);
    }
}

fn f32_samples_to_le_bytes(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(samples));
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// One remote peer's slot: jitter buffer + dedicated rodio Player.
///
/// Per-frame seq/timestamp come from the v2 wire header (sender-authored).
/// The relay forwards `peer_index | header | opus_bytes` opaquely; we
/// parse the header here and pass the sender's own monotonic seq + 48 kHz media
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
    initial_peers: Vec<(u8, String, u8)>,
    tts_active: Arc<AtomicBool>,
    tts_cancel: Arc<AtomicBool>,
    local_tts_publishers: super::tts::LocalTtsPublishers,
    remote_stt_pipeline: Arc<std::sync::Mutex<Option<std::sync::Weak<super::stt::SttPipeline>>>>,
    agent_pubkeys: Arc<std::sync::Mutex<Vec<String>>>,
    human_floor: HumanFloor,
) {
    use rodio::buffer::SamplesBuffer;
    use std::num::NonZero;

    let mut peers: std::collections::HashMap<u8, PeerSlot> = std::collections::HashMap::new();
    let channels = NonZero::new(1u16).expect("1 is non-zero");
    let rate = NonZero::new(SAMPLE_RATE_HZ).expect("48k is non-zero");

    let mut index_to_pubkey: std::collections::HashMap<u8, String> =
        std::collections::HashMap::new();
    // Occupancy epoch per index, mirroring the authoritative roster. Advances
    // each time a slot is reused by a new occupant, so a frame authored by a
    // departed occupant that arrives after its index is reassigned carries the
    // old epoch and is fenced rather than mis-attributed to the new occupant.
    let mut index_to_epoch: std::collections::HashMap<u8, u8> = std::collections::HashMap::new();
    for (idx, pubkey, epoch) in initial_peers {
        index_to_pubkey.insert(idx, pubkey);
        index_to_epoch.insert(idx, epoch);
    }
    let mut active_indices: std::collections::HashSet<u8> = std::collections::HashSet::new();
    let mut speaker_levels: std::collections::HashMap<u8, f32> = std::collections::HashMap::new();
    let mut remote_release_deadlines: std::collections::HashMap<u8, tokio::time::Instant> =
        std::collections::HashMap::new();
    let mut remote_floor_owners: std::collections::HashSet<u8> = std::collections::HashSet::new();
    let mut frame_counts: std::collections::HashMap<u8, u16> = std::collections::HashMap::new();
    let mut last_frame_reset = tokio::time::Instant::now();

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
                let mut remote_stt_mix = Vec::new();
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
                            if !is_locally_synthesized_peer(*peer_idx, &local_tts_publishers) {
                                let remote_agent = {
                                    let agents = agent_pubkeys
                                        .lock()
                                        .unwrap_or_else(|error| error.into_inner());
                                    is_agent_peer(*peer_idx, &index_to_pubkey, &agents)
                                };
                                if !remote_agent {
                                    mix_remote_stt_samples(&mut remote_stt_mix, &samples);
                                }
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
                if !remote_stt_mix.is_empty() {
                    let pipeline = remote_stt_pipeline
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .as_ref()
                        .and_then(std::sync::Weak::upgrade);
                    if let Some(pipeline) = pipeline {
                        let _ = pipeline.push_remote_audio(f32_samples_to_le_bytes(
                            &remote_stt_mix,
                        ));
                    }
                }
            }
            _ = speaker_tick.tick() => {
                release_expired_remote_floors(
                    tokio::time::Instant::now(),
                    &mut remote_floor_owners,
                    &mut remote_release_deadlines,
                    &human_floor,
                );
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
                        // The minimum size is 1 (peer index) + 8 (header) + ≥1 Opus byte.
                        let Some((peer_idx, header, opus_bytes)) = parse_relay_frame(&data) else {
                            eprintln!(
                                "buzz-desktop: dropping malformed v2 audio relay frame ({} bytes)",
                                data.len(),
                            );
                            continue;
                        };
                        // Protocol v2 has no media epoch. Drop frames for slots
                        // absent from the control roster; delayed frames after
                        // an index is reassigned cannot be fenced until v3.
                        if !is_current_occupant(peer_idx, &index_to_epoch) {
                            continue;
                        }
                        // Suppress only an agent stream synthesized and
                        // published by this desktop. Other bot-role peers may
                        // publish their own legitimate audio and must play.
                        if is_locally_synthesized_peer(peer_idx, &local_tts_publishers) {
                            continue;
                        }
                        let is_dtx = (header.flags & FLAG_DTX) != 0;
                        let is_remote_speech =
                            is_remote_speech_frame(is_dtx, header.level_dbov);
                        // Only count audible arrivals toward the UI's
                        // active-speaker set. An open mobile microphone can
                        // continuously emit non-DTX room tone, so require an
                        // audible level before treating a packet as speech.
                        update_remote_release_deadline(
                            peer_idx,
                            is_remote_speech,
                            &remote_floor_owners,
                            &mut remote_release_deadlines,
                            tokio::time::Instant::now(),
                        );
                        if is_remote_speech {
                            active_indices.insert(peer_idx);
                            let level = normalized_speaker_level(header.level_dbov);
                            speaker_levels
                                .entry(peer_idx)
                                .and_modify(|current| *current = current.max(level))
                                .or_insert(level);
                        }

                        // Track remote speech independently of TTS liveness so a
                        // human who starts while output is idle still owns the
                        // floor and rejects delayed synthesis.
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

                        let remote_human = {
                            let agents = agent_pubkeys
                                .lock()
                                .unwrap_or_else(|error| error.into_inner());
                            !is_agent_peer(peer_idx, &index_to_pubkey, &agents)
                        };
                        // Count only remote-human speech toward floor onset.
                        // Agent audio still plays, but it must not acquire the
                        // human floor or suppress another agent's response.
                        if is_remote_speech && remote_human {
                            if last_frame_reset.elapsed() >= FRAME_WINDOW {
                                frame_counts.clear();
                                last_frame_reset = tokio::time::Instant::now();
                            }
                            let count = frame_counts.entry(peer_idx).or_insert(0);
                            *count = count.saturating_add(1);
                            if *count >= REMOTE_SPEECH_THRESHOLD {
                                human_floor.enter_remote(peer_idx);
                                remote_floor_owners.insert(peer_idx);
                                // The threshold-crossing frame is processed
                                // before this peer becomes an owner. Arm its
                                // release here so silence need not arrive in a
                                // later packet to let queued TTS continue.
                                remote_release_deadlines.insert(
                                    peer_idx,
                                    tokio::time::Instant::now() + REMOTE_RELEASE_DEBOUNCE,
                                );
                                if tts_active.load(Ordering::Acquire) {
                                    tts_cancel.store(true, Ordering::Release);
                                }
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
                                                // Absent `epoch` (legacy relay) degrades to
                                                // 0 so the fence stays a no-op.
                                                let epoch =
                                                    p["epoch"].as_u64().unwrap_or(0) as u8;
                                                // Any new occupancy (pubkey or epoch) must
                                                // flush the old peer's NetEq + Player so
                                                // the next frame starts clean.
                                                if !same_occupancy(
                                                    key,
                                                    pk,
                                                    epoch,
                                                    &index_to_pubkey,
                                                    &index_to_epoch,
                                                ) {
                                                    peers.remove(&key);
                                                    frame_counts.remove(&key);
                                                    remote_release_deadlines.remove(&key);
                                                    remote_floor_owners.remove(&key);
                                                    human_floor.leave_remote(key);
                                                    active_indices.remove(&key);
                                                    speaker_levels.remove(&key);
                                                }
                                                index_to_pubkey.insert(key, pk.to_string());
                                                index_to_epoch.insert(key, epoch);
                                            }
                                        }
                                    }
                                }
                                Some("roster") => {
                                    if let Some(peer_list) = v["peers"].as_array() {
                                        let mut replacement = std::collections::HashMap::new();
                                        let mut replacement_epochs =
                                            std::collections::HashMap::new();
                                        for p in peer_list {
                                            if let (Some(pk), Some(idx)) = (
                                                p["pubkey"].as_str(),
                                                p["peer_index"].as_u64(),
                                            ) {
                                                let key = idx as u8;
                                                let epoch =
                                                    p["epoch"].as_u64().unwrap_or(0) as u8;
                                                replacement.insert(key, pk.to_string());
                                                replacement_epochs.insert(key, epoch);
                                            }
                                        }
                                        let identity_unchanged = |idx: &u8| {
                                            replacement.get(idx).is_some_and(|pubkey| {
                                                replacement_epochs.get(idx).is_some_and(|epoch| {
                                                    same_occupancy(
                                                        *idx,
                                                        pubkey,
                                                        *epoch,
                                                        &index_to_pubkey,
                                                        &index_to_epoch,
                                                    )
                                                })
                                            })
                                        };
                                        peers.retain(|idx, _| identity_unchanged(idx));
                                        for idx in index_to_pubkey
                                            .keys()
                                            .filter(|idx| !identity_unchanged(idx))
                                            .copied()
                                            .collect::<Vec<_>>()
                                        {
                                            human_floor.leave_remote(idx);
                                            remote_release_deadlines.remove(&idx);
                                            remote_floor_owners.remove(&idx);
                                        }
                                        frame_counts.retain(|idx, _| identity_unchanged(idx));
                                        active_indices.retain(identity_unchanged);
                                        speaker_levels.retain(|idx, _| identity_unchanged(idx));
                                        index_to_pubkey = replacement;
                                        index_to_epoch = replacement_epochs;
                                    }
                                }
                                Some("left") => {
                                    if let Some(idx) = v["peer_index"].as_u64() {
                                        let key = idx as u8;
                                        index_to_pubkey.remove(&key);
                                        index_to_epoch.remove(&key);
                                        frame_counts.remove(&key);
                                        remote_release_deadlines.remove(&key);
                                        remote_floor_owners.remove(&key);
                                        human_floor.leave_remote(key);
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

    human_floor.clear_remote();
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
    fn continuous_silence_does_not_extend_remote_floor_deadline() {
        let peer = 7;
        let started = tokio::time::Instant::now();
        let owners = std::collections::HashSet::from([peer]);
        let mut deadlines = std::collections::HashMap::new();

        update_remote_release_deadline(peer, false, &owners, &mut deadlines, started);
        let armed = deadlines[&peer];
        for elapsed_ms in [100, 200, 300, 400] {
            update_remote_release_deadline(
                peer,
                false,
                &owners,
                &mut deadlines,
                started + std::time::Duration::from_millis(elapsed_ms),
            );
        }

        assert_eq!(deadlines[&peer], armed);
        assert!(armed <= started + REMOTE_RELEASE_DEBOUNCE);

        let human_floor = HumanFloor::new();
        human_floor.enter_remote(peer);
        let mut owners = owners;
        release_expired_remote_floors(armed, &mut owners, &mut deadlines, &human_floor);
        assert!(!human_floor.is_blocked());
        assert!(owners.is_empty());
        assert!(deadlines.is_empty());
    }

    #[test]
    fn last_speech_frame_arms_remote_floor_release_without_follow_up_audio() {
        let peer = 7;
        let started = tokio::time::Instant::now();
        let owners = std::collections::HashSet::from([peer]);
        let mut deadlines = std::collections::HashMap::new();

        update_remote_release_deadline(peer, true, &owners, &mut deadlines, started);
        let armed = started + REMOTE_RELEASE_DEBOUNCE;
        assert_eq!(deadlines[&peer], armed);

        let human_floor = HumanFloor::new();
        human_floor.enter_remote(peer);
        let mut owners = owners;
        release_expired_remote_floors(armed, &mut owners, &mut deadlines, &human_floor);

        assert!(!human_floor.is_blocked());
        assert!(owners.is_empty());
        assert!(deadlines.is_empty());
    }

    #[test]
    fn silence_from_non_owner_does_not_arm_remote_floor_deadline() {
        let mut deadlines = std::collections::HashMap::new();
        update_remote_release_deadline(
            7,
            false,
            &std::collections::HashSet::new(),
            &mut deadlines,
            tokio::time::Instant::now(),
        );
        assert!(deadlines.is_empty());
    }

    #[test]
    fn remote_speech_requires_non_dtx_audio_above_the_activity_floor() {
        assert!(!is_remote_speech_frame(true, 0));
        assert!(!is_remote_speech_frame(false, -127));
        assert!(!is_remote_speech_frame(false, -56));
        assert!(is_remote_speech_frame(false, -55));
        assert!(is_remote_speech_frame(false, -12));
    }

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

    #[test]
    fn only_the_local_socket_is_suppressed_for_a_shared_agent_identity() {
        let local_publishers = super::super::tts::LocalTtsPublishers::default();
        local_publishers
            .lock()
            .expect("local publishers")
            .insert(3, 1);

        assert!(is_locally_synthesized_peer(3, &local_publishers));
        assert!(
            !is_locally_synthesized_peer(4, &local_publishers),
            "a second socket for the same agent remains audible"
        );
        assert!(!is_locally_synthesized_peer(9, &local_publishers));
    }

    #[test]
    fn remote_agent_identity_is_excluded_from_human_stt() {
        let peers =
            std::collections::HashMap::from([(3, "human".to_owned()), (4, "AGENT".to_owned())]);
        let agents = vec!["agent".to_owned()];

        assert!(!is_agent_peer(3, &peers, &agents));
        assert!(is_agent_peer(4, &peers, &agents));
        assert!(!is_agent_peer(9, &peers, &agents));
    }

    #[test]
    fn occupancy_identity_includes_epoch_for_same_pubkey_rejoin() {
        let pubkeys = std::collections::HashMap::from([(3_u8, "alice".to_owned())]);
        let epochs = std::collections::HashMap::from([(3_u8, 4_u8)]);

        assert!(same_occupancy(3, "alice", 4, &pubkeys, &epochs));
        assert!(
            !same_occupancy(3, "alice", 5, &pubkeys, &epochs),
            "same pubkey with a new epoch must reset decoder and playout state"
        );
    }

    #[test]
    fn v2_media_is_routed_only_for_current_roster_indices() {
        let mut index_to_epoch = std::collections::HashMap::new();
        index_to_epoch.insert(3_u8, 0_u8);
        assert!(
            is_current_occupant(3, &index_to_epoch),
            "current occupant's frame is delivered"
        );
        assert!(
            !is_current_occupant(9, &index_to_epoch),
            "frame for an unoccupied index is dropped"
        );
    }

    #[test]
    fn remote_human_stt_mix_sums_and_clamps_concurrent_speakers() {
        let mut mix = Vec::new();
        mix_remote_stt_samples(&mut mix, &[0.4, -0.7, 0.2]);
        mix_remote_stt_samples(&mut mix, &[0.8, -0.6, -0.1]);

        assert_eq!(mix, vec![1.0, -1.0, 0.1]);
        let bytes = f32_samples_to_le_bytes(&mix);
        assert_eq!(bytes.len(), std::mem::size_of_val(mix.as_slice()));
        assert_eq!(f32::from_le_bytes(bytes[0..4].try_into().unwrap()), 1.0);
    }
}
