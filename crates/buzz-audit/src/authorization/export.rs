use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};

use super::{
    AuthorizationEvidenceError, CanonicalEvent, DeliveryAttemptId, EventId, EvidenceStreamKind,
    StreamId,
};

/// Evidence capacity priority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum CapacityClass {
    /// Denial, revocation, expiry, containment, and replay evidence.
    RestrictiveReserve = 1,
    /// A new allow or widening transition.
    NewAllow = 2,
    /// Nonessential inspection or preview evidence.
    NonessentialRead = 3,
}

/// Closed low-cardinality pipeline control signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum ControlCode {
    /// Evidence storage could not durably accept a decision.
    AcceptanceUnavailable = 1,
    /// Reserved capacity was exhausted.
    CapacityExhausted = 2,
    /// Export sink was unavailable.
    SinkUnavailable = 3,
    /// Sink rejected the event as poison.
    PoisonEvent = 4,
    /// Event schema is not supported by the sink.
    UnsupportedSchema = 5,
    /// Stream or event digest did not verify.
    IntegrityFailure = 6,
    /// Delivery lease expired before acknowledgement.
    LeaseExpired = 7,
    /// Restore reconciliation found inconsistent evidence.
    RestoreMismatch = 8,
}

impl ControlCode {
    /// Stable provider-neutral code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::AcceptanceUnavailable => "acceptance_unavailable",
            Self::CapacityExhausted => "capacity_exhausted",
            Self::SinkUnavailable => "sink_unavailable",
            Self::PoisonEvent => "poison_event",
            Self::UnsupportedSchema => "unsupported_schema",
            Self::IntegrityFailure => "integrity_failure",
            Self::LeaseExpired => "lease_expired",
            Self::RestoreMismatch => "restore_mismatch",
        }
    }
}

/// Immutable event lane selected for export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeliveryKind {
    /// Transactional outbox event.
    AuditOutbox,
    /// Non-mutating decision event.
    Decision,
}

impl DeliveryKind {
    /// Corresponding durable stream lane.
    pub const fn stream_kind(self) -> EvidenceStreamKind {
        match self {
            Self::AuditOutbox => EvidenceStreamKind::AuditOutbox,
            Self::Decision => EvidenceStreamKind::Decision,
        }
    }
}

/// Bounded retry policy for exporter delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    initial_delay: Duration,
    maximum_delay: Duration,
    maximum_attempts: u32,
}

impl RetryPolicy {
    /// Validate retry bounds.
    pub fn new(
        initial_delay: Duration,
        maximum_delay: Duration,
        maximum_attempts: u32,
    ) -> Result<Self, AuthorizationEvidenceError> {
        if initial_delay.is_zero()
            || maximum_delay < initial_delay
            || maximum_delay > Duration::from_secs(24 * 60 * 60)
            || maximum_attempts == 0
            || maximum_attempts > 100
        {
            return Err(AuthorizationEvidenceError::InvalidRetryPolicy);
        }
        Ok(Self {
            initial_delay,
            maximum_delay,
            maximum_attempts,
        })
    }

    /// Exponential delay capped at the configured maximum.
    pub fn delay_for(self, attempt: u32) -> Option<Duration> {
        if attempt == 0 || attempt > self.maximum_attempts {
            return None;
        }
        let multiplier = 1_u32.checked_shl(attempt.saturating_sub(1).min(31))?;
        Some(
            self.initial_delay
                .checked_mul(multiplier)
                .unwrap_or(self.maximum_delay)
                .min(self.maximum_delay),
        )
    }

    /// Maximum attempts before quarantine/dead-letter handling.
    pub const fn maximum_attempts(self) -> u32 {
        self.maximum_attempts
    }
}

