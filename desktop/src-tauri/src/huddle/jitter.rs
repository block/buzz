//! Per-peer NetEQ-style jitter buffer for huddle remote audio.
//!
//! Wraps the [`neteq`] crate's `NetEq` state machine and registers a thin
//! `AudioDecoder` impl over our existing `opus = "0.3"` dependency. Default
//! features on `neteq` are disabled so we don't pull in axum/cpal/clap; this
//! module owns the Opus side of the trait instead.
//!
//! ## Lifecycle
//!
//! One [`PeerJitterBuffer`] per active remote peer. It owns:
//!   * a `NetEq` configured for 48 kHz mono with conservative jitter bounds,
//!   * a stable synthetic RTP SSRC (the relay-assigned `peer_index`), and
//!   * a 16-bit sequence number that wraps with the wire protocol's `seq`.
//!
//! On `joined` / first packet, construct one. On `left` / rejoin, drop and
//! recreate so NetEq's internal delay state doesn't carry over.
//!
//! ## Decode path
//!
//! Each inbound Opus frame becomes a `neteq::AudioPacket` with:
//!   * sequence number from the protocol v2 header,
//!   * 48 kHz RTP-style media timestamp (frame_index * 960 for 20 ms frames),
//!   * a synthetic SSRC == peer_index,
//!   * payload type [`OPUS_PAYLOAD_TYPE`] (any value, only NetEq uses it to
//!     dispatch to its registered decoder),
//!   * the raw Opus bytes as `payload`,
//!   * `duration_ms = 20` (our encoder frame size).
//!
//! The playout side calls [`PeerJitterBuffer::get_audio`] on a 10 ms tick;
//! NetEq returns 10 ms of decoded PCM with adaptive-delay / expand / accelerate
//! decisions already applied. For 20 ms input frames at 48 kHz mono, that's
//! 480 f32 samples per call.

use neteq::{
    codec::AudioDecoder, neteq::SpeechType, AudioPacket, NetEq, NetEqConfig, NetEqError, RtpHeader,
};

/// RTP payload type used internally. NetEq only needs it to look up the decoder
/// registered via `register_decoder`; the relay forwards Opus payloads opaquely
/// so the wire never carries a payload-type byte.
pub const OPUS_PAYLOAD_TYPE: u8 = 111;

/// Audio sample rate for huddle (Opus VoIP, mono).
pub const SAMPLE_RATE_HZ: u32 = 48_000;
/// One channel (mono).
pub const CHANNELS: u8 = 1;
/// Opus encoder frame size in milliseconds. The wire format is one Opus packet
/// per 20 ms frame; the receiver still drains NetEq at 10 ms granularity.
pub const FRAME_DURATION_MS: u32 = 20;
/// 48 kHz media-time increment per encoder frame.
pub const FRAME_TIMESTAMP_DELTA: u32 = SAMPLE_RATE_HZ / 1000 * FRAME_DURATION_MS; // 960
/// NetEq returns 10 ms frames; this is the sample count per `get_audio` call.
/// Kept as a documented constant for the protocol-v2 follow-up and consumers
/// that want to size their own buffers around the playout contract.
#[allow(dead_code)]
pub const PLAYOUT_SAMPLES: usize = (SAMPLE_RATE_HZ as usize / 1000) * 10; // 480

/// Minimum jitter delay NetEq is allowed to converge to (ms).
const MIN_DELAY_MS: u32 = 40;
/// Maximum jitter delay NetEq is allowed to converge to (ms).
const MAX_DELAY_MS: u32 = 200;
/// Buffer cap in packets — generous, but bounded so a runaway sender can't
/// drive memory growth.
const MAX_PACKETS_IN_BUFFER: usize = 50;

