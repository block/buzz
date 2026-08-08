//! Wire types for the narrow ACP delivery broker.
//!
//! The broker is intentionally transport-only: the CLI still resolves mentions
//! and threads, builds tags, and signs events. The harness only performs the
//! relay HTTP operations that a sandboxed CLI cannot perform itself.

use std::collections::BTreeSet;

use nostr::{Event, PublicKey};
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
/// Optional exact mention set for new stream messages and forum posts/comments.
///
/// Replies may omit `p` tags only after their signed parent is verified in the
/// same channel. If they carry any, those tags must still match the configured
/// set. Edits and diff payloads are outside this policy.
pub const TOP_LEVEL_MENTION_PUBKEYS_ENV: &str = "BUZZ_OUTBOUND_TOP_LEVEL_MENTION_PUBKEYS";
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

/// Return whether an outbound message kind is subject to mention routing.
///
/// Edits and diff payloads share the delivery broker but do not create a new
/// conversational post and their builders do not accept mention tags.
pub fn is_mention_policy_message_kind(kind: u16) -> bool {
    matches!(
        u32::from(kind),
        crate::kind::KIND_STREAM_MESSAGE
            | crate::kind::KIND_STREAM_MESSAGE_V2
            | crate::kind::KIND_FORUM_POST
            | crate::kind::KIND_FORUM_COMMENT
    )
}

/// Relay context that must be verified before an unmentioned reply can pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionPolicyReplyContext {
    /// Signed event ID carried by the child's NIP-10 reply marker.
    pub parent_event_id: String,
    /// Canonical UUID from the child's `h` channel tag.
    pub channel_id: String,
}

/// Result of evaluating a signed event against the configured mention policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MentionPolicyDecision {
    /// The signed event already carries the exact configured mention set.
    Allow,
    /// The event has no mentions and requires signed same-channel parent proof.
    VerifyReply(MentionPolicyReplyContext),
}

/// Evaluate the exact `p`-tag set on an outbound conversational message.
///
/// `configured_pubkeys` is a comma-separated list of hex pubkeys or npubs.
/// Top-level events, and replies that carry `p` tags, must contain exactly the
/// configured mention set. An unmentioned reply returns a verification request;
/// callers must fetch and validate its signed same-channel parent before send.
pub fn evaluate_top_level_mention_policy(
    event: &Event,
    configured_pubkeys: &str,
) -> Result<MentionPolicyDecision, String> {
    let required = parse_mention_policy_pubkeys(configured_pubkeys)?;
    let mut actual = BTreeSet::new();
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) != Some("p") {
            continue;
        }
        let value = parts
            .get(1)
            .ok_or_else(|| "outbound message contains a malformed p tag".to_string())?;
        let pubkey = PublicKey::parse(value)
            .map_err(|_| format!("outbound message contains an invalid p-tag pubkey: {value}"))?;
        if !actual.insert(pubkey.to_hex()) {
            return Err(format!(
                "outbound message contains a duplicate p-tag pubkey: {}",
                pubkey.to_hex()
            ));
        }
    }

    if actual == required {
        return Ok(MentionPolicyDecision::Allow);
    }

    if !actual.is_empty() {
        return Err(mention_policy_mismatch(&required, &actual));
    }

    let mut reply_parent = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) != Some("e")
            || parts.get(3).map(String::as_str) != Some("reply")
        {
            continue;
        }
        if reply_parent.is_some() {
            return Err("outbound reply contains multiple NIP-10 reply markers".into());
        }
        let value = parts
            .get(1)
            .ok_or_else(|| "outbound reply contains a malformed reply marker".to_string())?;
        let event_id = nostr::EventId::parse(value)
            .map_err(|_| format!("outbound reply contains an invalid parent event id: {value}"))?;
        reply_parent = Some(event_id.to_hex());
    }

    let Some(parent_event_id) = reply_parent else {
        return Err(mention_policy_mismatch(&required, &actual));
    };
    let channel_id = single_channel_id(event)?;
    Ok(MentionPolicyDecision::VerifyReply(
        MentionPolicyReplyContext {
            parent_event_id,
            channel_id,
        },
    ))
}