/// Claimed immutable event plus mutable lease facts.
#[derive(Clone, PartialEq, Eq)]
pub struct DeliveryLease {
    /// Event lane.
    kind: DeliveryKind,
    /// Event identity.
    event_id: EventId,
    /// Stream identity.
    stream_id: StreamId,
    /// Stream-local position.
    stream_position: u64,
    /// Unique identity for this exporter attempt.
    delivery_attempt_id: DeliveryAttemptId,
    /// Bounded attempt ordinal.
    attempt: u32,
    /// Lease expiry under database time.
    lease_expires_at: DateTime<Utc>,
    /// Canonical immutable event bytes.
    canonical_event: Vec<u8>,
    /// Semantic content digest required for sink acknowledgement.
    content_digest: [u8; 32],
    /// Expected stream-chain digest.
    chain_digest: [u8; 32],
}

impl DeliveryLease {
    /// Construct a lease from transactionally claimed storage facts.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: DeliveryKind,
        event_id: EventId,
        stream_id: StreamId,
        stream_position: u64,
        delivery_attempt_id: DeliveryAttemptId,
        attempt: u32,
        lease_expires_at: DateTime<Utc>,
        canonical_event: Vec<u8>,
        content_digest: [u8; 32],
        chain_digest: [u8; 32],
    ) -> Self {
        Self {
            kind,
            event_id,
            stream_id,
            stream_position,
            delivery_attempt_id,
            attempt,
            lease_expires_at,
            canonical_event,
            content_digest,
            chain_digest,
        }
    }

    /// Event lane.
    pub const fn kind(&self) -> DeliveryKind {
        self.kind
    }
    /// Event identity.
    pub const fn event_id(&self) -> EventId {
        self.event_id
    }
    /// Stream identity.
    pub const fn stream_id(&self) -> StreamId {
        self.stream_id
    }
    /// Stream-local position.
    pub const fn stream_position(&self) -> u64 {
        self.stream_position
    }
    /// Unique identity for this exporter attempt.
    pub const fn delivery_attempt_id(&self) -> DeliveryAttemptId {
        self.delivery_attempt_id
    }
    /// Bounded attempt ordinal.
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }
    /// Lease expiry under database time.
    pub const fn lease_expires_at(&self) -> DateTime<Utc> {
        self.lease_expires_at
    }
    /// Canonical immutable bytes presented to the sink.
    pub fn canonical_event(&self) -> &[u8] {
        &self.canonical_event
    }
    /// Semantic content digest the sink must acknowledge.
    pub const fn content_digest(&self) -> [u8; 32] {
        self.content_digest
    }
    /// Expected stream-chain digest.
    pub const fn chain_digest(&self) -> [u8; 32] {
        self.chain_digest
    }

    /// Validate lease and payload bounds before handing it to a sink.
    pub fn validate(&self, now: DateTime<Utc>) -> Result<(), AuthorizationEvidenceError> {
        if self.stream_position == 0
            || self.attempt == 0
            || self.lease_expires_at <= now
            || self.canonical_event.is_empty()
            || self.canonical_event.len() > 64 * 1024
        {
            return Err(AuthorizationEvidenceError::InvalidDeliveryLease);
        }
        CanonicalEvent::verify_accepted_bytes(
            &self.canonical_event,
            self.stream_id,
            self.stream_position,
            self.content_digest,
            self.chain_digest,
        )
    }
}

impl std::fmt::Debug for DeliveryLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeliveryLease")
            .field("kind", &self.kind)
            .field("event_id", &"[redacted]")
            .field("stream_id", &"[redacted]")
            .field("stream_position", &self.stream_position)
            .field("delivery_attempt_id", &"[redacted]")
            .field("attempt", &self.attempt)
            .field("lease_expires_at", &self.lease_expires_at)
            .field("canonical_event", &"[redacted]")
            .field("content_digest", &"[redacted]")
            .field("chain_digest", &"[redacted]")
            .finish()
    }
}

/// Sink outcome for one claimed delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryDisposition {
    /// Sink durably accepted this exact event.
    Accepted,
    /// Retry according to bounded policy.
    Retry(ControlCode),
    /// Quarantine while preserving the immutable event.
    Quarantine(ControlCode),
}

