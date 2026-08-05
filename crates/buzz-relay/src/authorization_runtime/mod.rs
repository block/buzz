//! Provider-neutral runtime authorization seams.
//!
//! This commit registers the complete O4 module shape while implementing only
//! exact-domain provider selection, provider-evidence finalization, and bounded
//! leases. Transport adoption, invalidation, and client status remain separate
//! extension lanes.

pub(crate) mod ephemeral;
/// Durable decision-evidence acceptance and fail-closed release.
pub mod evidence;
/// Transaction-owned protected mutation execution and idempotency.
pub mod executor;
/// Exact-domain provider selection and authorization finalization.
pub mod finalization;
/// Durable provider-neutral invalidation, reconciliation, and use fences.
pub mod invalidation;
/// Disabled-by-default production runtime construction.
pub mod production;
/// Independent high-water protection against stale PostgreSQL restoration.
pub mod restore;
/// Reserved provider-neutral client-status extension seam.
pub mod status;
/// Reserved provider-neutral transport-adoption extension seam.
pub mod transport;
