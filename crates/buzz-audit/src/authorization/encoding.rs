use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use super::{
    ActorReference, AuthorizationEventV1, AuthorizationEvidenceError, EventPayloadV1, StreamId,
};

/// Database-assigned acceptance and chain metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcceptedEventMetadata {
    stream_id: StreamId,
    stream_position: u64,
    previous_chain_digest: [u8; 32],
    accepted_at: DateTime<Utc>,
}

impl AcceptedEventMetadata {
    /// Construct metadata after the stream head is locked.
    pub fn new(
        stream_id: StreamId,
        stream_position: u64,
        previous_chain_digest: [u8; 32],
        accepted_at: DateTime<Utc>,
    ) -> Result<Self, AuthorizationEvidenceError> {
        if stream_position == 0 {
            return Err(AuthorizationEvidenceError::NilIdentifier);
        }
        Ok(Self {
            stream_id,
            stream_position,
            previous_chain_digest,
            accepted_at,
        })
    }

    /// Durable stream identity.
    pub const fn stream_id(self) -> StreamId {
        self.stream_id
    }

    /// Stream-local position.
    pub const fn stream_position(self) -> u64 {
        self.stream_position
    }

    /// Digest at the preceding stream position.
    pub const fn previous_chain_digest(self) -> [u8; 32] {
        self.previous_chain_digest
    }

    /// Database acceptance time.
    pub const fn accepted_at(self) -> DateTime<Utc> {
        self.accepted_at
    }
}

/// Canonical bytes and digests for one durably accepted event.
#[derive(Clone, PartialEq, Eq)]
pub struct CanonicalEvent {
    bytes: Vec<u8>,
    content_digest: [u8; 32],
    chain_digest: [u8; 32],
}

impl std::fmt::Debug for CanonicalEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanonicalEvent")
            .field("bytes", &"[redacted]")
            .field("content_digest", &"[redacted]")
            .field("chain_digest", &"[redacted]")
            .finish()
    }
}

impl CanonicalEvent {
    /// Digest only trusted semantic fields for idempotency before stream allocation.
    pub fn semantic_digest(event: &AuthorizationEventV1) -> [u8; 32] {
        Sha256::digest(encode_semantic(event)).into()
    }

    /// Encode V1 in fixed field order with explicit length and optional markers.
    pub fn encode(event: &AuthorizationEventV1, accepted: AcceptedEventMetadata) -> Self {
        let semantic = encode_semantic(event);
        let content_digest: [u8; 32] = Sha256::digest(&semantic).into();
        let mut bytes = Vec::with_capacity(semantic.len() + 128);
        push_bytes(&mut bytes, b"buzz-authorization-event-accepted-v1");
        push_bytes(&mut bytes, accepted.stream_id.as_uuid().as_bytes());
        push_u64(&mut bytes, accepted.stream_position);
        push_bytes(&mut bytes, &accepted.previous_chain_digest);
        push_time(&mut bytes, accepted.accepted_at);
        push_bytes(&mut bytes, &semantic);
        let chain_digest: [u8; 32] = Sha256::digest(&bytes).into();
        Self {
            bytes,
            content_digest,
            chain_digest,
        }
    }

    /// Canonical accepted-event bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Digest of trusted semantic fields, excluding database acceptance facts.
    pub const fn content_digest(&self) -> [u8; 32] {
        self.content_digest
    }

    /// Digest chaining semantic fields to exact stream acceptance metadata.
    pub const fn chain_digest(&self) -> [u8; 32] {
        self.chain_digest
    }

    /// Verify a stored accepted-event frame before it crosses the export boundary.
    ///
    /// This checks the immutable row's stream coordinates, semantic digest, and
    /// full chain digest without decoding or exposing any event field.
    pub fn verify_accepted_bytes(
        bytes: &[u8],
        stream_id: StreamId,
        stream_position: u64,
        expected_content_digest: [u8; 32],
        expected_chain_digest: [u8; 32],
    ) -> Result<(), AuthorizationEvidenceError> {
        if stream_position == 0 || <[u8; 32]>::from(Sha256::digest(bytes)) != expected_chain_digest
        {
            return Err(AuthorizationEvidenceError::InvalidDeliveryLease);
        }

        let mut cursor = 0_usize;
        let label = take_bytes(bytes, &mut cursor)?;
        let embedded_stream = take_bytes(bytes, &mut cursor)?;
        let embedded_position = take_u64(bytes, &mut cursor)?;
        let previous_digest = take_bytes(bytes, &mut cursor)?;
        take_exact(bytes, &mut cursor, 12)?;
        let semantic = take_bytes(bytes, &mut cursor)?;
        if cursor != bytes.len()
            || label != b"buzz-authorization-event-accepted-v1"
            || embedded_stream != stream_id.as_uuid().as_bytes()
            || embedded_position != stream_position
            || previous_digest.len() != 32
            || <[u8; 32]>::from(Sha256::digest(semantic)) != expected_content_digest
        {
            return Err(AuthorizationEvidenceError::InvalidDeliveryLease);
        }
        Ok(())
    }
}

