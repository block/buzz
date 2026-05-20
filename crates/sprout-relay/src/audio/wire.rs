//! Huddle audio wire protocol — relay-side parse helpers.
//!
//! Mirrors the desktop client's `huddle::wire` for the half of the protocol
//! the relay cares about: validating that v2 frames carry an 8-byte header,
//! parsing it into structured metrics, and clamping
//! caller-authored telemetry to safe ranges. The relay never *generates*
//! v2 frames (clients author them) and never *re-encodes* them (broadcast
//! is opaque byte forwarding) — so only the parse half of the protocol
//! lives here, not the encode half.
//!
//! # Threat-model invariant
//!
//! `level_dbov` is client-authored telemetry. Anything we surface from it
//! (logs, active-speaker UI hints, dominant-talker decisions) must treat
//! it as untrusted. This module's [`FrameHeader::parse`] clamps out-of-range
//! values into the canonical `-127..=0` range but never drops the audio
//! frame on bad VU data — bad metadata must not cause audible loss.
//! Trust decisions (admission, moderation, kicks) MUST NOT consume
//! `level_dbov`.

/// Length of the v2 per-frame header in bytes. The wire layout is, in
/// network byte order:
///
/// ```text
///  byte 0..=1 : seq         u16
///  byte 2..=5 : ts_48k      u32
///  byte 6     : level_dbov  i8   range [-127, 0]
///  byte 7     : flags       u8   bit 0 = DTX; other bits reserved
/// ```
pub const V2_HEADER_LEN: usize = 8;

/// `flags & FLAG_DTX` indicates a DTX/comfort-noise frame.
pub const FLAG_DTX: u8 = 0x01;

/// Parsed v2 header view. Cheap (Copy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Sender-authored sequence number; wraps every 2^16 frames.
    pub seq: u16,
    /// Sender-authored 48 kHz RTP-style media timestamp.
    pub ts_48k: u32,
    /// Audio level in dBov. Always parsed into the canonical `-127..=0`
    /// range — out-of-range inputs are clamped to `-127` (silence floor).
    /// Untrusted; for diagnostics only.
    pub level_dbov: i8,
    /// Raw flags byte. Use bit masks; reserved bits MAY be set.
    pub flags: u8,
}

impl FrameHeader {
    /// True if `FLAG_DTX` is set.
    pub fn is_dtx(&self) -> bool {
        (self.flags & FLAG_DTX) != 0
    }

    /// Parse a v2 header from the leading 8 bytes of `bytes`. Returns the
    /// header and the remaining payload slice (the opaque Opus body the
    /// relay forwards). On `None`, the caller should treat the frame as
    /// malformed and drop it.
    ///
    /// `level_dbov` is clamped into `-127..=0`; out-of-range inputs become
    /// `-127`. The audio frame is never rejected for bad telemetry — only
    /// the metric is suppressed.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canonical layout: BE u16 seq, BE u32 ts_48k, i8 level, u8 flags.
    /// This test pins the byte order so an accidental endianness flip on
    /// either side of the protocol is caught immediately.
    #[test]
    fn parse_reads_network_byte_order() {
        let bytes = [
            0x01, 0x02, // seq = 0x0102
            0x03, 0x04, 0x05, 0x06, // ts_48k = 0x0304_0506
            0xFF, // level_dbov = -1
            0x01, // flags = FLAG_DTX
            0xAA, 0xBB, 0xCC, // opaque payload — what the relay forwards
        ];
        let (h, payload) = FrameHeader::parse(&bytes).expect("parse");
        assert_eq!(h.seq, 0x0102);
        assert_eq!(h.ts_48k, 0x0304_0506);
        assert_eq!(h.level_dbov, -1);
        assert!(h.is_dtx());
        assert_eq!(payload, &[0xAA, 0xBB, 0xCC]);
    }

    /// Anything shorter than the fixed 8-byte header is malformed.
    #[test]
    fn parse_rejects_short_input() {
        for len in 0..V2_HEADER_LEN {
            let buf = vec![0u8; len];
            assert!(
                FrameHeader::parse(&buf).is_none(),
                "{len}-byte input must fail"
            );
        }
    }

    /// Out-of-range `level_dbov` must not drop the frame. The metric is
    /// suppressed (clamped to -127, the silence floor) but the audio
    /// payload is preserved for forwarding. This pins the "bad VU
    /// metadata is not audible loss" invariant relay-side as well as
    /// desktop-side.
    #[test]
    fn parse_clamps_out_of_range_level_keeps_frame() {
        // i8 = +127 → outside [-127, 0]
        let bytes = [
            0x00, 0x07, // seq = 7
            0x00, 0x00, 0x03, 0xC0, // ts_48k = 960
            0x7F, // level_dbov raw = +127 (invalid)
            0x00, // flags
            b'o', b'p', b'u', b's',
        ];
        let (h, payload) = FrameHeader::parse(&bytes).expect("parse must succeed");
        assert_eq!(h.level_dbov, -127, "invalid level clamps to silence floor");
        assert_eq!(h.seq, 7, "valid fields preserved alongside clamp");
        assert_eq!(
            payload, b"opus",
            "audio payload still available for forwarding"
        );
    }

    /// Reserved flag bits are passed through untouched. Receivers (the
    /// other half of the protocol) are responsible for ignoring them;
    /// the relay's only job is to forward the bytes faithfully.
    #[test]
    fn parse_preserves_reserved_flag_bits() {
        let bytes = [
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0b1010_1010, // FLAG_DTX clear, reserved bits set
        ];
        let (h, _) = FrameHeader::parse(&bytes).expect("parse");
        assert_eq!(h.flags, 0b1010_1010);
        assert!(!h.is_dtx());
    }
}
