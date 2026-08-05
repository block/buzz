//! Wire types for the narrow ACP delivery broker.
//!
//! The broker is intentionally transport-only: the CLI still resolves mentions
//! and threads, builds tags, and signs events. The harness only performs the
//! relay HTTP operations that a sandboxed CLI cannot perform itself.

use nostr::Event;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Environment variable containing the broker's private request directory.
pub const BROKER_DIR_ENV: &str = "BUZZ_DELIVERY_BROKER_DIR";
/// Environment variable containing the per-harness broker capability.
pub const BROKER_CAPABILITY_ENV: &str = "BUZZ_DELIVERY_BROKER_CAPABILITY";
/// Environment variable containing the broker's ephemeral response-signing pubkey.
pub const BROKER_RESPONSE_PUBKEY_ENV: &str = "BUZZ_DELIVERY_BROKER_RESPONSE_PUBKEY";
/// Current on-disk protocol version.
pub const BROKER_PROTOCOL_VERSION: u8 = 1;
/// Local-only Nostr kind used to attest broker response bytes.
pub const BROKER_RESPONSE_ATTESTATION_KIND: u16 = 24_201;
/// Maximum serialized request size accepted by either endpoint.
pub const MAX_BROKER_REQUEST_BYTES: u64 = 512 * 1024;
/// Maximum serialized response size accepted by the CLI.
pub const MAX_BROKER_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
/// Maximum serialized relay result before response and attestation overhead.
pub const MAX_BROKER_RESULT_BYTES: u64 = 7 * 1024 * 1024;

/// Return whether a stored event is a user-visible message payload eligible
/// for exact-event broker delivery.
pub fn is_brokered_message_kind(kind: u16) -> bool {
    matches!(
        u32::from(kind),
        crate::kind::KIND_STREAM_MESSAGE
            | crate::kind::KIND_STREAM_MESSAGE_V2
            | crate::kind::KIND_STREAM_MESSAGE_EDIT
            | crate::kind::KIND_STREAM_MESSAGE_DIFF
            | crate::kind::KIND_FORUM_POST
            | crate::kind::KIND_FORUM_COMMENT
    )
}

/// Canonical digest signed by the broker's response attestation.
///
/// Signing a fixed-size digest avoids embedding the complete response twice in
/// the on-disk envelope while still binding every response byte.
pub fn broker_response_digest(response: &BrokerResponse) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(response)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

/// A single authenticated request from `buzz-cli` to `buzz-acp`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerRequest {
    /// Protocol version used to fail closed across incompatible releases.
    pub version: u8,
    /// Unique request identifier, also used as the response filename.
    pub request_id: Uuid,
    /// Per-harness bearer capability supplied out-of-band in the child environment.
    pub capability: String,
    /// Client wall-clock timestamp in Unix milliseconds.
    pub created_at_ms: u64,
    /// Narrow operation requested from the harness.
    pub operation: BrokerOperation,
}

/// Relay operations exposed by the delivery broker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum BrokerOperation {
    /// Execute a bounded Nostr filter query through `POST /query`.
    Query {
        /// One or more Nostr filter objects.
        filters: Vec<Value>,
    },
    /// Execute a bounded Nostr count through `POST /count`.
    Count {
        /// One or more Nostr filter objects.
        filters: Vec<Value>,
    },
    /// Submit an already-signed, stored message event through `POST /events`.
    SubmitStoredMessage {
        /// Exact event signed by the CLI. The broker never rebuilds or re-signs it.
        event: Box<Event>,
    },
}

/// A broker response written atomically for one request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerResponse {
    /// Protocol version used to fail closed across incompatible releases.
    pub version: u8,
    /// Request identifier copied from the authenticated request.
    pub request_id: Uuid,
    /// Successful relay response, present only when `error` is absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Structured failure, present only when `result` is absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BrokerError>,
}

/// Authenticated response envelope written by the harness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerResponseEnvelope {
    /// Structured response whose exact JSON encoding is signed.
    pub response: BrokerResponse,
    /// Ephemeral Nostr signature over the serialized response.
    pub attestation: Event,
}

impl BrokerResponse {
    /// Construct a successful response.
    pub fn success(request_id: Uuid, result: Value) -> Self {
        Self {
            version: BROKER_PROTOCOL_VERSION,
            request_id,
            result: Some(result),
            error: None,
        }
    }

    /// Construct a failed response.
    pub fn failure(request_id: Uuid, code: BrokerErrorCode, message: impl Into<String>) -> Self {
        Self {
            version: BROKER_PROTOCOL_VERSION,
            request_id,
            result: None,
            error: Some(BrokerError {
                code,
                message: message.into(),
            }),
        }
    }
}

/// Structured broker failure returned to the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerError {
    /// Stable machine-readable error category.
    pub code: BrokerErrorCode,
    /// Sanitized human-readable detail.
    pub message: String,
}

/// Stable delivery-broker error categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerErrorCode {
    /// The request failed schema, size, age, or filter validation.
    InvalidRequest,
    /// The bearer capability did not match this harness.
    Unauthorized,
    /// The operation or event kind is outside the broker allowlist.
    Unsupported,
    /// The broker is at its bounded concurrency limit and did not execute the request.
    Busy,
    /// The relay explicitly rejected the operation.
    RelayRejected,
    /// The event may have been accepted but could not be verified by exact readback.
    DeliveryUnknown,
    /// A local broker transport or serialization failure occurred.
    Internal,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};

    #[test]
    fn signed_event_round_trips_without_reconstruction() {
        let event = EventBuilder::new(Kind::Custom(9), "exact\ncontent")
            .tags([])
            .sign_with_keys(&Keys::generate())
            .expect("sign event");
        let request = BrokerRequest {
            version: BROKER_PROTOCOL_VERSION,
            request_id: Uuid::new_v4(),
            capability: "capability".into(),
            created_at_ms: 1,
            operation: BrokerOperation::SubmitStoredMessage {
                event: Box::new(event.clone()),
            },
        };

        let encoded = serde_json::to_vec(&request).expect("serialize request");
        let decoded: BrokerRequest = serde_json::from_slice(&encoded).expect("parse request");
        let BrokerOperation::SubmitStoredMessage { event: decoded } = decoded.operation else {
            panic!("wrong operation");
        };
        assert_eq!(*decoded, event);
    }

    #[test]
    fn brokered_message_kind_allowlist_covers_message_payloads_only() {
        for kind in [9, 40002, 40003, 40008, 45001, 45003] {
            assert!(is_brokered_message_kind(kind), "kind {kind}");
        }
        for kind in [5, 7, 40004, 40005, 40006, 40007, 45002] {
            assert!(!is_brokered_message_kind(kind), "kind {kind}");
        }
    }

    #[test]
    fn response_digest_changes_with_the_bound_payload() {
        let request_id = Uuid::new_v4();
        let first = BrokerResponse::success(request_id, serde_json::json!({"count": 1}));
        let second = BrokerResponse::success(request_id, serde_json::json!({"count": 2}));
        assert_ne!(
            broker_response_digest(&first).unwrap(),
            broker_response_digest(&second).unwrap()
        );
    }
}
