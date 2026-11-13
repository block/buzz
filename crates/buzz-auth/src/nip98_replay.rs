//! NIP-98 replay protection — shared, community-scoped, atomic seen-set.
//!
//! NIP-98 verification ([`crate::nip98::verify_nip98_event`]) is structurally
//! complete: it checks signature, kind, timestamp window, URL, method, and
//! optional body hash. It does **not** check whether the same event id has
//! already been used — that requires shared state. With multiple relay pods
//! ("any pod, any connection" per the rewrite §4 architecture), an in-process
//! cache (moka, DashMap) does not carry the freshness proof across pods, so
//! replay protection is a §5 hard gate.
//!
//! The required shape (§5):
//!
//! - shared state (Redis), atomic set-if-absent, TTL ≥ [`DEFAULT_REPLAY_TTL_SECS`]
//! - community-scoped key — see [`nip98_replay_key`]
//!
//! ## Usage shape
//!
//! Verify first, then mark. Burning a seen-set slot on a forgery would let an
//! attacker who knows a future event id of a victim DoS the legitimate event.
//!
//! ```ignore
//! let pubkey = buzz_auth::verify_nip98_event(json, url, method, body)?;
//! if !replay.try_mark(&ctx, &event_id, buzz_auth::DEFAULT_REPLAY_TTL_SECS).await? {
//!     return Err(AuthError::Nip98Replay);
//! }
//! // safe to honor the request as `pubkey`
//! ```
//!
//! The TTL must cover the verifier's clock-skew tolerance (±120s, so the
//! window over which a duplicate event id is even plausible is 2×120 = 240s),
//! plus the slack described on [`DEFAULT_REPLAY_TTL_SECS`].
//! [`DEFAULT_REPLAY_TTL_SECS`] is the floor; deployments may raise it.

use std::{future::Future, pin::Pin};

use buzz_core::TenantContext;
use nostr::EventId;

use crate::{error::AuthError, TIMESTAMP_TOLERANCE_SECS};

/// Floor for the replay-prevention window, in seconds.
///
/// An event is admissible for `2 × TIMESTAMP_TOLERANCE_SECS` (240s) of
/// verifier time, plus 1s because both whole-second bounds are inclusive.
/// The remaining 9s budget covers the clock difference between any two relay
/// pods sharing this seen-set: a marker written by one pod must outlive the
/// last second another pod would still admit the event. That budget assumes
/// NTP-synced pods whose pairwise difference stays within 9s; it is an
/// operational assumption, not something this constant can enforce.
///
/// Implementations MAY use a larger TTL for safety margin; they MUST NOT use
/// a smaller one.
pub const DEFAULT_REPLAY_TTL_SECS: u64 = 250;

/// Pairwise relay-pod clock difference budgeted into [`DEFAULT_REPLAY_TTL_SECS`].
const POD_CLOCK_DIFFERENCE_BUDGET_SECS: u64 = 9;

const _: () = assert!(
    DEFAULT_REPLAY_TTL_SECS >= 2 * TIMESTAMP_TOLERANCE_SECS + 1 + POD_CLOCK_DIFFERENCE_BUDGET_SECS
);

/// Ceiling for the replay-prevention window, in seconds.
///
/// Any TTL beyond an hour is implausible for NIP-98 replay protection: the
/// verifier only accepts events within ±120s, so a same-id replay is only
/// physically possible inside that window plus clock skew. A 1-hour cap is
/// 30× the natural maximum and still keeps Redis values well inside
/// `i64::MAX` seconds (which Redis `EX` requires). Anything larger reaching
/// this code is a config/caller bug; implementations MUST clamp down to it
/// rather than admit values that risk Redis `EX` parse failures or
/// pathologically long-lived seen-set entries.
pub const MAX_REPLAY_TTL_SECS: u64 = 3600;

