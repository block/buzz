//! Provider-neutral object-store error taxonomy.
//!
//! The distinction that matters most here is **classified vs. unknown**. A
//! provider that answered — with a status, a body, or a malformed response —
//! told us something about the object's state. A request that never got a
//! classified answer (socket refused, send flaked, mid-flight reset) told us
//! nothing: it is neither evidence that the write committed nor that it did
//! not. The Git conformance probe drops exactly the unknown outcomes from its
//! observer set, so [`ObjectStoreError::TransportAmbiguous`] must stay
//! reserved for pre-classification failures and nothing else.

use std::time::Duration;

use crate::revision::ProviderKind;

/// Everything a provider operation can fail with.
///
/// Losing a compare-and-swap race is deliberately *not* in here — that is
/// [`crate::ConditionalWrite::Conflict`], an ordinary outcome.
#[derive(Debug, thiserror::Error)]
pub enum ObjectStoreError {
    /// The requested key does not exist.
    #[error("object not found: {key}")]
    NotFound {
        /// Object key that was addressed.
        key: String,
    },

    /// A precondition failed on an operation that is not a conditional write.
    ///
    /// Conditional writes surface a failed precondition as
    /// [`crate::ConditionalWrite::Conflict`] instead; this variant catches a
    /// precondition failure arriving where no CAS semantics were requested.
    #[error("precondition failed on {key}")]
    Conflict {
        /// Object key that was addressed.
        key: String,
    },

    /// The provider rejected the request for exceeding its request rate.
    ///
    /// Throttling is never evidence that a writer lost a CAS race.
    #[error("object store throttled the {operation} request")]
    Throttled {
        /// Provider operation that was throttled.
        operation: &'static str,
        /// Provider-advertised backoff, when the response carried one.
        retry_after: Option<Duration>,
    },

    /// The provider answered with a transient failure; the operation may be
    /// retried under the caller's policy (with its original precondition, if
    /// it had one).
    #[error("retryable object store failure during {operation}: {message}")]
    TransportRetryable {
        /// Provider operation that failed.
        operation: &'static str,
        /// Redacted provider detail — never object bytes or credentials.
        message: String,
    },

    /// The request never produced a classified provider response, so the
    /// operation's outcome is unknown.
    ///
    /// A conditional write that fails this way must never be retried
    /// unconditionally: reread the object and classify the committed revision
    /// instead.
    #[error("ambiguous object store outcome during {operation}: {message}")]
    TransportAmbiguous {
        /// Provider operation whose outcome is unknown.
        operation: &'static str,
        /// Redacted provider detail — never object bytes or credentials.
        message: String,
    },

    /// A permanent provider or authorization failure.
    #[error("object store backend error during {operation}: {message}")]
    Provider {
        /// Provider operation that failed.
        operation: &'static str,
        /// Redacted provider detail — never object bytes or credentials.
        message: String,
    },

    /// Invalid storage configuration, detected at client construction.
    #[error("object store config error: {0}")]
    Config(String),

    /// The object is larger than the caller's bounded read budget.
    #[error("object too large: {key} is {size} bytes (max {max})")]
    ObjectTooLarge {
        /// Object key that was read.
        key: String,
        /// Object size reported by the provider.
        size: u64,
        /// Maximum bytes the caller allows for this read.
        max: u64,
    },

    /// A content-addressed read returned bytes that do not hash to the key.
    #[error("digest mismatch on {key}: expected {expected}, got {actual}")]
    DigestMismatch {
        /// Object key that was read.
        key: String,
        /// Digest the caller expected (the content-addressed key).
        expected: String,
        /// Digest computed from the returned bytes.
        actual: String,
    },

    /// A revision minted by one provider was presented to another.
    #[error("revision provider mismatch: expected {expected}, got {actual}")]
    RevisionMismatch {
        /// Provider the operation is running against.
        expected: ProviderKind,
        /// Provider that minted the offered revision.
        actual: ProviderKind,
    },
}

impl ObjectStoreError {
    /// Whether the operation's outcome is unknown rather than classified.
    ///
    /// Callers that reason about *observers* — the Git conformance probe, and
    /// any future CAS retry policy — use this to drop unknown outcomes from
    /// the evidence set rather than counting them as a lost race.
    pub fn is_ambiguous(&self) -> bool {
        matches!(self, Self::TransportAmbiguous { .. })
    }

    /// Whether the provider indicated the request may be retried as-is.
    ///
    /// A conditional write may only be retried with its original precondition.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Throttled { .. } | Self::TransportRetryable { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pre_classification_failures_are_ambiguous() {
        assert!(ObjectStoreError::TransportAmbiguous {
            operation: "put",
            message: "connection reset".into(),
        }
        .is_ambiguous());

        for classified in [
            ObjectStoreError::NotFound { key: "k".into() },
            ObjectStoreError::Conflict { key: "k".into() },
            ObjectStoreError::Throttled {
                operation: "put",
                retry_after: None,
            },
            ObjectStoreError::TransportRetryable {
                operation: "put",
                message: "503".into(),
            },
            ObjectStoreError::Provider {
                operation: "put",
                message: "403".into(),
            },
            ObjectStoreError::Config("bad".into()),
        ] {
            assert!(
                !classified.is_ambiguous(),
                "{classified} must stay a classified observation"
            );
        }
    }

    #[test]
    fn throttling_and_transient_failures_are_retryable() {
        assert!(ObjectStoreError::Throttled {
            operation: "get",
            retry_after: Some(Duration::from_secs(1)),
        }
        .is_retryable());
        assert!(ObjectStoreError::TransportRetryable {
            operation: "get",
            message: "503".into(),
        }
        .is_retryable());
        assert!(!ObjectStoreError::TransportAmbiguous {
            operation: "get",
            message: "reset".into(),
        }
        .is_retryable());
        assert!(!ObjectStoreError::NotFound { key: "k".into() }.is_retryable());
    }
}
