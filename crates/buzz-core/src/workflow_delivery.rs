//! Shared bounds for exclusive durable workflow-delivery leases.

/// Default lease used by clients that do not request an explicit duration.
pub const DEFAULT_LEASE_SECONDS: i64 = 120;

/// Maximum lease accepted by the relay.
///
/// This covers buzz-acp's supported seven-day turn ceiling plus its 400-second
/// local retry and handoff margin.
pub const MAX_LEASE_SECONDS: i64 = 604_800 + 400;

/// Lifetime of a durable delivery row.
///
/// The extra day permits delayed polling before a maximum-duration lease is
/// claimed while still leaving the complete supported turn window available.
pub const ROW_LIFETIME_SECONDS: i64 = MAX_LEASE_SECONDS + 24 * 60 * 60;
