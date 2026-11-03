//! Stateful 48 kHz mono Opus sender shared by human and agent publishers.

use crate::wire::{audio_level_dbov, FrameHeader, FLAG_DTX, V2_HEADER_LEN};

/// Samples in one 20 ms, 48 kHz mono frame.
pub const FRAME_SAMPLES: usize = 960;

/// Opus encoder and v2 sequence/media clock for one connection.
pub struct AudioEncoder {
    opus: opus::Encoder,
    sequence: u16,
    timestamp: u32,
    output: Vec<u8>,
}

impl AudioEncoder {
    /// Create the standard 32 kbit/s VoIP encoder with discontinuous transmission.
    pub fn new() -> Result<Self, String> {
        let mut opus = opus::Encoder::new(48_000, opus::Channels::Mono, opus::Application::Voip)
            .map_err(|e| format!("opus encoder: {e}"))?;
        opus.set_bitrate(opus::Bitrate::Bits(32_000))
            .map_err(|e| format!("opus bitrate: {e}"))?;
        opus.set_dtx(true).map_err(|e| format!("opus dtx: {e}"))?;
        Ok(Self {
            opus,
            sequence: 0,
            timestamp: 0,
            output: vec![0; 4_000],
        })
    }

    /// Encode up to 20 ms of finite mono PCM; pad the final short frame with silence.
    ///
    /// Returns the eight-byte v2 header followed by Opus. Empty/oversized or
    /// non-finite input is rejected without advancing the media clock.
    pub fn encode(&mut self, samples: &[f32]) -> Result<Vec<u8>, String> {
        if samples.is_empty()
            || samples.len() > FRAME_SAMPLES
            || samples.iter().any(|s| !s.is_finite())
        {
            return Err("Invalid Huddle audio frame".into());
        }
        let mut padded = [0.0; FRAME_SAMPLES];
        padded[..samples.len()].copy_from_slice(samples);
        let n = self
            .opus
            .encode_float(&padded, &mut self.output)
            .map_err(|e| format!("opus encode: {e}"))?;
        if n == 0 {
            return Err("Opus produced an empty frame".into());
        }
        let header = FrameHeader {
            seq: self.sequence,
            ts_48k: self.timestamp,
            level_dbov: audio_level_dbov(samples),
            flags: if n <= 2 { FLAG_DTX } else { 0 },
        };
        let mut frame = Vec::with_capacity(V2_HEADER_LEN + n);
        frame.extend_from_slice(&header.encode());
        frame.extend_from_slice(&self.output[..n]);
        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(FRAME_SAMPLES as u32);
        Ok(frame)
    }
}
