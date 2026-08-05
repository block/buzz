//! Closed contracts for durable authorization evidence.
//!
//! This module is deliberately separate from the legacy general-purpose audit
//! log. Authorization evidence has no arbitrary JSON or string payload and no
//! raw identity field. Database code assigns stream identity and position only
//! after the event is accepted durably.

mod encoding;
mod event;
mod export;
mod identifiers;
mod registry;

pub use encoding::{AcceptedEventMetadata, CanonicalEvent};
pub use event::{
    ActorReference, AuthorizationEventV1, EventPayloadV1, LifecycleEvidenceV1, VersionVectorV1,
};
pub use export::{
    CapacityClass, ControlCode, DeliveryDisposition, DeliveryFailure, DeliveryKind, DeliveryLease,
    EvidenceHealthSignal, RetryPolicy,
};
pub use identifiers::{
    ApprovalEvidenceId, AttemptId, AuthorityEvidenceId, CorrelationId, DeliveryAttemptId, EffectId,
    EventId, ExporterId, OperationId, PseudonymKey, Pseudonymizer, PseudonymousReference,
    ReceiptId, ReferenceKind, StreamId,
};
pub use registry::{
    ActorClass, DecisionReason, EventKind, EventResult, EvidenceStreamKind, OperationClass,
    SourceClass, TransportClass,
};

use thiserror::Error;

/// Validation or canonical-encoding failure for an authorization event.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AuthorizationEvidenceError {
    /// A required UUID was nil.
    #[error("authorization evidence identifier must not be nil")]
    NilIdentifier,
    /// A temporal bound was invalid.
    #[error("authorization evidence time bounds are invalid")]
    InvalidTime,
    /// Pseudonym input was empty or exceeded its fixed bound.
    #[error("authorization pseudonym input is invalid")]
    InvalidPseudonymInput,
    /// Retry configuration was invalid.
    #[error("authorization evidence retry policy is invalid")]
    InvalidRetryPolicy,
    /// Delivery lease state was invalid.
    #[error("authorization evidence delivery lease is invalid")]
    InvalidDeliveryLease,
}
