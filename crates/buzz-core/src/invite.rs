//! Shared server-side contract constants for relay invite links.
//!
//! Both the relay API and database persistence layer depend on `buzz-core`, so
//! the lifetime bounds live here rather than being duplicated across crates.

/// Minimum invite lifetime accepted by the mint API: 60 seconds.
pub const MIN_INVITE_TTL_SECS: u64 = 60;

/// Default invite lifetime when the mint request omits `ttl_secs`: 72 hours.
pub const DEFAULT_INVITE_TTL_SECS: u64 = 72 * 60 * 60;

/// Maximum invite lifetime accepted by the mint API: 30 days.
pub const MAX_INVITE_TTL_SECS: u64 = 30 * 24 * 60 * 60;