/// Shared seen-set for NIP-98 event ids, scoped per community.
///
/// The production implementation lives in `buzz-pubsub` (Redis `SET NX EX`).
/// A test impl is provided behind `cfg(any(test, feature = "test-utils"))`.
pub trait Nip98ReplayGuard: Send + Sync {
    /// Atomically claim `event_id` in an explicit deployment or community scope.
    fn try_mark_in_scope<'a>(
        &'a self,
        scope: &'a str,
        event_id: &'a EventId,
        ttl_secs: u64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>>;

    /// Atomically claim `event_id` for `ctx`'s community.
    ///
    /// Returns `Ok(true)` when the id is newly inserted (proceed) and
    /// `Ok(false)` when an entry already exists (the caller MUST reject the
    /// request as replay).
    ///
    /// On `Err` (Redis unreachable, etc.) callers MUST fail closed — reject
    /// the request rather than admitting it. The shared seen-set is a
    /// correctness fence; degrading to "best effort, allow on error" forfeits
    /// the freshness proof.
    ///
    /// Implementations MUST use an atomic set-if-absent operation; a
    /// read-then-write sequence loses to concurrent inserts and forfeits the
    /// freshness proof.
    ///
    /// `ttl_secs` MUST be at least [`DEFAULT_REPLAY_TTL_SECS`]. Implementations
    /// MAY clamp a smaller value up to the floor rather than reject; they MUST
    /// NOT honor it as-given.
    ///
    /// `ttl_secs` MUST be clamped down to [`MAX_REPLAY_TTL_SECS`] if larger.
    /// The replay window's natural maximum is the verifier's ±120s tolerance;
    /// values past an hour are implausible and risk Redis `EX` parse failures
    /// (Redis interprets `EX` as a signed 64-bit integer).
    fn try_mark<'a>(
        &'a self,
        ctx: &'a TenantContext,
        event_id: &'a EventId,
        ttl_secs: u64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>> {
        let scope = ctx.community().to_string();
        Box::pin(async move { self.try_mark_in_scope(&scope, event_id, ttl_secs).await })
    }
}

/// Redis key for a NIP-98 replay marker:
/// `buzz:{community}:nip98:{event_id_hex}`.
///
/// The community prefix is the S1 isolation fence at the replay layer.
/// Event ids are content-addressed (SHA-256 of the canonical event tuple) so
/// natural cross-community collision is zero, but the gate is fail-closed
/// isolation: a same-id replay across communities must consult two distinct
/// seen-set rows, not one shared row.
pub fn nip98_replay_key(ctx: &TenantContext, event_id: &EventId) -> String {
    nip98_replay_key_for_scope(&ctx.community().to_string(), event_id)
}

/// Redis key for a NIP-98 replay marker in an explicit trusted scope.
pub fn nip98_replay_key_for_scope(scope: &str, event_id: &EventId) -> String {
    format!("buzz:{scope}:nip98:{}", event_id.to_hex())
}

/// Always-fresh seen-set for unit tests — every `try_mark` returns `Ok(true)`.
///
/// Use only in test code that does not exercise the replay path itself.
#[cfg(any(test, feature = "test-utils"))]
pub struct AlwaysFreshReplayGuard;

