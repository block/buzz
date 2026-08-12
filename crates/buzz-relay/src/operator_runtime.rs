//! Fail-closed installation boundary for authenticated operator routes.

use axum::Router;

/// Stable installation failures that leave all operator routes unmounted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OperatorRouteInstallError {
    /// The canonical atomic admission authority was not configured.
    #[error("canonical admission committer is unavailable")]
    MissingCanonicalAdmissionCommitter,
    /// Lifecycle, replay, quota, or audit health was not ready.
    #[error("operator lifecycle dependencies are unhealthy")]
    UnhealthyDependencies,
    /// Installation was attempted more than once for one runtime.
    #[error("operator routes are already installed")]
    DuplicateInstallation,
}

/// Typed all-or-nothing operator route installation boundary.
///
/// Implementations must install only authenticated, allowlisted routes backed
/// by the canonical admission committer. An error returns no partially mounted
/// router and must not weaken the caller's default-deny fallback.
pub trait OperatorRouteInstaller: Send + Sync {
    /// Application state carried by the relay router.
    type State: Clone + Send + Sync + 'static;

    /// Install the complete closed operator route set.
    fn install_operator_routes(
        &self,
        router: Router<Self::State>,
    ) -> Result<Router<Self::State>, OperatorRouteInstallError>;
}
