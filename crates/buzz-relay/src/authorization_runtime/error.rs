//! Stable, credential-free runtime failures.

use thiserror::Error;

/// Fail-closed errors returned by the provider-free relay runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RuntimeAuthorizationError {
    /// The sole runtime configuration was absent or internally inconsistent.
    #[error("invalid provider-free authorization configuration")]
    InvalidConfiguration,
    /// The closed protected-route inventory was incomplete or ambiguous.
    #[error("protected route inventory is incomplete")]
    IncompleteRouteInventory,
    /// No exact protected route matched the request.
    #[error("protected route is not classified")]
    UnknownProtectedRoute,
    /// The verified transport did not match the registered route.
    #[error("protected route transport mismatch")]
    TransportMismatch,
    /// A protected resource witness did not match route or authorization state.
    #[error("protected resource witness mismatch")]
    ResourceWitnessMismatch,
    /// A required storage, lifecycle, transport, or status dependency is absent.
    #[error("provider-free authorization dependency unavailable")]
    DependencyUnavailable,
    /// Verifier policy, keys, or freshness changed before final use.
    #[error("provider-free verifier state is stale")]
    StaleVerifier,
    /// The current verifier rejected the presented assertion.
    #[error("provider-free verifier rejected assertion")]
    AssertionDenied,
    /// The assertion's exact half-open validity interval has elapsed.
    #[error("provider-free assertion expired")]
    AssertionExpired,
    /// Invalidation delivery or reconciliation is not healthy.
    #[error("authorization invalidation state is unavailable")]
    InvalidationUnavailable,
    /// Restore evidence was missing, ambiguous, stale, or inconsistent.
    #[error("authorization restore was rejected")]
    RestoreRejected,
    /// Current status could not be safely produced or withdrawn.
    #[error("current authorization status is unavailable")]
    StatusUnavailable,
    /// Startup attempted to install a partial or mixed-lineage runtime.
    #[error("provider-free runtime state is incomplete")]
    PartialState,
    /// The installed runtime is not ready for protected work.
    #[error("provider-free runtime is not ready")]
    NotReady,
}

impl RuntimeAuthorizationError {
    /// Stable machine code safe for logs, metrics, and protocol denials.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "nip_fi_runtime_invalid_config",
            Self::IncompleteRouteInventory => "nip_fi_runtime_incomplete_routes",
            Self::UnknownProtectedRoute => "nip_fi_runtime_unknown_route",
            Self::TransportMismatch => "nip_fi_runtime_transport_mismatch",
            Self::ResourceWitnessMismatch => "nip_fi_runtime_resource_witness_mismatch",
            Self::DependencyUnavailable => "nip_fi_runtime_dependency_unavailable",
            Self::StaleVerifier => "nip_fi_runtime_stale_verifier",
            Self::AssertionDenied => "nip_fi_runtime_assertion_denied",
            Self::AssertionExpired => "nip_fi_auth_expired",
            Self::InvalidationUnavailable => "nip_fi_runtime_invalidation_unavailable",
            Self::RestoreRejected => "nip_fi_runtime_restore_rejected",
            Self::StatusUnavailable => "nip_fi_runtime_status_unavailable",
            Self::PartialState => "nip_fi_runtime_partial_state",
            Self::NotReady => "nip_fi_runtime_not_ready",
        }
    }
}