/// Thin `AudioDecoder` over `opus::Decoder` so `neteq` can drive Opus payloads.
///
/// NetEq's `decode` trait is sample-based, not FEC-aware (no `fec` bool). The
/// stock `neteq` `NativeOpusDecoder` (gated behind the `native` feature, which
/// pulls in cpal/axum) calls `decode_float(..., false)` unconditionally; this
/// matches that behavior so we don't need the feature.
///
/// Receive-side FEC ("decode this frame's redundant copy on a known-lost prior
/// frame") would require either a trait change upstream or NetEq calling decode
/// twice with different `fec` values — out of scope for the initial 10-person
/// fix. Tracked as a follow-up alongside encoder-side `set_inband_fec`.
struct OpusFrameDecoder {
    inner: opus::Decoder,
    sample_rate: u32,
    channels: u8,
    /// Scratch buffer reused across calls to avoid per-frame allocation.
    scratch: Vec<f32>,
}

impl OpusFrameDecoder {
    fn new(sample_rate: u32, channels: u8) -> Result<Self, String> {
        let opus_channels = match channels {
            1 => opus::Channels::Mono,
            2 => opus::Channels::Stereo,
            n => return Err(format!("unsupported channel count: {n}")),
        };
        let inner = opus::Decoder::new(sample_rate, opus_channels)
            .map_err(|e| format!("opus decoder init: {e}"))?;
        // Largest plausible frame: 60 ms at 48 kHz stereo. We always use
        // 20 ms mono in practice; this is just a one-shot allocation cap.
        let cap = (sample_rate as usize / 1000) * 60 * channels as usize;
        Ok(Self {
            inner,
            sample_rate,
            channels,
            scratch: vec![0.0; cap],
        })
    }
}

impl AudioDecoder for OpusFrameDecoder {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> u8 {
        self.channels
    }

    fn decode(&mut self, encoded: &[u8]) -> Result<Vec<f32>, NetEqError> {
        match self.inner.decode_float(encoded, &mut self.scratch, false) {
            Ok(samples_per_channel) => {
                let total = samples_per_channel * self.channels as usize;
                Ok(self.scratch[..total].to_vec())
            }
            Err(e) => Err(NetEqError::DecoderError(format!("opus decode: {e}"))),
        }
    }
}

/// One peer's jitter buffer + decoder pair.
pub struct PeerJitterBuffer {
    neteq: NetEq,
    ssrc: u32,
}

impl PeerJitterBuffer {
    /// Construct a new jitter buffer for the peer identified by `peer_index`.
    ///
    /// `peer_index` is the relay-assigned 0..=254 stable index for the peer's
    /// lifetime in the room; we promote it to a 32-bit synthetic SSRC so NetEq
    /// has a stable stream identity for its delay-manager state.
    pub fn new(peer_index: u8) -> Result<Self, NetEqError> {
        let config = NetEqConfig {
            sample_rate: SAMPLE_RATE_HZ,
            channels: CHANNELS,
            max_packets_in_buffer: MAX_PACKETS_IN_BUFFER,
            min_delay_ms: MIN_DELAY_MS,
            max_delay_ms: MAX_DELAY_MS,
            ..Default::default()
        };
        let mut neteq = NetEq::new(config)?;
        let decoder =
            OpusFrameDecoder::new(SAMPLE_RATE_HZ, CHANNELS).map_err(NetEqError::DecoderError)?;
        neteq.register_decoder(OPUS_PAYLOAD_TYPE, Box::new(decoder));
        Ok(Self {
            neteq,
            ssrc: peer_index as u32,
        })
    }

    /// Insert a received Opus packet.
    ///
    /// `seq` and `ts_48k` come from the protocol v2 header (sender-authored,
    /// 16-bit wrapping sequence + 48 kHz RTP-style media time).
    pub fn insert_packet(
        &mut self,
        seq: u16,
        ts_48k: u32,
        opus_payload: &[u8],
    ) -> Result<(), NetEqError> {
        let header = RtpHeader::new(seq, ts_48k, self.ssrc, OPUS_PAYLOAD_TYPE, false);
        let packet = AudioPacket::new(
            header,
            opus_payload.to_vec(),
            SAMPLE_RATE_HZ,
            CHANNELS,
            FRAME_DURATION_MS,
        );
        self.neteq.insert_packet(packet)
    }

