//! Provider-neutral authorization interfaces available before runtime installation.

/// Fail-closed ephemeral-authority interfaces installed by the session slice.
pub mod ephemeral;
/// Fail-closed transaction interfaces installed by the finalization slice.
pub mod executor;
/// Activation and enrollment types consumed by protected transports.
pub mod finalization;
/// Fail-closed restore-witness interfaces installed by the invalidation slice.
pub mod restore;
/// Protected transport authorization and lease fencing.
pub mod transport;
