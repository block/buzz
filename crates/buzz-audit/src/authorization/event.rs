use std::fmt;

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};

use super::{
    ActorClass, AttemptId, AuthorizationEvidenceError, CorrelationId, DecisionReason, EffectId,
    EventId, EventKind, EventResult, OperationClass, OperationId, PseudonymousReference, ReceiptId,
    ReferenceKind, SourceClass, TransportClass,
};

/// Maximum independently verified approvals represented in one evidence event.
pub const MAX_EVENT_APPROVERS: usize = 4;

/// Redacted actor and approval facts for one event.
#[derive(Clone, PartialEq, Eq)]
pub enum ActorReference {
    /// No actor applies to this observation.
    NotApplicable,
    /// Input did not reach authenticated actor resolution.
    Unresolved,
    /// Authenticated direct actor.
    Direct(PseudonymousReference),
    /// Authenticated delegate and independently verified owner relationship.
    Delegated {
        /// Domain-scoped actor pseudonym.
        actor: PseudonymousReference,
        /// Domain-scoped owner pseudonym.
        owner: PseudonymousReference,
        /// Exact persisted relationship revision.
        relationship_revision: u64,
    },
    /// Authenticated operator and independently authenticated approvers.
    Operator {
        /// Domain-scoped operator pseudonym.
        actor: PseudonymousReference,
        /// Distinct domain-scoped approver pseudonyms.
        approvers: Box<[PseudonymousReference]>,
    },
    /// Non-human control-plane action.
    ControlPlane,
}

impl ActorReference {
    /// Construct a direct authenticated actor.
    pub fn direct(actor: PseudonymousReference) -> Result<Self, AuthorizationEvidenceError> {
        if actor.kind() != ReferenceKind::Actor {
            return Err(AuthorizationEvidenceError::InvalidPseudonymInput);
        }
        Ok(Self::Direct(actor))
    }

    /// Construct exact delegated actor evidence.
    pub fn delegated(
        actor: PseudonymousReference,
        owner: PseudonymousReference,
        relationship_revision: u64,
    ) -> Result<Self, AuthorizationEvidenceError> {
        if actor.kind() != ReferenceKind::Actor
            || owner.kind() != ReferenceKind::Actor
            || relationship_revision == 0
        {
            return Err(AuthorizationEvidenceError::InvalidPseudonymInput);
        }
        Ok(Self::Delegated {
            actor,
            owner,
            relationship_revision,
        })
    }

    /// Construct operator evidence with a bounded, distinct approval set.
    pub fn operator(
        actor: PseudonymousReference,
        mut approvers: Vec<PseudonymousReference>,
    ) -> Result<Self, AuthorizationEvidenceError> {
        if actor.kind() != ReferenceKind::Actor || approvers.len() > MAX_EVENT_APPROVERS {
            return Err(AuthorizationEvidenceError::InvalidPseudonymInput);
        }
        if approvers
            .iter()
            .any(|approver| approver.kind() != ReferenceKind::Approver)
        {
            return Err(AuthorizationEvidenceError::InvalidPseudonymInput);
        }
        approvers.sort();
        approvers.dedup();
        Ok(Self::Operator {
            actor,
            approvers: approvers.into_boxed_slice(),
        })
    }

    /// Closed actor class stored alongside the canonical payload.
    pub const fn class(&self) -> ActorClass {
        match self {
            Self::NotApplicable => ActorClass::NotApplicable,
            Self::Unresolved => ActorClass::Unresolved,
            Self::Direct(_) => ActorClass::Direct,
            Self::Delegated { .. } => ActorClass::Delegated,
            Self::Operator { .. } => ActorClass::Operator,
            Self::ControlPlane => ActorClass::ControlPlane,
        }
    }
}

impl fmt::Debug for ActorReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorReference")
            .field("class", &self.class())
            .field("value", &"[redacted]")
            .finish()
    }
}

/// Authorization-relevant versions available at a decision boundary.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct VersionVectorV1 {
    /// Binding version.
    pub binding: Option<u64>,
    /// Lease version.
    pub lease: Option<u64>,
    /// Domain lifecycle revision.
    pub lifecycle: Option<u64>,
    /// Durable invalidation generation.
    pub invalidation: Option<u64>,
    /// Opaque policy-revision digest.
    pub policy_digest: Option<[u8; 32]>,
}

impl fmt::Debug for VersionVectorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VersionVectorV1")
            .field("binding", &self.binding)
            .field("lease", &self.lease)
            .field("lifecycle", &self.lifecycle)
            .field("invalidation", &self.invalidation)
            .field("policy_digest", &"[redacted]")
            .finish()
    }
}

/// Closed lifecycle evidence payload.
#[derive(Clone, PartialEq, Eq)]
pub struct LifecycleEvidenceV1 {
    target: PseudonymousReference,
    previous_version: Option<u64>,
    current_version: Option<u64>,
    receipt_id: Option<ReceiptId>,
    effect_id: Option<EffectId>,
    invalidation_generation: Option<u64>,
    lineage_binding: Option<PseudonymousReference>,
}