fn take_u64(source: &[u8], cursor: &mut usize) -> Result<u64, AuthorizationEvidenceError> {
    let value: [u8; 8] = take_exact(source, cursor, 8)?
        .try_into()
        .map_err(|_| AuthorizationEvidenceError::InvalidDeliveryLease)?;
    Ok(u64::from_be_bytes(value))
}

fn take_bytes<'a>(
    source: &'a [u8],
    cursor: &mut usize,
) -> Result<&'a [u8], AuthorizationEvidenceError> {
    let length = take_u64(source, cursor)?;
    let length =
        usize::try_from(length).map_err(|_| AuthorizationEvidenceError::InvalidDeliveryLease)?;
    take_exact(source, cursor, length)
}

fn take_exact<'a>(
    source: &'a [u8],
    cursor: &mut usize,
    length: usize,
) -> Result<&'a [u8], AuthorizationEvidenceError> {
    let end = cursor
        .checked_add(length)
        .filter(|end| *end <= source.len())
        .ok_or(AuthorizationEvidenceError::InvalidDeliveryLease)?;
    let value = &source[*cursor..end];
    *cursor = end;
    Ok(value)
}

fn encode_semantic(event: &AuthorizationEventV1) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(512);
    push_bytes(&mut bytes, b"buzz-authorization-event-semantic-v1");
    push_u16(&mut bytes, 1);
    push_bytes(&mut bytes, event.event_id().as_uuid().as_bytes());
    push_bytes(&mut bytes, event.domain().as_uuid().as_bytes());
    push_time(&mut bytes, event.occurred_at());
    push_optional_uuid(
        &mut bytes,
        event.operation_id().map(|value| value.as_uuid()),
    );
    push_bytes(&mut bytes, event.correlation_id().as_uuid().as_bytes());
    push_bytes(&mut bytes, event.attempt_id().as_uuid().as_bytes());
    push_optional_uuid(
        &mut bytes,
        event.causal_parent().map(|value| value.as_uuid()),
    );
    encode_actor(&mut bytes, event.actor());
    push_optional_reference(&mut bytes, event.principal_reference());
    push_optional_reference(&mut bytes, event.key_reference());
    push_u16(&mut bytes, event.transport().discriminant());
    push_u16(&mut bytes, event.operation().discriminant());
    push_u16(&mut bytes, event.source().discriminant());
    push_u16(&mut bytes, event.kind().discriminant());
    push_u16(&mut bytes, event.result().discriminant());
    push_u16(&mut bytes, event.reason().discriminant());
    let versions = event.versions();
    push_optional_u64(&mut bytes, versions.binding);
    push_optional_u64(&mut bytes, versions.lease);
    push_optional_u64(&mut bytes, versions.lifecycle);
    push_optional_u64(&mut bytes, versions.invalidation);
    push_optional_digest(&mut bytes, versions.policy_digest);
    encode_payload(&mut bytes, event.payload());
    bytes
}

fn encode_actor(bytes: &mut Vec<u8>, actor: &ActorReference) {
    push_u16(bytes, actor.class().discriminant());
    match actor {
        ActorReference::NotApplicable
        | ActorReference::Unresolved
        | ActorReference::ControlPlane => {}
        ActorReference::Direct(actor) => push_reference(bytes, *actor),
        ActorReference::Delegated {
            actor,
            owner,
            relationship_revision,
        } => {
            push_reference(bytes, *actor);
            push_reference(bytes, *owner);
            push_u64(bytes, *relationship_revision);
        }
        ActorReference::Operator { actor, approvers } => {
            push_reference(bytes, *actor);
            push_u16(bytes, approvers.len() as u16);
            for approver in approvers {
                push_reference(bytes, *approver);
            }
        }
    }
}

fn encode_payload(bytes: &mut Vec<u8>, payload: &EventPayloadV1) {
    match payload {
        EventPayloadV1::None => push_u16(bytes, 0),
        EventPayloadV1::Lifecycle(value) => {
            push_u16(bytes, 1);
            push_reference(bytes, value.target());
            push_optional_u64(bytes, value.previous_version());
            push_optional_u64(bytes, value.current_version());
            push_optional_uuid(bytes, value.receipt_id().map(|id| id.as_uuid()));
            push_optional_uuid(bytes, value.effect_id().map(|id| id.as_uuid()));
            push_optional_u64(bytes, value.invalidation_generation());
            match value.lineage_binding() {
                Some(reference) => {
                    bytes.push(1);
                    push_reference(bytes, reference);
                }
                None => bytes.push(0),
            }
        }
        EventPayloadV1::BoundedSummary {
            count,
            snapshot_digest,
        } => {
            push_u16(bytes, 2);
            push_u32(bytes, *count);
            push_bytes(bytes, snapshot_digest);
        }
        EventPayloadV1::Delivery {
            original_event_id,
            delivery_attempt,
        } => {
            push_u16(bytes, 3);
            push_bytes(bytes, original_event_id.as_uuid().as_bytes());
            push_u32(bytes, *delivery_attempt);
        }
    }
}

