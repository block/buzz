//! Huddle audio wire protocol — versioning + frame header.
//!
//! ## v1 (legacy)
//!
//! Client → relay: `<opus_bytes>`
//! Relay   → client: `<peer_index: u8><opus_bytes>`
//!
//! No per-frame metadata; receiver synthesizes sequence/timestamp on arrival.
//! Kept for backward compatibility — relay still admits v1 clients into
//! v1-pinned rooms — but new clients always speak v2.
//!
//! ## v2 (this commit)
//!
//! Client → relay: `<header: [u8; 8]><opus_bytes>`
//! Relay   → client: `<peer_index: u8><header: [u8; 8]><opus_bytes>`
//!
//! Header layout (8 bytes, network byte order, big-endian):
//!
//! ```text
//!  byte 0..=1 : seq         u16  wrapping sequence number, +1 per packet
//!  byte 2..=5 : ts_48k      u32  sender RTP-style 48 kHz media time,
//!                                +960 per 20 ms encoded frame
//!  byte 6     : level_dbov  i8   audio level in dBov, range [-127, 0]
//!  byte 7     : flags       u8   bit 0 = DTX/comfort frame; other bits
//!                                MUST be ignored on decode
//! ```
//!
//! Notes for reviewers (Max's spec):
//! * Header is fixed-size. No extension mechanism in v2 — future fields go
//!   into a v3 with a new pinned version.
//! * `level_dbov` is client-authored telemetry. The relay parses it for
//!   logging/active-speaker hints, clamps invalid values into range, and
//!   **never** uses it for trust decisions (admission, moderation, etc.).
//! * Negotiation lives in the WS auth message (`protocol_version: 2`), not
//!   in any bit of `flags`. Mixed-version rooms are rejected at the relay
//!   with `upgrade_required`.

/// Wire protocol version this client speaks. Bumped only when the frame
/// layout itself changes; the relay tracks pinned per-room.
pub const PROTOCOL_VERSION: u8 = 2;

/// Length of the v2 per-frame header in bytes.
pub const V2_HEADER_LEN: usize = 8;

/// Flag bit indicating a DTX / comfort-noise frame. Receivers MAY skip
/// arrival accounting for these; encoders MUST set it for any Opus packet
/// they tag as DTX. Unset on normal speech frames.
pub const FLAG_DTX: u8 = 0x01;

/// Reserved bit-mask. Any bits outside this set MUST be ignored by the
/// receiver — they're available for the next minor protocol bump within v2.
#[allow(dead_code)] // Public constant for diagnostics / future flag additions.
pub const RESERVED_FLAG_MASK: u8 = !FLAG_DTX;

/// Parsed view of one v2 header. Cheap (Copy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Sender-authored sequence number; wraps every 2^16 frames (~21.8 min
    /// at 50 Hz).
    pub seq: u16,
    /// Sender-authored 48 kHz RTP-style media timestamp.
    pub ts_48k: u32,
    /// Audio level in dBov. Always parsed into the canonical `-127..=0`
    /// range — out-of-range inputs are clamped to `-127` (silence).
    pub level_dbov: i8,
    /// Raw flags byte. Use [`Self::is_dtx`] / explicit masks; bits outside
    /// `FLAG_DTX | reserved` are not inspected.
    pub flags: u8,
}

impl FrameHeader {
    /// Encode `self` into 8 bytes, network byte order. Suitable for direct
    /// concatenation onto an Opus payload.
    pub fn encode(self) -> [u8; V2_HEADER_LEN] {
        let mut out = [0u8; V2_HEADER_LEN];
        out[0..2].copy_from_slice(&self.seq.to_be_bytes());
        out[2..6].copy_from_slice(&self.ts_48k.to_be_bytes());
        out[6] = self.level_dbov as u8;
        out[7] = self.flags;
        out
    }

    /// Parse a v2 header from the leading 8 bytes of `bytes`. Returns
    /// `(header, remainder)` so callers can pass the remainder straight to
    /// the Opus decoder.
    ///
    /// `level_dbov` is clamped into `-127..=0`; values outside that range
    /// are coerced to `-127`. Per Max's spec, malformed VU metadata must
    /// *not* cause the audio frame to be dropped — only the metric is
    /// suppressed.
    pub fn parse(bytes: &[u8]) -> Option<(Self, &[u8])> {
        if bytes.len() < V2_HEADER_LEN {
            return None;
        }
        let seq = u16::from_be_bytes([bytes[0], bytes[1]]);
        let ts_48k = u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
        let raw_level = bytes[6] as i8;
        let level_dbov = if (-127..=0).contains(&raw_level) {
            raw_level
        } else {
            // Out-of-range telemetry is coerced to "silent floor"; the
            // audio packet is still delivered. This matches Max's "don't
            // drop audio on bad VU" guidance.
            -127
        };
        let flags = bytes[7];
        Some((
            Self {
                seq,
                ts_48k,
                level_dbov,
                flags,
            },
            &bytes[V2_HEADER_LEN..],
        ))
    }

    /// True if [`FLAG_DTX`] is set.
    #[allow(dead_code)]
    pub fn is_dtx(&self) -> bool {
        (self.flags & FLAG_DTX) != 0
    }
}