fn mention_policy_mismatch(required: &BTreeSet<String>, actual: &BTreeSet<String>) -> String {
    format!(
        "outbound message blocked: mention pubkeys must exactly match [{}], but the signed event contains [{}]; retry with the configured mention identity and no other mentions, or use --reply-to without an explicit mention for a genuine reply",
        required.iter().cloned().collect::<Vec<_>>().join(","),
        actual.iter().cloned().collect::<Vec<_>>().join(",")
    )
}

fn single_channel_id(event: &Event) -> Result<String, String> {
    let mut channel_id = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) != Some("h") {
            continue;
        }
        if channel_id.is_some() {
            return Err("outbound message contains multiple channel tags".into());
        }
        let value = parts
            .get(1)
            .ok_or_else(|| "outbound message contains a malformed channel tag".to_string())?;
        let parsed = Uuid::parse_str(value)
            .map_err(|_| format!("outbound message contains an invalid channel id: {value}"))?;
        channel_id = Some(parsed.to_string());
    }
    channel_id.ok_or_else(|| "outbound reply is missing its channel tag".into())
}

/// Verify the signed parent required by an unmentioned reply decision.
pub fn validate_mention_policy_reply_parent(
    parent: &Event,
    context: &MentionPolicyReplyContext,
) -> Result<(), String> {
    parent
        .verify()
        .map_err(|error| format!("reply parent signature verification failed: {error}"))?;
    if parent.id.to_hex() != context.parent_event_id {
        return Err("reply parent id does not match the signed reply marker".into());
    }
    if !is_brokered_message_kind(parent.kind.as_u16()) {
        return Err(format!(
            "reply parent kind {} is not a supported message kind",
            parent.kind.as_u16()
        ));
    }
    let parent_channel = single_channel_id(parent)?;
    if parent_channel != context.channel_id {
        return Err(format!(
            "reply parent belongs to channel {parent_channel}, not {}",
            context.channel_id
        ));
    }
    Ok(())
}

fn parse_mention_policy_pubkeys(configured_pubkeys: &str) -> Result<BTreeSet<String>, String> {
    if configured_pubkeys.trim().is_empty() {
        return Err("top-level mention policy is configured but empty".into());
    }

    let mut pubkeys = BTreeSet::new();
    for value in configured_pubkeys.split(',') {
        let value = value.trim();
        if value.is_empty() {
            return Err("top-level mention policy contains an empty pubkey".into());
        }
        let pubkey = PublicKey::parse(value)
            .map_err(|_| format!("top-level mention policy contains an invalid pubkey: {value}"))?;
        if !pubkeys.insert(pubkey.to_hex()) {
            return Err(format!(
                "top-level mention policy contains a duplicate pubkey: {}",
                pubkey.to_hex()
            ));
        }
    }
    Ok(pubkeys)
}