fn push_reference(bytes: &mut Vec<u8>, reference: super::PseudonymousReference) {
    bytes.push(reference.kind() as u8);
    push_u32(bytes, reference.key_epoch());
    push_bytes(bytes, &reference.digest());
}

fn push_optional_reference(bytes: &mut Vec<u8>, reference: Option<super::PseudonymousReference>) {
    match reference {
        Some(reference) => {
            bytes.push(1);
            push_reference(bytes, reference);
        }
        None => bytes.push(0),
    }
}

fn push_optional_uuid(bytes: &mut Vec<u8>, value: Option<uuid::Uuid>) {
    match value {
        Some(value) => {
            bytes.push(1);
            push_bytes(bytes, value.as_bytes());
        }
        None => bytes.push(0),
    }
}

fn push_optional_u64(bytes: &mut Vec<u8>, value: Option<u64>) {
    match value {
        Some(value) => {
            bytes.push(1);
            push_u64(bytes, value);
        }
        None => bytes.push(0),
    }
}

fn push_optional_digest(bytes: &mut Vec<u8>, value: Option<[u8; 32]>) {
    match value {
        Some(value) => {
            bytes.push(1);
            push_bytes(bytes, &value);
        }
        None => bytes.push(0),
    }
}

fn push_time(bytes: &mut Vec<u8>, value: DateTime<Utc>) {
    bytes.extend_from_slice(&value.timestamp().to_be_bytes());
    bytes.extend_from_slice(&value.timestamp_subsec_nanos().to_be_bytes());
}

fn push_bytes(target: &mut Vec<u8>, value: &[u8]) {
    target.extend_from_slice(&(value.len() as u64).to_be_bytes());
    target.extend_from_slice(value);
}

fn push_u16(target: &mut Vec<u8>, value: u16) {
    target.extend_from_slice(&value.to_be_bytes());
}

fn push_u32(target: &mut Vec<u8>, value: u32) {
    target.extend_from_slice(&value.to_be_bytes());
}

fn push_u64(target: &mut Vec<u8>, value: u64) {
    target.extend_from_slice(&value.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::authorization::{
        ActorReference, AttemptId, CorrelationId, DecisionReason, EventId, EventKind, EventResult,
        OperationClass, SourceClass, TransportClass, VersionVectorV1,
    };
    use buzz_core::CommunityId;

    fn fixture_event() -> AuthorizationEventV1 {
        let domain = CommunityId::from_uuid(
            uuid::Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
        );
        AuthorizationEventV1::new(
            EventId::from_uuid(
                uuid::Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap(),
            )
            .unwrap(),
            domain,
            Utc.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap(),
            None,
            CorrelationId::from_uuid(
                uuid::Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap(),
            )
            .unwrap(),
            AttemptId::from_uuid(
                uuid::Uuid::parse_str("44444444-4444-4444-8444-444444444444").unwrap(),
            )
            .unwrap(),
            None,
            ActorReference::Unresolved,
            TransportClass::Internal,
            OperationClass::Read,
            SourceClass::Policy,
            EventKind::AdmissionDenied,
            EventResult::Denied,
            DecisionReason::PolicyDenied,
            VersionVectorV1::default(),
            EventPayloadV1::None,
        )
    }

    #[test]
    fn canonical_encoding_is_deterministic_and_chain_sensitive() {
        let event = fixture_event();
        let accepted = AcceptedEventMetadata::new(
            StreamId::from_uuid(
                uuid::Uuid::parse_str("55555555-5555-4555-8555-555555555555").unwrap(),
            )
            .unwrap(),
            7,
            [6; 32],
            Utc.with_ymd_and_hms(2026, 8, 1, 12, 0, 1).unwrap(),
        )
        .unwrap();
        let first = CanonicalEvent::encode(&event, accepted);
        let second = CanonicalEvent::encode(&event, accepted);
        assert_eq!(first, second);
        CanonicalEvent::verify_accepted_bytes(
            first.bytes(),
            accepted.stream_id(),
            accepted.stream_position(),
            first.content_digest(),
            first.chain_digest(),
        )
        .expect("exact canonical frame verifies");
        let mut tampered = first.bytes().to_vec();
        let last = tampered.last_mut().expect("canonical frame is nonempty");
        *last ^= 1;
        assert_eq!(
            CanonicalEvent::verify_accepted_bytes(
                &tampered,
                accepted.stream_id(),
                accepted.stream_position(),
                first.content_digest(),
                first.chain_digest(),
            ),
            Err(AuthorizationEvidenceError::InvalidDeliveryLease)
        );
        let next = AcceptedEventMetadata::new(
            accepted.stream_id(),
            8,
            first.chain_digest(),
            accepted.accepted_at(),
        )
        .unwrap();
        assert_ne!(
            first.chain_digest(),
            CanonicalEvent::encode(&event, next).chain_digest()
        );
        let debug = format!("{first:?}");
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains(&hex::encode(first.bytes())));
    }
}
