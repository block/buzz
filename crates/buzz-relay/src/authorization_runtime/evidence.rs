//! Durable decision-evidence gate for protected authorization outcomes.
//!
//! A newly allowed value is released only after the append-only decision lane
//! accepts its event. A denial is never widened when that lane is degraded;
//! instead, a bounded independent control signal records the evidence gap.

use async_trait::async_trait;
use buzz_audit::authorization::{
    AuthorizationEventV1, CapacityClass, ControlCode, EvidenceHealthSignal,
};
use buzz_db::authorization_evidence::AcceptedEvidence;
use thiserror::Error;

/// Closed decision disposition at the durable-acceptance boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionDisposition {
    /// A value that would newly allow access.
    Allow,
    /// A value that preserves a denial or unavailability outcome.
    Deny,
}

/// Redaction-safe failure returned when a new allow cannot be made durable.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum DecisionEvidenceError {
    /// Durable evidence storage did not accept the event.
    #[error("authorization evidence acceptance is unavailable")]
    AcceptanceUnavailable,
}

impl DecisionEvidenceError {
    /// Stable provider-neutral client/control code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::AcceptanceUnavailable => "authorization_evidence_acceptance_unavailable",
        }
    }
}

/// Minimal object-safe sink needed by the decision gate.
#[async_trait]
pub trait DecisionEvidenceSink: Send + Sync {
    /// Durably accept one immutable decision event.
    async fn accept(
        &self,
        event: &AuthorizationEventV1,
        capacity: CapacityClass,
    ) -> Result<AcceptedEvidence, DecisionEvidenceError>;
}

#[async_trait]
impl DecisionEvidenceSink for buzz_db::Db {
    async fn accept(
        &self,
        event: &AuthorizationEventV1,
        capacity: CapacityClass,
    ) -> Result<AcceptedEvidence, DecisionEvidenceError> {
        self.accept_authorization_decision(event, capacity, ())
            .await
            .map(|accepted| accepted.evidence())
            .map_err(|_| DecisionEvidenceError::AcceptanceUnavailable)
    }
}

/// Decision released by the durable evidence gate.
pub enum AcceptedAuthorizationDecision<T> {
    /// A newly allowed value with its mandatory durable receipt.
    Allow {
        /// Protected value released after commit.
        value: T,
        /// Durable decision-stream receipt.
        evidence: AcceptedEvidence,
    },
    /// A denied value; evidence may be absent only when the independent health
    /// signal records a storage failure.
    Deny {
        /// Original denied value.
        value: T,
        /// Durable decision-stream receipt, when storage accepted it.
        evidence: Option<AcceptedEvidence>,
    },
}

impl<T> std::fmt::Debug for AcceptedAuthorizationDecision<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (disposition, evidence) = match self {
            Self::Allow { evidence, .. } => ("allow", Some(evidence)),
            Self::Deny { evidence, .. } => ("deny", evidence.as_ref()),
        };
        formatter
            .debug_struct("AcceptedAuthorizationDecision")
            .field("disposition", &disposition)
            .field("value", &"[redacted]")
            .field("evidence", &evidence)
            .finish()
    }
}

/// Accept one decision without ever widening a denial during evidence outage.
pub async fn accept_authorization_decision<T>(
    sink: &dyn DecisionEvidenceSink,
    health: &EvidenceHealthSignal,
    event: &AuthorizationEventV1,
    disposition: DecisionDisposition,
    value: T,
) -> Result<AcceptedAuthorizationDecision<T>, DecisionEvidenceError> {
    let capacity = match disposition {
        DecisionDisposition::Allow => CapacityClass::NewAllow,
        DecisionDisposition::Deny => CapacityClass::RestrictiveReserve,
    };
    match sink.accept(event, capacity).await {
        Ok(evidence) => Ok(match disposition {
            DecisionDisposition::Allow => AcceptedAuthorizationDecision::Allow { value, evidence },
            DecisionDisposition::Deny => AcceptedAuthorizationDecision::Deny {
                value,
                evidence: Some(evidence),
            },
        }),
        Err(error) => match disposition {
            DecisionDisposition::Allow => Err(error),
            DecisionDisposition::Deny => {
                health.record(ControlCode::AcceptanceUnavailable);
                Ok(AcceptedAuthorizationDecision::Deny {
                    value,
                    evidence: None,
                })
            }
        },
    }
}