/// Validate a configured mention policy without evaluating an event.
pub fn validate_top_level_mention_policy_config(configured_pubkeys: &str) -> Result<(), String> {
    parse_mention_policy_pubkeys(configured_pubkeys).map(|_| ())
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
    use nostr::{EventBuilder, Keys, Kind, Tag};

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
    fn mention_policy_scope_excludes_edits_and_diffs() {
        for kind in [9, 40002, 45001, 45003] {
            assert!(is_mention_policy_message_kind(kind), "kind {kind}");
        }
        for kind in [40003, 40008] {
            assert!(!is_mention_policy_message_kind(kind), "kind {kind}");
        }
    }

    #[test]
    fn top_level_mention_policy_requires_the_exact_configured_set() {
        let signer = Keys::generate();
        let required = Keys::generate().public_key();
        let other = Keys::generate().public_key();
        let matching = EventBuilder::new(Kind::Custom(9), "matching")
            .tags([Tag::public_key(required)])
            .sign_with_keys(&signer)
            .expect("sign matching event");
        let missing = EventBuilder::new(Kind::Custom(9), "missing")
            .tags([])
            .sign_with_keys(&signer)
            .expect("sign missing event");
        let additional = EventBuilder::new(Kind::Custom(9), "additional")
            .tags([Tag::public_key(required), Tag::public_key(other)])
            .sign_with_keys(&signer)
            .expect("sign additional event");

        let configured = required.to_hex();
        assert_eq!(
            evaluate_top_level_mention_policy(&matching, &configured),
            Ok(MentionPolicyDecision::Allow)
        );
        assert!(evaluate_top_level_mention_policy(&missing, &configured).is_err());
        assert!(evaluate_top_level_mention_policy(&additional, &configured).is_err());
    }

    #[test]
    fn top_level_mention_policy_allows_unmentioned_replies_but_checks_reply_mentions() {
        let signer = Keys::generate();
        let required = Keys::generate().public_key();
        let other = Keys::generate().public_key();
        let channel_id = Uuid::new_v4().to_string();
        let other_channel_id = Uuid::new_v4().to_string();
        let parent = EventBuilder::new(Kind::Custom(9), "parent")
            .tags([Tag::parse(["h", &channel_id]).expect("parent channel")])
            .sign_with_keys(&signer)
            .expect("sign parent");
        let reply_tag =
            Tag::parse(["e", &parent.id.to_hex(), "", "reply"]).expect("build reply tag");
        let unmentioned_reply = EventBuilder::new(Kind::Custom(9), "reply")
            .tags([
                Tag::parse(["h", &channel_id]).expect("reply channel"),
                reply_tag.clone(),
            ])
            .sign_with_keys(&signer)
            .expect("sign reply");
        let matching_reply = EventBuilder::new(Kind::Custom(9), "matching reply")
            .tags([reply_tag.clone(), Tag::public_key(required)])
            .sign_with_keys(&signer)
            .expect("sign matching reply");
        let wrong_reply = EventBuilder::new(Kind::Custom(9), "wrong reply")
            .tags([reply_tag, Tag::public_key(other)])
            .sign_with_keys(&signer)
            .expect("sign wrong reply");
        let cross_channel_parent = EventBuilder::new(Kind::Custom(9), "cross-channel parent")
            .tags([Tag::parse(["h", &other_channel_id]).expect("other channel")])
            .sign_with_keys(&signer)
            .expect("sign cross-channel parent");
        let cross_channel_reply = EventBuilder::new(Kind::Custom(9), "cross-channel reply")
            .tags([
                Tag::parse(["h", &channel_id]).expect("reply channel"),
                Tag::parse(["e", &cross_channel_parent.id.to_hex(), "", "reply"])
                    .expect("cross-channel reply tag"),
            ])
            .sign_with_keys(&signer)
            .expect("sign cross-channel reply");

        let configured = required.to_hex();
        let MentionPolicyDecision::VerifyReply(reply_context) =
            evaluate_top_level_mention_policy(&unmentioned_reply, &configured)
                .expect("reply requires verification")
        else {
            panic!("unmentioned reply must require parent verification");
        };
        validate_mention_policy_reply_parent(&parent, &reply_context)
            .expect("signed same-channel parent");
        assert_eq!(
            evaluate_top_level_mention_policy(&matching_reply, &configured),
            Ok(MentionPolicyDecision::Allow)
        );
        assert!(evaluate_top_level_mention_policy(&wrong_reply, &configured).is_err());

        let MentionPolicyDecision::VerifyReply(cross_channel_context) =
            evaluate_top_level_mention_policy(&cross_channel_reply, &configured)
                .expect("reply shape")
        else {
            panic!("unmentioned reply must require parent verification");
        };
        assert!(validate_mention_policy_reply_parent(
            &cross_channel_parent,
            &cross_channel_context
        )
        .is_err());
    }

    #[test]
    fn top_level_mention_policy_rejects_invalid_configuration() {
        let event = EventBuilder::new(Kind::Custom(9), "message")
            .tags([])
            .sign_with_keys(&Keys::generate())
            .expect("sign event");

        assert!(evaluate_top_level_mention_policy(&event, "").is_err());
        assert!(evaluate_top_level_mention_policy(&event, "not-a-pubkey").is_err());
    }

    #[test]
    fn top_level_mention_policy_rejects_duplicate_config_and_event_tags() {
        let signer = Keys::generate();
        let required = Keys::generate().public_key();
        let duplicate = EventBuilder::new(Kind::Custom(9), "duplicate")
            .tags([Tag::public_key(required), Tag::public_key(required)])
            .sign_with_keys(&signer)
            .expect("sign duplicate event");
        let matching = EventBuilder::new(Kind::Custom(9), "matching")
            .tags([Tag::public_key(required)])
            .sign_with_keys(&signer)
            .expect("sign matching event");
        let configured = required.to_hex();

        assert!(evaluate_top_level_mention_policy(&duplicate, &configured).is_err());
        assert!(evaluate_top_level_mention_policy(
            &matching,
            &format!("{configured},{configured}")
        )
        .is_err());
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