impl LifecycleEvidenceV1 {
    /// Construct a bounded lifecycle payload.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        target: PseudonymousReference,
        previous_version: Option<u64>,
        current_version: Option<u64>,
        receipt_id: Option<ReceiptId>,
        effect_id: Option<EffectId>,
        invalidation_generation: Option<u64>,
        lineage_binding: Option<PseudonymousReference>,
    ) -> Result<Self, AuthorizationEvidenceError> {
        if target.kind() == ReferenceKind::Approver
            || lineage_binding.is_some_and(|value| value.kind() != ReferenceKind::Binding)
        {
            return Err(AuthorizationEvidenceError::InvalidPseudonymInput);
        }
        Ok(Self {
            target,
            previous_version,
            current_version,
            receipt_id,
            effect_id,
            invalidation_generation,
            lineage_binding,
        })
    }

    pub(crate) const fn target(&self) -> PseudonymousReference {
        self.target
    }

    pub(crate) const fn previous_version(&self) -> Option<u64> {
        self.previous_version
    }

    pub(crate) const fn current_version(&self) -> Option<u64> {
        self.current_version
    }

    pub(crate) const fn receipt_id(&self) -> Option<ReceiptId> {
        self.receipt_id
    }

    pub(crate) const fn effect_id(&self) -> Option<EffectId> {
        self.effect_id
    }

    pub(crate) const fn invalidation_generation(&self) -> Option<u64> {
        self.invalidation_generation
    }

    pub(crate) const fn lineage_binding(&self) -> Option<PseudonymousReference> {
        self.lineage_binding
    }
}

impl fmt::Debug for LifecycleEvidenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LifecycleEvidenceV1")
            .field("target", &"[redacted]")
            .field("previous_version", &self.previous_version)
            .field("current_version", &self.current_version)
            .field("receipt_id", &"[redacted]")
            .field("effect_id", &"[redacted]")
            .field("invalidation_generation", &self.invalidation_generation)
            .field("lineage_binding", &"[redacted]")
            .finish()
    }
}

/// Versioned closed event payload.
#[derive(Clone, PartialEq, Eq)]
pub enum EventPayloadV1 {
    /// Event needs no additional fields.
    None,
    /// Authorization-relevant lifecycle facts.
    Lifecycle(LifecycleEvidenceV1),
    /// Bounded listing or preview summary.
    BoundedSummary {
        /// Number of records represented, already capped by the caller.
        count: u32,
        /// Digest of the snapshot or preview input.
        snapshot_digest: [u8; 32],
    },
    /// Delivery or quarantine observation for an immutable event.
    Delivery {
        /// Original immutable event identity.
        original_event_id: EventId,
        /// Delivery attempt ordinal, distinct from request attempt identity.
        delivery_attempt: u32,
    },
}

impl fmt::Debug for EventPayloadV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("EventPayloadV1::None"),
            Self::Lifecycle(value) => formatter.debug_tuple("Lifecycle").field(value).finish(),
            Self::BoundedSummary { count, .. } => formatter
                .debug_struct("BoundedSummary")
                .field("count", count)
                .field("snapshot_digest", &"[redacted]")
                .finish(),
            Self::Delivery {
                delivery_attempt, ..
            } => formatter
                .debug_struct("Delivery")
                .field("original_event_id", &"[redacted]")
                .field("delivery_attempt", delivery_attempt)
                .finish(),
        }
    }
}

/// Closed provider-neutral authorization evidence before durable acceptance.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationEventV1 {
    event_id: EventId,
    domain: CommunityId,
    occurred_at: DateTime<Utc>,
    operation_id: Option<OperationId>,
    correlation_id: CorrelationId,
    attempt_id: AttemptId,
    causal_parent: Option<EventId>,
    actor: ActorReference,
    principal_reference: Option<PseudonymousReference>,
    key_reference: Option<PseudonymousReference>,
    transport: TransportClass,
    operation: OperationClass,
    source: SourceClass,
    kind: EventKind,
    result: EventResult,
    reason: DecisionReason,
    versions: VersionVectorV1,
    payload: EventPayloadV1,
}