    /// Drain one 10 ms playout frame. Always succeeds: NetEq emits PLC /
    /// comfort-noise / silence rather than an error when there's nothing to
    /// play.
    ///
    /// Returns `(samples, vad_active)`. `samples.len() == PLAYOUT_SAMPLES`.
    pub fn get_audio(&mut self) -> Result<(Vec<f32>, bool), NetEqError> {
        let frame = self.neteq.get_audio()?;
        let vad = frame.vad_activity && !matches!(frame.speech_type, SpeechType::Expand);
        Ok((frame.samples, vad))
    }

    /// True when NetEq's buffer has no packets queued and the next
    /// `get_audio` call would emit expand/silence. Used by
    /// `huddle::playout` as part of the idle-peer gate — if a peer has
    /// gone quiet (`last_packet_at` is stale) *and* there's nothing
    /// buffered to drain, we stop appending frames to that peer's rodio
    /// Player. The check is conservative: a peer who just sent a burst
    /// then disconnected may have buffered audio that should still play
    /// out, so this method drives that decision.
    pub fn is_empty(&self) -> bool {
        self.neteq.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opus_silence_frame() -> Vec<u8> {
        // Encode 20 ms of silence at 48 kHz mono so the test doesn't rely on
        // synthetic Opus byte sequences.
        let mut encoder = opus::Encoder::new(
            SAMPLE_RATE_HZ,
            opus::Channels::Mono,
            opus::Application::Voip,
        )
        .expect("opus encoder");
        let pcm = vec![0.0_f32; FRAME_TIMESTAMP_DELTA as usize];
        let mut out = vec![0u8; 4000];
        let n = encoder
            .encode_float(&pcm, &mut out)
            .expect("opus encode silence");
        out.truncate(n);
        out
    }

    #[test]
    fn jitter_buffer_construction_succeeds_for_each_peer_index() {
        for idx in [0u8, 1, 7, 42, 254] {
            let jb = PeerJitterBuffer::new(idx).expect("construct jitter buffer");
            assert!(jb.is_empty(), "peer {idx} should start empty");
            // SSRC is the peer_index as u32 — verifies the stable-identity contract.
            assert_eq!(jb.ssrc, idx as u32);
        }
    }

    #[test]
    fn insert_packet_then_get_audio_returns_playout_frame() {
        let mut jb = PeerJitterBuffer::new(3).expect("jitter buffer");
        let payload = opus_silence_frame();

        // Insert a few sequential packets so NetEq has enough buffer to start
        // emitting Normal-decoded frames instead of pure Expand/silence.
        for i in 0..6u16 {
            let ts = (i as u32) * FRAME_TIMESTAMP_DELTA;
            jb.insert_packet(i, ts, &payload).expect("insert");
        }

        // Pull a playout frame — should be 10 ms = 480 mono samples.
        let (samples, _vad) = jb.get_audio().expect("get_audio");
        assert_eq!(
            samples.len(),
            PLAYOUT_SAMPLES,
            "NetEq must emit exactly one 10 ms frame per get_audio call",
        );
    }

    #[test]
    fn empty_buffer_get_audio_still_produces_a_frame() {
        // Even with no packets inserted, NetEq's contract is to always emit
        // 10 ms — as Expand/silence — so the playout clock keeps ticking.
        let mut jb = PeerJitterBuffer::new(0).expect("jitter buffer");
        let (samples, vad) = jb.get_audio().expect("get_audio");
        assert_eq!(samples.len(), PLAYOUT_SAMPLES);
        assert!(
            !vad,
            "empty buffer should not report voice activity (Expand frame)",
        );
    }
}