#[cfg(any(test, feature = "test-utils"))]
impl Nip98ReplayGuard for AlwaysFreshReplayGuard {
    fn try_mark_in_scope<'a>(
        &'a self,
        _scope: &'a str,
        _event_id: &'a EventId,
        _ttl_secs: u64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>> {
        Box::pin(async { Ok(true) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::CommunityId;
    use nostr::{EventBuilder, Keys, Kind};
    use sha2::{Digest, Sha256};
    use uuid::Uuid;

    fn fixture_ctx(host: &str) -> TenantContext {
        let bytes = Sha256::digest(host.as_bytes());
        let mut uuid_bytes = [0u8; 16];
        uuid_bytes.copy_from_slice(&bytes[..16]);
        let id = CommunityId::from_uuid(Uuid::from_bytes(uuid_bytes));
        TenantContext::resolved(id, host)
    }

    fn fixture_event_id() -> EventId {
        let keys = Keys::generate();
        EventBuilder::new(Kind::HttpAuth, "")
            .sign_with_keys(&keys)
            .expect("sign")
            .id
    }

    #[test]
    fn key_includes_community_prefix() {
        let ctx = fixture_ctx("relay-a.example");
        let eid = fixture_event_id();
        let key = nip98_replay_key(&ctx, &eid);
        let expected_prefix = format!("buzz:{}:nip98:", ctx.community());
        assert!(
            key.starts_with(&expected_prefix),
            "key {key} should start with {expected_prefix}"
        );
        assert!(key.ends_with(&eid.to_hex()));
    }

    #[test]
    fn key_isolates_communities_for_same_event_id() {
        // Belt-and-suspenders: even if a same-id event surfaces in two
        // communities (which content-addressing makes implausible), the
        // seen-set MUST consult two distinct rows.
        let eid = fixture_event_id();
        let ctx_a = fixture_ctx("relay-a.example");
        let ctx_b = fixture_ctx("relay-b.example");
        let key_a = nip98_replay_key(&ctx_a, &eid);
        let key_b = nip98_replay_key(&ctx_b, &eid);
        assert_ne!(
            key_a, key_b,
            "same event id in two communities must not share a seen-set key"
        );
    }

    #[test]
    fn key_components_are_lowercase() {
        // Stability/idempotence: if event id hex or community Display ever
        // started emitting uppercase, a same logical claim would produce two
        // distinct Redis rows → the seen-set would no longer be a seen-set.
        let ctx = fixture_ctx("relay-a.example");
        let eid = fixture_event_id();
        let key = nip98_replay_key(&ctx, &eid);
        for c in key.chars() {
            assert!(
                !c.is_ascii_uppercase(),
                "nip98 replay key {key} must be all-lowercase ASCII"
            );
        }
    }

    #[test]
    fn default_ttl_meets_gate_floor() {
        // TTL must cover the ±120s window. Drift this constant down and the
        // gate breaks.
        // Const-drift tripwire: the assertion is intentionally over a constant.
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(DEFAULT_REPLAY_TTL_SECS >= 2 * TIMESTAMP_TOLERANCE_SECS);
        }
    }

    #[test]
    fn marker_covers_a_pod_with_a_slower_clock() {
        // Pod A first sees an event signed 120 s ahead of A's clock, at A
        // time t, and writes its marker until t + TTL. Pod B's clock runs
        // 9 s behind A's, so B still admits the event until B time
        // t + 240, which is A time t + 249. The marker must still be live
        // then, or B would accept a replay.
        let t = 1_000_000u64;
        let marker_expires_a = t + DEFAULT_REPLAY_TTL_SECS;
        let last_admissible_on_b_in_a_time =
            t + 2 * TIMESTAMP_TOLERANCE_SECS + POD_CLOCK_DIFFERENCE_BUDGET_SECS;
        assert!(last_admissible_on_b_in_a_time < marker_expires_a);
    }

    #[test]
    fn ttl_floor_below_ceiling() {
        // Sanity: any caller's clamped TTL must end up in [DEFAULT, MAX].
        // If these ever cross, the impl can't satisfy both bounds and the
        // contract is broken.
        // Const-drift tripwire: the assertion is intentionally over a constant.
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(DEFAULT_REPLAY_TTL_SECS < MAX_REPLAY_TTL_SECS);
        }
    }

    #[test]
    fn max_ttl_fits_in_redis_signed_ex() {
        // Redis `EX` is parsed as i64. `MAX_REPLAY_TTL_SECS` must fit so the
        // clamp itself can't push us into a Redis-side parse failure.
        assert!(MAX_REPLAY_TTL_SECS <= i64::MAX as u64);
    }

    #[tokio::test]
    async fn always_fresh_returns_true() {
        let guard = AlwaysFreshReplayGuard;
        let ctx = fixture_ctx("relay-a.example");
        let eid = fixture_event_id();
        assert!(guard
            .try_mark(&ctx, &eid, DEFAULT_REPLAY_TTL_SECS)
            .await
            .unwrap());
    }
}