impl AuthorizationEventV1 {
    /// Construct schema-V1 evidence from closed trusted fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_id: EventId,
        domain: CommunityId,
        occurred_at: DateTime<Utc>,
        operation_id: Option<OperationId>,
        correlation_id: CorrelationId,
        attempt_id: AttemptId,
        causal_parent: Option<EventId>,
        actor: ActorReference,
        transport: TransportClass,
        operation: OperationClass,
        source: SourceClass,
        kind: EventKind,
        result: EventResult,
        reason: DecisionReason,
        versions: VersionVectorV1,
        payload: EventPayloadV1,
    ) -> Self {
        Self {
            event_id,
            domain,
            occurred_at,
            operation_id,
            correlation_id,
            attempt_id,
            causal_parent,
            actor,
            principal_reference: None,
            key_reference: None,
            transport,
            operation,
            source,
            kind,
            result,
            reason,
            versions,
            payload,
        }
    }

    /// Attach domain-separated principal and key pseudonyms without retaining
    /// the raw issuer, subject, or key bytes.
    pub fn with_subject_references(
        mut self,
        principal_reference: Option<PseudonymousReference>,
        key_reference: Option<PseudonymousReference>,
    ) -> Result<Self, AuthorizationEvidenceError> {
        if principal_reference.is_some_and(|value| value.kind() != ReferenceKind::Principal)
            || key_reference.is_some_and(|value| value.kind() != ReferenceKind::Key)
        {
            return Err(AuthorizationEvidenceError::InvalidPseudonymInput);
        }
        self.principal_reference = principal_reference;
        self.key_reference = key_reference;
        Ok(self)
    }

    /// Event identity.
    pub const fn event_id(&self) -> EventId {
        self.event_id
    }

    /// Authorization domain.
    pub const fn domain(&self) -> CommunityId {
        self.domain
    }

    /// Occurrence time supplied by the trusted decision boundary.
    pub const fn occurred_at(&self) -> DateTime<Utc> {
        self.occurred_at
    }

    /// Semantic operation identity, when applicable.
    pub const fn operation_id(&self) -> Option<OperationId> {
        self.operation_id
    }

    /// Cross-component attempt correlation.
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }

    /// Invocation attempt identity.
    pub const fn attempt_id(&self) -> AttemptId {
        self.attempt_id
    }

    /// Closed event kind.
    pub const fn kind(&self) -> EventKind {
        self.kind
    }

    /// Closed result.
    pub const fn result(&self) -> EventResult {
        self.result
    }

    /// Closed trusted reason.
    pub const fn reason(&self) -> DecisionReason {
        self.reason
    }

    /// Closed actor classification, without its pseudonymous values.
    pub const fn actor_class(&self) -> ActorClass {
        self.actor.class()
    }

    /// Pseudonymous issuer-qualified principal reference, when known.
    pub const fn principal_reference(&self) -> Option<PseudonymousReference> {
        self.principal_reference
    }

    /// Pseudonymous authenticated-key fingerprint, when known.
    pub const fn key_reference(&self) -> Option<PseudonymousReference> {
        self.key_reference
    }

    pub(crate) const fn causal_parent(&self) -> Option<EventId> {
        self.causal_parent
    }

    pub(crate) const fn actor(&self) -> &ActorReference {
        &self.actor
    }

    pub(crate) const fn transport(&self) -> TransportClass {
        self.transport
    }

    pub(crate) const fn operation(&self) -> OperationClass {
        self.operation
    }

    pub(crate) const fn source(&self) -> SourceClass {
        self.source
    }

    pub(crate) const fn versions(&self) -> VersionVectorV1 {
        self.versions
    }

    pub(crate) const fn payload(&self) -> &EventPayloadV1 {
        &self.payload
    }
}

impl fmt::Debug for AuthorizationEventV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationEventV1")
            .field("event_id", &"[redacted]")
            .field("domain", &"[redacted]")
            .field("occurred_at", &self.occurred_at)
            .field("operation_id", &"[redacted]")
            .field("correlation_id", &"[redacted]")
            .field("attempt_id", &"[redacted]")
            .field("causal_parent", &"[redacted]")
            .field("actor", &self.actor)
            .field("principal_reference", &"[redacted]")
            .field("key_reference", &"[redacted]")
            .field("transport", &self.transport)
            .field("operation", &self.operation)
            .field("source", &self.source)
            .field("kind", &self.kind)
            .field("result", &self.result)
            .field("reason", &self.reason)
            .field("versions", &self.versions)
            .field("payload", &self.payload)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::{PseudonymKey, Pseudonymizer};

    #[test]
    fn operator_approvers_are_distinct_and_not_self() {
        let domain = CommunityId::from_uuid(uuid::Uuid::new_v4());
        let pseudonyms = Pseudonymizer::new(PseudonymKey::new([3; 32]).unwrap(), 1);
        let actor = pseudonyms
            .derive(domain, ReferenceKind::Actor, b"actor")
            .unwrap();
        let approver = pseudonyms
            .derive(domain, ReferenceKind::Approver, b"approver")
            .unwrap();
        let value = ActorReference::operator(actor, vec![approver, approver]).unwrap();
        let ActorReference::Operator { approvers, .. } = value else {
            panic!("expected operator actor");
        };
        assert_eq!(approvers.len(), 1);
    }

    #[test]
    fn debug_output_contains_no_reference_digest() {
        let domain = CommunityId::from_uuid(uuid::Uuid::new_v4());
        let pseudonyms = Pseudonymizer::new(PseudonymKey::new([8; 32]).unwrap(), 2);
        let actor = pseudonyms
            .derive(domain, ReferenceKind::Actor, b"synthetic-sensitive-value")
            .unwrap();
        let rendered = format!("{:?}", ActorReference::direct(actor).unwrap());
        assert!(!rendered.contains(&hex::encode(actor.digest())));
        assert!(rendered.contains("[redacted]"));
    }
}
