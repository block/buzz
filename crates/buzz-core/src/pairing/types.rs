//! NIP-AB pairing message types.
//!
//! All message types are serialized as JSON with a `"type"` discriminant field
//! (kebab-case). These are the plaintext payloads that get NIP-44 encrypted
//! before being placed in a [`crate::kind::KIND_PAIRING`] event.

use serde::{Deserialize, Serialize};

fn default_version() -> u32 {
    1
}

/// The set of messages exchanged during a NIP-AB device-pairing session.
///
/// Serialized with `"type"` as the tag field (kebab-case). Example:
/// ```json
/// {"type":"offer","session_id":"a1b2c3..."}
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PairingMessage {
    /// Target → Source. Announces the session and proves possession of the QR secret.
    Offer {
        /// Hex-encoded 32-byte session ID derived via HKDF from the session secret.
        session_id: String,
        /// Protocol version. Always `1` for this implementation.
        ///
        /// Defaults to `1` when absent (backward compat with pre-versioned implementations).
        #[serde(default = "default_version")]
        version: u32,
    },

    /// Either party → other. Confirms the Short Authentication String matches.
    SasConfirm {
        /// Hex-encoded 32-byte transcript hash, binding all session parameters.
        transcript_hash: String,
    },

    /// Initiator → Responder (or vice-versa). Delivers the actual secret payload.
    Payload {
        /// Discriminates the payload format so the receiver knows how to handle it.
        payload_type: PayloadType,
        /// The payload content (format depends on `payload_type`).
        payload: String,
    },

    /// Sent by either party to signal successful session completion.
    Complete {
        /// `true` if the session completed successfully, `false` on partial failure.
        success: bool,
    },

    /// Sent by either party to abort the session early.
    Abort {
        /// Machine-readable reason for the abort.
        reason: AbortReason,
    },
}

/// Discriminates the content of a [`PairingMessage::Payload`] message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadType {
    /// Raw `nsec` bech32 secret key.
    Nsec,
    /// NIP-46 bunker connection string.
    Bunker,
    /// NIP-46 `nostrconnect://` URI.
    Connect,
    /// Application-defined payload; interpretation is out-of-band.
    Custom,
}

/// Machine-readable reason a pairing session was aborted.
///
/// The spec allows implementations to define additional reason strings.
/// Unknown reasons are deserialized as [`Unknown`](AbortReason::Unknown)
/// and SHOULD be treated as `protocol_error` per NIP-AB §Abort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbortReason {
    /// The Short Authentication Strings shown to both users did not match.
    SasMismatch,
    /// The user explicitly denied the pairing request.
    UserDenied,
    /// The session exceeded its time limit without completing.
    Timeout,
    /// An unexpected or malformed message was received.
    ProtocolError,
    /// An unrecognized abort reason from a future or extended implementation.
    /// Produced only by deserialization of unknown reason strings.
    /// Callers MUST NOT use this variant for outbound aborts — use a
    /// spec-defined reason instead. Treat as `ProtocolError` per NIP-AB §Abort.
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offer_round_trip() {
        let msg = PairingMessage::Offer {
            session_id: "deadbeef".repeat(8),
            version: 1,
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(
            json.contains(r#""type":"offer""#),
            "tag field present: {json}"
        );
        assert!(
            json.contains(r#""version":1"#),
            "version field present: {json}"
        );
        let back: PairingMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(msg, back);
    }

    #[test]
    fn offer_version_defaults_to_1_when_absent() {
        // Simulate a legacy offer message without the version field.
        let json = r#"{"type":"offer","session_id":"deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"}"#;
        let msg: PairingMessage = serde_json::from_str(json).expect("deserialize");
        assert_eq!(
            msg,
            PairingMessage::Offer {
                session_id: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
                    .to_string(),
                version: 1,
            }
        );
    }

    #[test]
    fn sas_confirm_round_trip() {
        let msg = PairingMessage::SasConfirm {
            transcript_hash: "cafebabe".repeat(8),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(
            json.contains(r#""type":"sas-confirm""#),
            "kebab-case tag: {json}"
        );
        let back: PairingMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(msg, back);
    }

    #[test]
    fn payload_round_trip() {
        let msg = PairingMessage::Payload {
            payload_type: PayloadType::Nsec,
            payload: "nsec1abc".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(r#""type":"payload""#));
        assert!(json.contains(r#""payload_type":"nsec""#));
        let back: PairingMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(msg, back);
    }

    #[test]
    fn abort_sas_mismatch_round_trip() {
        let msg = PairingMessage::Abort {
            reason: AbortReason::SasMismatch,
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(
            json.contains(r#""reason":"sas_mismatch""#),
            "snake_case: {json}"
        );
        let back: PairingMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(msg, back);
    }

    #[test]
    fn complete_round_trip() {
        for success in [true, false] {
            let msg = PairingMessage::Complete { success };
            let json = serde_json::to_string(&msg).expect("serialize");
            let back: PairingMessage = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(msg, back);
        }
    }

    #[test]
    fn all_abort_reasons_round_trip() {
        let reasons = [
            AbortReason::SasMismatch,
            AbortReason::UserDenied,
            AbortReason::Timeout,
            AbortReason::ProtocolError,
        ];
        for reason in reasons {
            let msg = PairingMessage::Abort { reason };
            let json = serde_json::to_string(&msg).expect("serialize");
            let back: PairingMessage = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(msg, back);
        }
    }

    #[test]
    fn unknown_abort_reason_deserializes_to_unknown() {
        // NIP-AB §Abort: "unknown reasons SHOULD be treated as protocol_error"
        let json = r#"{"type":"abort","reason":"solar_flare"}"#;
        let msg: PairingMessage = serde_json::from_str(json).expect("deserialize");
        assert_eq!(
            msg,
            PairingMessage::Abort {
                reason: AbortReason::Unknown
            }
        );
    }

    #[test]
    fn unknown_abort_reason_is_not_protocol_error_variant() {
        // Unknown is a distinct variant — callers should never construct it
        // for outbound use, but if they do it serializes distinctly from
        // ProtocolError so we can catch the mistake.
        assert_ne!(AbortReason::Unknown, AbortReason::ProtocolError);
    }

    #[test]
    fn all_payload_types_round_trip() {
        let types = [
            PayloadType::Nsec,
            PayloadType::Bunker,
            PayloadType::Connect,
            PayloadType::Custom,
        ];
        for payload_type in types {
            let msg = PairingMessage::Payload {
                payload_type,
                payload: "data".to_string(),
            };
            let json = serde_json::to_string(&msg).expect("serialize");
            let back: PairingMessage = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(msg, back);
        }
    }
}