/// Redaction-safe delivery failure returned by a sink adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeliveryFailure {
    /// Closed control code; no raw upstream error is retained.
    pub control_code: ControlCode,
    /// Whether the adapter considers the failure retryable.
    pub retryable: bool,
}

/// Independent bounded counters for evidence-pipeline control gaps.
///
/// Counters intentionally accept only [`ControlCode`]. They cannot retain an
/// identity, correlation, route, provider error, or credential fragment.
#[derive(Debug, Default)]
pub struct EvidenceHealthSignal {
    acceptance_unavailable: AtomicU64,
    capacity_exhausted: AtomicU64,
    sink_unavailable: AtomicU64,
    poison_event: AtomicU64,
    unsupported_schema: AtomicU64,
    integrity_failure: AtomicU64,
    lease_expired: AtomicU64,
    restore_mismatch: AtomicU64,
}

impl EvidenceHealthSignal {
    /// Increment one closed control category with saturation.
    pub fn record(&self, code: ControlCode) {
        let counter = match code {
            ControlCode::AcceptanceUnavailable => &self.acceptance_unavailable,
            ControlCode::CapacityExhausted => &self.capacity_exhausted,
            ControlCode::SinkUnavailable => &self.sink_unavailable,
            ControlCode::PoisonEvent => &self.poison_event,
            ControlCode::UnsupportedSchema => &self.unsupported_schema,
            ControlCode::IntegrityFailure => &self.integrity_failure,
            ControlCode::LeaseExpired => &self.lease_expired,
            ControlCode::RestoreMismatch => &self.restore_mismatch,
        };
        let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            Some(value.saturating_add(1))
        });
    }

    /// Read one counter without exposing labels or event data.
    pub fn count(&self, code: ControlCode) -> u64 {
        let counter = match code {
            ControlCode::AcceptanceUnavailable => &self.acceptance_unavailable,
            ControlCode::CapacityExhausted => &self.capacity_exhausted,
            ControlCode::SinkUnavailable => &self.sink_unavailable,
            ControlCode::PoisonEvent => &self.poison_event,
            ControlCode::UnsupportedSchema => &self.unsupported_schema,
            ControlCode::IntegrityFailure => &self.integrity_failure,
            ControlCode::LeaseExpired => &self.lease_expired,
            ControlCode::RestoreMismatch => &self.restore_mismatch,
        };
        counter.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_is_bounded_and_capped() {
        let policy = RetryPolicy::new(Duration::from_secs(2), Duration::from_secs(10), 5).unwrap();
        assert_eq!(policy.delay_for(1), Some(Duration::from_secs(2)));
        assert_eq!(policy.delay_for(4), Some(Duration::from_secs(10)));
        assert_eq!(policy.delay_for(6), None);
    }

    #[test]
    fn restrictive_evidence_has_highest_capacity_priority() {
        assert!(CapacityClass::RestrictiveReserve < CapacityClass::NewAllow);
        assert!(CapacityClass::NewAllow < CapacityClass::NonessentialRead);
    }

    #[test]
    fn health_signal_is_bounded_and_saturating() {
        let signal = EvidenceHealthSignal::default();
        signal.record(ControlCode::AcceptanceUnavailable);
        assert_eq!(signal.count(ControlCode::AcceptanceUnavailable), 1);
        assert_eq!(signal.count(ControlCode::IntegrityFailure), 0);
    }

    #[test]
    fn delivery_debug_never_renders_canonical_bytes_or_digests() {
        let lease = DeliveryLease::new(
            DeliveryKind::AuditOutbox,
            EventId::generate(),
            StreamId::generate(),
            1,
            DeliveryAttemptId::generate(),
            1,
            Utc::now() + chrono::Duration::minutes(1),
            b"planted-private-claim-canary".to_vec(),
            [17; 32],
            [23; 32],
        );
        let rendered = format!("{lease:?}");
        assert!(rendered.contains("[redacted]"));
        assert!(!rendered.contains("planted-private-claim-canary"));
        assert!(!rendered.contains(&hex::encode([17; 32])));
        assert!(!rendered.contains(&hex::encode([23; 32])));
    }
}
