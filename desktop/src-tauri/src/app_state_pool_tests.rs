//! Pinned-constant tests for the shared HTTP client pool config.
//!
//! Kept in a sibling file (not `app_state_tests.rs`, which is already at the
//! 1000-line gate override) so these guard the pool constants without growing
//! the oversized test module. `#[path]`-included from `app_state.rs`.
//!
//! reqwest exposes no getter for a built client's pool settings, so these pin
//! the source-of-truth constants that `build_app_state` feeds into the builder.
//! They guard against a silent revert to the previous tighter 10s/1 values,
//! which forced fresh TLS handshakes on ordinary channel-open / prefetch bursts.

use super::*;

#[test]
fn http_pool_idle_timeout_is_ninety_seconds() {
    assert_eq!(
        HTTP_POOL_IDLE_TIMEOUT,
        std::time::Duration::from_secs(90),
        "idle keep-alives must survive realistic dwell time so a follow-up \
         /query reuses the warm connection"
    );
}

#[test]
fn http_pool_max_idle_per_host_is_eight() {
    assert_eq!(
        HTTP_POOL_MAX_IDLE_PER_HOST, 8,
        "retain a small warm set for concurrent multi-channel prefetch, bounded \
         so a media/model burst cannot pin an unbounded socket set"
    );
}
