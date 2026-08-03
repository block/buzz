//! Deterministic cache contract for the sibling-authorization path that gates
//! whether an accepted event may wake a lazy ACP pool.

#[allow(dead_code)]
#[path = "../src/sibling_auth.rs"]
mod sibling_auth;

use std::time::Duration;

use sibling_auth::{OwnerCache, NEGATIVE_SIBLING_CACHE_TTL};
use tokio::time::Instant;

#[test]
fn verified_sibling_remains_authorized_for_later_wakes() {
    let cache = OwnerCache::new(Some("synthetic-owner".into()));
    let first_wake = Instant::now();
    cache.cache_sibling_at("synthetic-sibling".into(), true, first_wake);

    assert_eq!(cache.get(), Some("synthetic-owner"));
    assert_eq!(
        cache.is_known_sibling_at(
            "synthetic-sibling",
            first_wake + NEGATIVE_SIBLING_CACHE_TTL + Duration::from_secs(86_400),
        ),
        Some(true),
        "a cryptographically verified sibling must remain eligible to wake the pool",
    );
}

#[test]
fn denied_sibling_is_retried_at_the_exact_expiry_boundary() {
    let cache = OwnerCache::new(Some("synthetic-owner".into()));
    let first_attempt = Instant::now();
    cache.cache_sibling_at("synthetic-candidate".into(), false, first_attempt);

    assert_eq!(
        cache.is_known_sibling_at(
            "synthetic-candidate",
            first_attempt + NEGATIVE_SIBLING_CACHE_TTL - Duration::from_nanos(1),
        ),
        Some(false),
        "negative authorization must suppress repeated profile lookups before expiry",
    );
    assert_eq!(
        cache.is_known_sibling_at(
            "synthetic-candidate",
            first_attempt + NEGATIVE_SIBLING_CACHE_TTL,
        ),
        None,
        "the expiry boundary must evict the denial so a newly tagged profile can be retried",
    );

    cache.cache_sibling_at(
        "synthetic-candidate".into(),
        true,
        first_attempt + NEGATIVE_SIBLING_CACHE_TTL,
    );
    assert_eq!(
        cache.is_known_sibling_at(
            "synthetic-candidate",
            first_attempt + NEGATIVE_SIBLING_CACHE_TTL + Duration::from_secs(1),
        ),
        Some(true),
        "a valid profile attestation found after expiry must authorize the next wake",
    );
}
