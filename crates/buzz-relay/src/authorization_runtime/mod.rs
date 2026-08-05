//! Provider-neutral runtime authorization seams.

pub(crate) mod ephemeral;
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
/// Protected transport authorization and lease fencing.
pub mod transport;