/// Compute a dBov audio level for a normalized f32 PCM frame.
///
/// "dBov" is RMS expressed in dB relative to full scale (where full scale =
/// peak amplitude 1.0), clamped into the v2 header's canonical `-127..=0`
/// range. A silent frame returns `-127`; full-scale ±1.0 returns `0`.
///
/// This is cheap (one pass over the samples, one log) and runs once per
/// 20 ms frame, so it's nowhere near the audio fast-path budget.
pub fn audio_level_dbov(samples: &[f32]) -> i8 {
    if samples.is_empty() {
        return -127;
    }
    let mean_square: f64 = samples
        .iter()
        .map(|&s| {
            let v = s as f64;
            v * v
        })
        .sum::<f64>()
        / samples.len() as f64;
    if mean_square <= 0.0 {
        return -127;
    }
    let rms = mean_square.sqrt();
    let db = 20.0 * rms.log10();
    // Clamp into the canonical range. -127 = silence floor, 0 = full scale.
    if !db.is_finite() || db <= -127.0 {
        -127
    } else if db >= 0.0 {
        0
    } else {
        db.round() as i8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_fields() {
        let h = FrameHeader {
            seq: 0xABCD,
            ts_48k: 0x12_34_56_78,
            level_dbov: -40,
            flags: FLAG_DTX,
        };
        let bytes = h.encode();
        let (parsed, tail) = FrameHeader::parse(&bytes).expect("parse");
        assert_eq!(parsed, h);
        assert!(tail.is_empty());
        assert!(parsed.is_dtx());
    }

    /// Header is fixed 8 bytes; anything shorter is rejected.
    #[test]
    fn parse_rejects_short_input() {
        for len in 0..V2_HEADER_LEN {
            let buf = vec![0u8; len];
            assert!(
                FrameHeader::parse(&buf).is_none(),
                "{len} bytes must be rejected",
            );
        }
    }

    /// Trailing bytes pass through as the remainder, which is what the
    /// receive path hands to the Opus decoder.
    #[test]
    fn parse_returns_payload_remainder() {
        let mut buf = FrameHeader {
            seq: 1,
            ts_48k: 960,
            level_dbov: -20,
            flags: 0,
        }
        .encode()
        .to_vec();
        buf.extend_from_slice(b"opus-bytes");
        let (_h, tail) = FrameHeader::parse(&buf).expect("parse");
        assert_eq!(tail, b"opus-bytes");
    }

    /// Bytes in big-endian network order, matching Max's spec. This pins
    /// the byte layout against accidental endianness changes.
    #[test]
    fn encoded_byte_order_is_network_byte_order() {
        let h = FrameHeader {
            seq: 0x0102,
            ts_48k: 0x0304_0506,
            level_dbov: -1,
            flags: 0,
        };
        let bytes = h.encode();
        assert_eq!(bytes[0..2], [0x01, 0x02]);
        assert_eq!(bytes[2..6], [0x03, 0x04, 0x05, 0x06]);
        assert_eq!(bytes[6], 0xFF); // -1 as i8 -> 0xFF
        assert_eq!(bytes[7], 0x00);
    }

    /// Per Max: malformed `level_dbov` must not drop the frame; only the
    /// metric is suppressed. We coerce to -127 (silence floor) and keep
    /// the payload available.
    #[test]
    fn out_of_range_level_dbov_is_clamped_not_rejected() {
        // i8(127) is +127 which is outside [-127, 0].
        let mut bytes = FrameHeader {
            seq: 7,
            ts_48k: 7 * 960,
            level_dbov: 0,
            flags: 0,
        }
        .encode();
        bytes[6] = 0x7F; // raw +127 — out of range
        let (parsed, _) = FrameHeader::parse(&bytes).expect("parse must succeed");
        assert_eq!(parsed.level_dbov, -127, "clamped to silence floor");
        assert_eq!(parsed.seq, 7, "other fields preserved");
    }

    #[test]
    fn audio_level_dbov_clamps_silence_to_minus_127() {
        assert_eq!(audio_level_dbov(&[]), -127);
        assert_eq!(audio_level_dbov(&[0.0_f32; 960]), -127);
    }

    #[test]
    fn audio_level_dbov_full_scale_returns_zero() {
        let frame: Vec<f32> = (0..960).map(|_| 1.0_f32).collect();
        assert_eq!(audio_level_dbov(&frame), 0);
    }

    #[test]
    fn audio_level_dbov_is_in_canonical_range_for_realistic_speech() {
        // A 1 kHz sine at amplitude 0.3 — typical normalized speech peak.
        // RMS ≈ 0.3 / sqrt(2) ≈ 0.212 → ≈ -13 dBov.
        let frame: Vec<f32> = (0..960)
            .map(|i| {
                let t = i as f32 / 48_000.0;
                0.3 * (2.0 * std::f32::consts::PI * 1_000.0 * t).sin()
            })
            .collect();
        let db = audio_level_dbov(&frame);
        assert!(
            (-20..=-8).contains(&db),
            "expected -20..=-8 dBov for 0.3-amp sine, got {db}",
        );
    }

    /// Reserved flag bits MUST be ignored on parse (no error, just preserved
    /// for forward-compat). The contract is that *receivers* don't inspect
    /// them; the parse path keeps them so introspection still works.
    #[test]
    fn reserved_flag_bits_are_preserved_for_inspection() {
        let mut bytes = FrameHeader {
            seq: 0,
            ts_48k: 0,
            level_dbov: 0,
            flags: 0,
        }
        .encode();
        bytes[7] = 0b1010_1010; // FLAG_DTX bit clear, reserved bits set
        let (parsed, _) = FrameHeader::parse(&bytes).expect("parse");
        assert_eq!(parsed.flags, 0b1010_1010);
        assert!(!parsed.is_dtx());
        // Caller can still check which reserved bits were set if needed.
        assert_eq!(parsed.flags & RESERVED_FLAG_MASK, 0b1010_1010);
    }
}
