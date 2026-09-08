//! In-memory NIP-FI deny set.
//!
//! Holds `(iss, pubkey) → until` entries.  No persistence — a relay restart
//! forgets active entries (Option B, as decided).  The issuer re-push path is
//! documented as the mitigation but is not implemented here.
//!
//! ## Invariants
//!
//! * **Merge rule**: inserting a new `until` for an existing key retains
//!   `max(existing_until, new_until)` — an accepted disconnect MUST NOT shorten
//!   an active deny. [FI-TRACE-DENY-SET]
//! * **Past-`until` commands**: close sessions but MUST NOT create or shorten
//!   entries.  Concretely, when `new_until < now` the merge still applies the
//!   `max` rule, which preserves any active entry and lets a "no active entry"
//!   case insert with an already-expired value (immediately inactive). [FI-TRACE-DENY-SET]
//! * **Per-issuer capacity cap**: each issuer has a hard ceiling on live entries.
//!   A capacity failure returns `Err(DenySetFull)` without inserting anything —
//!   the spec requires `503` here and neither the jti nor the deny entry is
//!   recorded. [FI-TRACE-DENY-SET]
//! * **Cross-issuer isolation**: capacity of issuer A MUST NOT affect issuer B.
//! * **Cross-pod capacity miss**: `merge_cross_pod_deny` returning `CapacityExceeded`
//!   preserves the existing shard contents; the caller closes the delivered
//!   target's sessions and reports/metrics the outcome.  No issuer-wide denial
//!   is synthesized.  Async propagation loss with issuer re-push is the
//!   sanctioned recovery. [NIP-FI.md:306-336]
//! * **jti reservation** and **deny-entry insertion** are performed atomically
//!   in one lock scope (both or neither). [VerifyCommandJwt step 7]
//! * **Issuer-global scope**: the deny applies across all communities served
//!   under that issuer. [FI-TRACE-DENY-SET]
//! * **Self-eviction**: expired entries are pruned lazily on each mutation and
//!   on read (deny check), so the map does not grow without bound.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use nostr::PublicKey;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ── Error type ────────────────────────────────────────────────────────────────

/// Returned when the per-issuer deny-set capacity is exhausted.
///
/// The caller MUST respond `503` and MUST NOT record the jti; the same signed
/// command remains replayable (the command identity was not consumed).
/// [VerifyCommandJwt step 7]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("deny set full for issuer")]
pub struct DenySetFull;

// ── Per-issuer shard ──────────────────────────────────────────────────────────

/// One issuer's worth of deny entries and jti deduplication state.
///
/// The shard mutex is acquired once per `AtomicReserveJtiAndDenyEntry` call
/// so both mutations happen under the same lock (both-or-neither atomicity).
struct IssuerShard {
    /// Active deny entries: hex-encoded pubkey → until.
    entries: HashMap<String, DateTime<Utc>>,
    /// Reserved jtis: jti string → effective_expiry.  Expired jtis are evicted
    /// lazily on each write so the map never grows to replay-corpus size.
    jtis: HashMap<String, DateTime<Utc>>,
    /// Maximum number of live deny entries for this issuer.
    capacity: usize,
    /// Maximum number of live jti reservations for this issuer.
    /// Bounded separately so an issuer cannot exhaust memory by replaying
    /// distinct jtis faster than they expire, even for already-denied keys.
    /// Set to `capacity * 2` at construction for O(capacity) memory with
    /// headroom for in-flight update commands on already-denied keys.
    max_jti_count: usize,
}

impl IssuerShard {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            jtis: HashMap::new(),
            capacity,
            // JTI resource bound: allow up to 2× capacity JTI reservations.
            // This gives one active command per entry slot plus headroom for
            // one in-flight update command per already-denied key without
            // blocking normal operation.  Still O(capacity) memory.
            max_jti_count: capacity.saturating_mul(2).max(1),
        }
    }

    /// Evict expired entries and jtis.  Called inside the lock on every write.
    fn evict_expired(&mut self, now: DateTime<Utc>) {
        self.entries.retain(|_, until| *until > now);
        self.jtis.retain(|_, exp| *exp > now);
    }

    /// True if `(iss, pubkey_hex)` has an active deny entry (`now < until`).
    fn is_denied(&self, pubkey_hex: &str, now: DateTime<Utc>) -> bool {
        self.entries
            .get(pubkey_hex)
            .map(|until| now < *until)
            .unwrap_or(false)
    }

    /// Attempt the atomic jti-reservation + deny-entry insertion.
    ///
    /// **Atomicity**: both HashMap inserts are precomputed before any write.
    /// Eviction is done first (pure mutation of existing map, always safe),
    /// then all fallible pre-conditions are checked, then both inserts happen
    /// under the same lock scope.  An unwind before the inserts leaves the
    /// shard unchanged; an unwind mid-insert is not possible because HashMap
    /// insert is infallible after capacity reservation.
    fn atomic_reserve_and_insert(
        &mut self,
        jti: &str,
        jti_effective_expiry: DateTime<Utc>,
        pubkey_hex: &str,
        until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), ReserveError> {
        self.evict_expired(now);

        // Replay check: jti already in set → AuthorizationDenied.
        if self.jtis.contains_key(jti) {
            return Err(ReserveError::JtiAlreadyReserved);
        }

        // JTI resource bound: cap live jti reservations at max_jti_count so an
        // issuer cannot exhaust memory by sending distinct jtis for already-denied
        // keys faster than they expire.  Uses CapacityExceeded so the caller
        // responds 503 and the command remains replayable (jti not burned).
        if self.jtis.len() >= self.max_jti_count {
            return Err(ReserveError::CapacityExceeded);
        }

        // Deny-entry capacity check: only count as new if no active entry exists.
        // The merge rule never increases live entry count.
        let is_update = self
            .entries
            .get(pubkey_hex)
            .map(|existing| now < *existing)
            .unwrap_or(false);
        if !is_update && self.entries.len() >= self.capacity {
            return Err(ReserveError::CapacityExceeded);
        }

        // Prebuild both values before writing anything.
        let jti_key = jti.to_owned();
        let entry_key = pubkey_hex.to_owned();
        let effective_until = match self.entries.get(pubkey_hex) {
            Some(&existing) => existing.max(until),
            None => until,
        };

        // Both mutations are infallible HashMap inserts; executed together
        // so no intermediate observable state exists.
        self.jtis.insert(jti_key, jti_effective_expiry);
        self.entries.insert(entry_key, effective_until);

        Ok(())
    }

    /// Merge a remote deny entry without consuming a jti.
    ///
    /// Used for cross-pod propagation where replay idempotency is achieved by
    /// the max(until) merge rule alone — no jti tracking needed.
    /// Returns `Err(CapacityExceeded)` if the entry is new and the shard is full.
    fn remote_merge(
        &mut self,
        pubkey_hex: &str,
        until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), ReserveError> {
        self.evict_expired(now);

        // Capacity check: only count as new if there is no active entry.
        let is_update = self
            .entries
            .get(pubkey_hex)
            .map(|existing| now < *existing)
            .unwrap_or(false);
        if !is_update && self.entries.len() >= self.capacity {
            return Err(ReserveError::CapacityExceeded);
        }

        // max(existing_until, until) merge.
        let effective_until = match self.entries.get(pubkey_hex) {
            Some(&existing) => existing.max(until),
            None => until,
        };
        self.entries.insert(pubkey_hex.to_owned(), effective_until);
        Ok(())
    }
}

/// Reasons an atomic reserve can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReserveError {
    /// The jti was already reserved — replay attempt.
    JtiAlreadyReserved,
    /// Per-issuer capacity ceiling reached.
    CapacityExceeded,
}

/// Outcome of a cross-pod deny merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossPodMergeResult {
    /// Entry was inserted or updated (max-merge applied).
    Merged,
    /// Issuer is not locally configured; message rejected.
    UnknownIssuer,
    /// Per-issuer capacity ceiling reached; the missed entry was not recorded.
    /// The caller should close any sessions matching the delivered target despite
    /// the capacity miss, and report/metric the outcome.
    CapacityExceeded,
    /// Shard mutex is poisoned; issuer is fail-closed.
    ShardPoisoned,
}

// ── Public map ────────────────────────────────────────────────────────────────

/// Relay-wide in-memory NIP-FI deny set.
///
/// One `Arc<NipFiDenyMap>` is held in `AppState`; the HTTP disconnect endpoint
/// and the WS admission check share it.
///
/// The deny-check interface is intentionally transport-agnostic — S5 (HTTP
/// enforcement) calls `is_denied` from HTTP admission without any WS coupling.
#[derive(Clone)]
pub struct NipFiDenyMap {
    /// Per-issuer shards.  Each shard owns its own Mutex so cross-issuer
    /// capacity exhaustion is impossible to cause cross-issuer denial.
    shards: Arc<DashMap<String, Mutex<IssuerShard>>>,
    /// Default per-issuer capacity, used when no issuer-specific override exists.
    default_capacity: usize,
}

/// A per-issuer capacity override supplied at construction time.
#[derive(Debug, Clone)]
pub struct IssuerCapacity {
    /// The exact issuer URI this capacity applies to.
    pub issuer: String,
    /// Maximum number of live deny entries for this issuer.
    pub capacity: usize,
}

impl NipFiDenyMap {
    /// Construct a new deny map.
    ///
    /// `default_capacity` is the per-issuer entry ceiling used for any issuer
    /// not listed in `issuer_capacities`.  Must be > 0.
    ///
    /// A zero capacity would make every command a 503; callers must validate
    /// before construction.
    pub fn new(default_capacity: usize, issuer_capacities: Vec<IssuerCapacity>) -> Self {
        let shards: DashMap<String, Mutex<IssuerShard>> = DashMap::new();
        for ic in issuer_capacities {
            shards.insert(ic.issuer, Mutex::new(IssuerShard::new(ic.capacity)));
        }
        Self {
            shards: Arc::new(shards),
            default_capacity,
        }
    }

    /// Returns `true` when `(iss, pubkey)` has an active deny entry at `now`.
    ///
    /// Used by S4 (WS admission step 6) and S5 (HTTP admission step 5).
    /// [FI-TRACE-DENY-SET]
    ///
    /// Fails **closed**: a poisoned shard lock returns `true` (deny) so that a
    /// damaged shard cannot silently admit a denied pubkey.
    pub fn is_denied(&self, issuer: &str, pubkey: &PublicKey, now: DateTime<Utc>) -> bool {
        let pubkey_hex = pubkey.to_hex();
        match self.shards.get(issuer) {
            Some(shard) => shard
                .lock()
                .map(|guard| guard.is_denied(&pubkey_hex, now))
                .unwrap_or(true), // poisoned shard → fail closed (deny)
            None => false,
        }
    }

    /// Atomically reserve `(iss, jti)` and insert/merge the deny entry.
    ///
    /// Both mutations happen under the same per-issuer lock (both-or-neither).
    ///
    /// * `Ok(())` — success.
    /// * `Err(ReserveError::JtiAlreadyReserved)` — replay; map is unchanged,
    ///   caller responds `AuthorizationDenied`.
    /// * `Err(ReserveError::CapacityExceeded)` — full; map is unchanged,
    ///   caller responds `503 deny set full`.
    ///
    /// [VerifyCommandJwt step 7]
    pub(crate) fn atomic_reserve_and_insert(
        &self,
        issuer: &str,
        jti: &str,
        jti_effective_expiry: DateTime<Utc>,
        pubkey: &PublicKey,
        until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), ReserveError> {
        let pubkey_hex = pubkey.to_hex();
        let shard = self
            .shards
            .entry(issuer.to_owned())
            .or_insert_with(|| Mutex::new(IssuerShard::new(self.default_capacity)));
        shard
            .lock()
            .map_err(|_| ReserveError::CapacityExceeded) // poisoned = fail closed
            .and_then(|mut guard| {
                guard.atomic_reserve_and_insert(jti, jti_effective_expiry, &pubkey_hex, until, now)
            })
    }

    /// Merge a cross-pod deny entry (e.g. from Redis propagation).
    ///
    /// Idempotent: repeated delivery of the same `(issuer, pubkey, until)` is
    /// a no-op due to the `max(until)` merge rule.  No synthetic jti is
    /// allocated — replay idempotency is structural, not tracked.
    ///
    /// Only merges into **locally-configured** issuer shards.  An unknown
    /// issuer returns [`CrossPodMergeResult::UnknownIssuer`] so the consumer
    /// can reject without allocating state.
    ///
    /// On capacity exhaustion, returns [`CrossPodMergeResult::CapacityExceeded`]
    /// without altering the shard.  The caller is responsible for closing the
    /// delivered target's sessions and reporting/metricing the outcome; no
    /// issuer-wide denial is synthesized.  [NIP-FI.md:306-336]
    pub fn merge_cross_pod_deny(
        &self,
        issuer: &str,
        pubkey: &PublicKey,
        until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> CrossPodMergeResult {
        let pubkey_hex = pubkey.to_hex();
        // Only operate on pre-configured shards — never allocate for unknown issuers.
        match self.shards.get(issuer) {
            None => CrossPodMergeResult::UnknownIssuer,
            Some(shard) => match shard.lock() {
                Err(_) => {
                    // Shard is poisoned — we cannot obtain the lock.  A poisoned
                    // mutex already causes `is_denied` to return `true` (the
                    // `unwrap_or(true)` path), so the issuer is implicitly
                    // fail-closed without any explicit write.
                    CrossPodMergeResult::ShardPoisoned
                }
                Ok(mut guard) => match guard.remote_merge(&pubkey_hex, until, now) {
                    Ok(()) => CrossPodMergeResult::Merged,
                    Err(ReserveError::CapacityExceeded) => {
                        // Cannot record the deny entry — return the outcome so
                        // the caller can close the delivered target's sessions
                        // and report/metric the capacity miss.  No issuer-wide
                        // denial is synthesized; active entries are preserved.
                        // [NIP-FI.md:306-336]
                        CrossPodMergeResult::CapacityExceeded
                    }
                    Err(ReserveError::JtiAlreadyReserved) => {
                        // remote_merge never touches jtis; this arm is unreachable.
                        unreachable!("remote_merge does not use jti tracking")
                    }
                },
            },
        }
    }

    /// Close all sessions whose proven pubkey is `pubkey` for any issuer and
    /// any community.  This is the issuer-global close scan.
    ///
    /// Returns the pubkey_hex for downstream use.
    pub fn pubkey_hex(pubkey: &PublicKey) -> String {
        pubkey.to_hex()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use nostr::Keys;

    fn key() -> PublicKey {
        Keys::generate().public_key()
    }

    fn map() -> NipFiDenyMap {
        NipFiDenyMap::new(100, vec![])
    }

    fn iss() -> &'static str {
        "https://issuer.example.com"
    }

    // ── FI-TRACE-DENY-SET: basic admit/deny ──────────────────────────────────

    #[test]
    fn not_denied_when_no_entry() {
        let m = map();
        assert!(
            !m.is_denied(iss(), &key(), Utc::now()),
            "no entry → admitted"
        );
    }

    #[test]
    fn denied_when_active_entry() {
        let m = map();
        let k = key();
        let until = Utc::now() + Duration::seconds(300);
        m.atomic_reserve_and_insert(iss(), "jti-1", until, &k, until, Utc::now())
            .expect("first insert");
        assert!(m.is_denied(iss(), &k, Utc::now()), "active entry → denied");
    }

    #[test]
    fn admitted_after_until_expires() {
        let m = map();
        let k = key();
        let until = Utc::now() - Duration::seconds(1); // already expired
        m.atomic_reserve_and_insert(iss(), "jti-exp", until, &k, until, Utc::now())
            .expect("insert with past-until");
        // is_denied with `now` past `until` → not denied
        assert!(
            !m.is_denied(iss(), &k, Utc::now()),
            "expired entry → admitted"
        );
    }

    // ── FI-TRACE-DENY-SET: merge rule ────────────────────────────────────────

    #[test]
    fn merge_rule_longer_command_wins() {
        let m = map();
        let k = key();
        let now = Utc::now();
        let longer = now + Duration::seconds(600);
        let shorter = now + Duration::seconds(300);

        // Insert longer first.
        m.atomic_reserve_and_insert(iss(), "jti-A", longer, &k, longer, now)
            .expect("insert longer");
        // Insert shorter — must not shorten.
        m.atomic_reserve_and_insert(iss(), "jti-B", shorter, &k, shorter, now)
            .expect("insert shorter");

        // Check just before shorter would expire (still in longer window).
        let check_time = now + Duration::seconds(400);
        assert!(
            m.is_denied(iss(), &k, check_time),
            "merge rule: longer deny survives shorter command"
        );
    }

    #[test]
    fn merge_rule_longer_command_second_wins() {
        let m = map();
        let k = key();
        let now = Utc::now();
        let shorter = now + Duration::seconds(300);
        let longer = now + Duration::seconds(600);

        // Insert shorter first, then longer.
        m.atomic_reserve_and_insert(iss(), "jti-A", shorter, &k, shorter, now)
            .expect("insert shorter");
        m.atomic_reserve_and_insert(iss(), "jti-B", longer, &k, longer, now)
            .expect("insert longer");

        let check_time = now + Duration::seconds(400);
        assert!(
            m.is_denied(iss(), &k, check_time),
            "delivery order does not matter — longer wins regardless"
        );
    }

    #[test]
    fn past_until_command_over_active_entry_leaves_active_unchanged() {
        let m = map();
        let k = key();
        let now = Utc::now();
        let active_until = now + Duration::seconds(600);
        let past_until = now - Duration::seconds(60);

        // Active entry first.
        m.atomic_reserve_and_insert(iss(), "jti-A", active_until, &k, active_until, now)
            .expect("insert active");

        // Past-until command: max(active_until, past_until) = active_until.
        m.atomic_reserve_and_insert(iss(), "jti-B", past_until, &k, past_until, now)
            .expect("past-until insert");

        // Active entry unchanged.
        let check_time = now + Duration::seconds(400);
        assert!(
            m.is_denied(iss(), &k, check_time),
            "past-until command must not shorten active deny"
        );
    }

    #[test]
    fn past_until_command_absent_entry_inserts_expired() {
        let m = map();
        let k = key();
        let now = Utc::now();
        let past_until = now - Duration::seconds(60);

        // Past-until, no existing entry → insert with expired value → immediately inactive.
        m.atomic_reserve_and_insert(iss(), "jti-A", past_until, &k, past_until, now)
            .expect("past-until on absent entry");

        // Not denied (entry is immediately expired).
        assert!(
            !m.is_denied(iss(), &k, now),
            "past-until with no prior entry creates no future denial"
        );
    }

    // ── Replay prevention ────────────────────────────────────────────────────

    #[test]
    fn jti_replay_is_rejected() {
        let m = map();
        let k = key();
        let until = Utc::now() + Duration::seconds(300);

        m.atomic_reserve_and_insert(iss(), "jti-same", until, &k, until, Utc::now())
            .expect("first use");
        let result = m.atomic_reserve_and_insert(iss(), "jti-same", until, &k, until, Utc::now());
        assert_eq!(
            result,
            Err(ReserveError::JtiAlreadyReserved),
            "replayed jti must be rejected"
        );
    }

    // ── Capacity ─────────────────────────────────────────────────────────────

    #[test]
    fn jti_resource_bound_limits_replay_state_for_already_denied_keys() {
        // max_jti_count = capacity * 2 = 4 (for capacity=2).
        // Fill all 4 JTI slots across 2 keys, then verify a fifth jti is rejected.
        // This proves the bound is enforced even though entry capacity is not
        // exhausted (only 2 entries for 2 keys, entry capacity is 2 — no new
        // entries would be inserted).
        //
        // Mutation anchor: removing the JTI resource-bound check would let
        // the jtis map grow without limit even though no new deny entries are
        // added (because the update path bypasses the entry-capacity check).
        let m = NipFiDenyMap::new(
            2,
            vec![IssuerCapacity {
                issuer: iss().to_owned(),
                capacity: 2,
            }],
        );
        let now = Utc::now();
        let until = now + Duration::seconds(300);
        let k1 = key();
        let k2 = key();

        // 4 commands across 2 keys: fills all 4 JTI slots (2 per key, 2*2=4).
        m.atomic_reserve_and_insert(iss(), "jti-k1-a", until, &k1, until, now)
            .expect("k1 first command");
        m.atomic_reserve_and_insert(
            iss(),
            "jti-k1-b",
            until + Duration::seconds(1),
            &k1,
            until + Duration::seconds(1),
            now,
        )
        .expect("k1 second command (update, within jti bound)");
        m.atomic_reserve_and_insert(iss(), "jti-k2-a", until, &k2, until, now)
            .expect("k2 first command");
        m.atomic_reserve_and_insert(
            iss(),
            "jti-k2-b",
            until + Duration::seconds(1),
            &k2,
            until + Duration::seconds(1),
            now,
        )
        .expect("k2 second command (update, within jti bound)");

        // 5th JTI: max_jti_count=4 exhausted → CapacityExceeded.
        let result = m.atomic_reserve_and_insert(
            iss(),
            "jti-k1-c",
            until + Duration::seconds(2),
            &k1,
            until + Duration::seconds(2),
            now,
        );
        assert_eq!(
            result,
            Err(ReserveError::CapacityExceeded),
            "jti resource bound must reject the fifth jti (max_jti_count=4 exhausted)"
        );
    }

    #[test]
    fn capacity_exceeded_returns_error_without_inserting() {
        // Capacity = 2, three distinct pubkeys.
        let m = NipFiDenyMap::new(2, vec![]);
        let now = Utc::now();
        let until = now + Duration::seconds(300);

        let k1 = key();
        let k2 = key();
        let k3 = key();

        m.atomic_reserve_and_insert(iss(), "jti-1", until, &k1, until, now)
            .expect("k1");
        m.atomic_reserve_and_insert(iss(), "jti-2", until, &k2, until, now)
            .expect("k2");
        let result = m.atomic_reserve_and_insert(iss(), "jti-3", until, &k3, until, now);
        assert_eq!(
            result,
            Err(ReserveError::CapacityExceeded),
            "third distinct key must be rejected when cap=2"
        );
        // k3 is NOT denied (entry was not inserted).
        assert!(!m.is_denied(iss(), &k3, now), "k3 must not be denied");
    }

    #[test]
    fn update_to_existing_key_does_not_count_against_entry_capacity() {
        // capacity=2: two entry slots, but only one is used.
        // An update to the existing key uses the second JTI slot but does NOT
        // add a second entry — verifies the entry-capacity check allows updates.
        // (JTI capacity is a separate bound: capacity=2 gives max_jti_count=2,
        // so the second JTI fits without hitting the JTI resource bound either.)
        let m = NipFiDenyMap::new(2, vec![]);
        let now = Utc::now();
        let k = key();
        let until_a = now + Duration::seconds(300);
        let until_b = now + Duration::seconds(600);

        m.atomic_reserve_and_insert(iss(), "jti-a", until_a, &k, until_a, now)
            .expect("first insert");
        // Same key, longer until — entry count stays at 1 (update), jti count goes to 2.
        m.atomic_reserve_and_insert(iss(), "jti-b", until_b, &k, until_b, now)
            .expect("update same key must succeed: only one entry used, entry-capacity is 2");

        assert!(
            m.is_denied(iss(), &k, now + Duration::seconds(400)),
            "updated entry is active"
        );
    }

    #[test]
    fn cross_issuer_capacity_is_independent() {
        let iss_a = "https://a.example.com";
        let iss_b = "https://b.example.com";
        // issuer A has capacity 1
        let m = NipFiDenyMap::new(
            100,
            vec![IssuerCapacity {
                issuer: iss_a.to_owned(),
                capacity: 1,
            }],
        );

        let now = Utc::now();
        let until = now + Duration::seconds(300);
        let k1 = key();
        let k2 = key();
        let k3 = key();

        // Fill issuer A.
        m.atomic_reserve_and_insert(iss_a, "jti-a1", until, &k1, until, now)
            .expect("iss_a k1");
        // Issuer B is at default capacity (100) → must accept.
        m.atomic_reserve_and_insert(iss_b, "jti-b1", until, &k2, until, now)
            .expect("iss_b k2 must succeed independent of iss_a capacity");
        // Issuer A is at capacity 1 → must reject.
        let result = m.atomic_reserve_and_insert(iss_a, "jti-a2", until, &k3, until, now);
        assert_eq!(
            result,
            Err(ReserveError::CapacityExceeded),
            "iss_a capacity exhaustion must not affect iss_b, and vice versa"
        );
    }

    // ── Poison-path: is_denied must fail closed ───────────────────────────────

    #[test]
    fn poisoned_shard_is_denied_fails_closed() {
        let iss = "https://poison.example.com";
        // Construct the map with a pre-registered shard for this issuer.
        let m = std::sync::Arc::new(NipFiDenyMap::new(
            10,
            vec![IssuerCapacity {
                issuer: iss.to_owned(),
                capacity: 10,
            }],
        ));
        let m_clone = std::sync::Arc::clone(&m);
        let k = key();

        // Poison the real IssuerShard by spawning a thread that acquires the
        // shard Mutex (which wraps a real IssuerShard) and then panics.
        // A thread panic while holding a Mutex guard poisons the mutex.
        let _ = std::thread::spawn(move || {
            let shard_ref = m_clone.shards.get(iss).expect("shard must exist");
            let _guard = shard_ref.lock().expect("lock acquired");
            panic!("intentional poison");
        })
        .join(); // Err(_) expected — that's the proof the thread panicked.

        // The shard is now poisoned.  is_denied must return true (fail closed).
        // Mutation anchor: reverting unwrap_or(true) → unwrap_or(false) makes
        // this assertion fail — that is the defect Thufir identified in pass 1.
        assert!(
            m.is_denied(iss, &k, Utc::now()),
            "poisoned shard must return true from is_denied (fail closed)"
        );

        // Confirm the normal path still works on a clean map.
        let clean = std::sync::Arc::new(NipFiDenyMap::new(
            10,
            vec![IssuerCapacity {
                issuer: iss.to_owned(),
                capacity: 10,
            }],
        ));
        let k2 = key();
        let until2 = Utc::now() + Duration::seconds(300);
        clean
            .atomic_reserve_and_insert(iss, "jti-clean", until2, &k2, until2, Utc::now())
            .expect("insert on clean map");
        assert!(
            clean.is_denied(iss, &k2, Utc::now()),
            "active entry on clean map must return true"
        );
    }

    // ── remote_merge: idempotent cross-pod semantics ─────────────────────────

    #[test]
    fn remote_merge_shorter_after_longer_does_not_shorten() {
        // Map with iss() pre-registered so remote_merge can operate on it.
        let m = NipFiDenyMap::new(
            100,
            vec![IssuerCapacity {
                issuer: iss().to_owned(),
                capacity: 100,
            }],
        );
        let k = key();
        let now = Utc::now();
        let longer = now + Duration::seconds(600);
        let shorter = now + Duration::seconds(300);

        // First merge: longer.
        assert_eq!(
            m.merge_cross_pod_deny(iss(), &k, longer, now),
            CrossPodMergeResult::Merged
        );
        // Second merge: shorter — must not shorten.
        assert_eq!(
            m.merge_cross_pod_deny(iss(), &k, shorter, now),
            CrossPodMergeResult::Merged
        );
        // At 400s: still denied (longer wins).
        assert!(
            m.is_denied(iss(), &k, now + Duration::seconds(400)),
            "shorter-after-longer remote merge must not shorten the deny"
        );
    }

    #[test]
    fn remote_merge_replay_is_idempotent() {
        // Map with iss() pre-registered so remote_merge can operate on it.
        let m = NipFiDenyMap::new(
            100,
            vec![IssuerCapacity {
                issuer: iss().to_owned(),
                capacity: 100,
            }],
        );
        let k = key();
        let now = Utc::now();
        let until = now + Duration::seconds(300);

        // Deliver twice.
        assert_eq!(
            m.merge_cross_pod_deny(iss(), &k, until, now),
            CrossPodMergeResult::Merged
        );
        assert_eq!(
            m.merge_cross_pod_deny(iss(), &k, until, now),
            CrossPodMergeResult::Merged
        );
        // Still denied at 200s (no spurious second-insert count growth).
        assert!(
            m.is_denied(iss(), &k, now + Duration::seconds(200)),
            "replay must be idempotent"
        );
    }

    #[test]
    fn remote_merge_unknown_issuer_rejected() {
        let m = map(); // default issuer is "https://issuer.example.com", not "unknown"
        let k = key();
        let until = Utc::now() + Duration::seconds(300);
        assert_eq!(
            m.merge_cross_pod_deny("https://unknown.example.com", &k, until, Utc::now()),
            CrossPodMergeResult::UnknownIssuer,
            "unknown issuer must be rejected without allocating state"
        );
        // No shard was created for the unknown issuer.
        assert!(
            m.shards.get("https://unknown.example.com").is_none(),
            "no shard must be allocated for unknown issuer"
        );
    }

    // ── remote_merge: capacity oracle (two-pod divergent model) ──────────────
    //
    // Verifies that ordinary remote capacity exhaustion leaves active entries
    // intact, returns the capacity outcome, and does NOT synthesize issuer-wide
    // denial or unrelated-key denial.  Uses two capacity-1 maps modeling
    // divergent pods with the same issuer.
    //
    // Mandatory reds:
    //  (a) guard/evict the active entry on capacity → original k1 entry gone;
    //      entry-retention assertion fails
    //  (b) synthesize issuer-wide denial (set blocked) → missed-target and
    //      unrelated-key is_denied assertions fail
    //  (c) admit at exact equality (use <= instead of <) → equality assertion fails

    #[test]
    fn remote_merge_capacity_exceeded_preserves_active_entry_and_does_not_deny_missed_or_unrelated()
    {
        // Two capacity-1 maps modeling divergent pods (pod A, pod B).
        // Pod A locally contains target k_a; pod B locally contains target k_b.
        // Both have the same finite TTL.
        let now = Utc::now();
        let until = now + Duration::seconds(300);
        let k_a = key();
        let k_b = key();
        let k_unrelated = key();

        let make_map = |local_key: &PublicKey| {
            let m = NipFiDenyMap::new(
                1,
                vec![IssuerCapacity {
                    issuer: iss().to_owned(),
                    capacity: 1,
                }],
            );
            // Pre-fill with the local target.
            m.atomic_reserve_and_insert(iss(), "jti-local", until, local_key, until, now)
                .expect("local pre-fill must succeed");
            m
        };

        // Pod A map: has k_a, receives k_b cross-pod.
        let map_a = make_map(&k_a);
        let result_a = map_a.merge_cross_pod_deny(iss(), &k_b, until, now);
        assert_eq!(
            result_a,
            CrossPodMergeResult::CapacityExceeded,
            "cross-delivery of k_b to pod A (capacity=1, already holds k_a) must return CapacityExceeded"
        );
        // Pod A still has its original k_a entry — capacity miss must not evict.
        assert!(
            map_a.is_denied(iss(), &k_a, now),
            "pod A must still deny k_a after capacity miss"
        );
        // Pod A must NOT deny the missed k_b via issuer-wide block.
        assert!(
            !map_a.is_denied(iss(), &k_b, now),
            "pod A must NOT deny missed target k_b — no issuer-wide denial on capacity miss"
        );
        // Pod A must NOT deny an unrelated key.
        assert!(
            !map_a.is_denied(iss(), &k_unrelated, now),
            "pod A must NOT deny unrelated key after capacity miss"
        );

        // Pod B map: has k_b, receives k_a cross-pod.
        let map_b = make_map(&k_b);
        let result_b = map_b.merge_cross_pod_deny(iss(), &k_a, until, now);
        assert_eq!(
            result_b,
            CrossPodMergeResult::CapacityExceeded,
            "cross-delivery of k_a to pod B (capacity=1, already holds k_b) must return CapacityExceeded"
        );
        assert!(
            map_b.is_denied(iss(), &k_b, now),
            "pod B must still deny k_b after capacity miss"
        );
        assert!(
            !map_b.is_denied(iss(), &k_a, now),
            "pod B must NOT deny missed target k_a"
        );

        // At exact equality with the TTL both entries are admitted (now < until fails).
        assert!(
            !map_a.is_denied(iss(), &k_a, until),
            "k_a must be admitted at exact equality with TTL"
        );
        assert!(
            !map_b.is_denied(iss(), &k_b, until),
            "k_b must be admitted at exact equality with TTL"
        );

        // Delayed already-expired remote entry: may return capacity outcome but
        // must not alter the live entry or deny the expired target / unrelated key.
        let expired_until = now - Duration::seconds(1);
        let map_c = make_map(&k_a); // pre-filled with k_a active
        let result_c = map_c.merge_cross_pod_deny(iss(), &k_b, expired_until, now);
        // Expired remote entry is treated as a new (already-expired) entry; since
        // the shard is at capacity the remote_merge returns CapacityExceeded.
        // The live k_a entry must remain; k_b and k_unrelated must not be denied.
        assert!(
            map_c.is_denied(iss(), &k_a, now),
            "live k_a must survive a capacity-miss with expired remote target"
        );
        assert!(
            !map_c.is_denied(iss(), &k_b, now),
            "expired k_b must not be map-denied after capacity miss"
        );
        assert!(
            !map_c.is_denied(iss(), &k_unrelated, now),
            "unrelated key must not be denied after capacity miss with expired remote"
        );
        // Confirm the result is CapacityExceeded (expired entry still counts as
        // new against a full shard — it has no active existing entry).
        assert_eq!(
            result_c,
            CrossPodMergeResult::CapacityExceeded,
            "expired remote against full shard must return CapacityExceeded"
        );
    }

    #[test]
    fn remote_merge_poisoned_shard_returns_shard_poisoned() {
        let iss = "https://poison-remote.example.com";
        let m = std::sync::Arc::new(NipFiDenyMap::new(
            10,
            vec![IssuerCapacity {
                issuer: iss.to_owned(),
                capacity: 10,
            }],
        ));
        let m_clone = std::sync::Arc::clone(&m);
        let k = key();
        let until = Utc::now() + Duration::seconds(300);

        // Poison the shard.
        let _ = std::thread::spawn(move || {
            let shard_ref = m_clone.shards.get(iss).expect("shard must exist");
            let _guard = shard_ref.lock().expect("lock acquired");
            panic!("intentional poison for remote_merge test");
        })
        .join();

        assert_eq!(
            m.merge_cross_pod_deny(iss, &k, until, Utc::now()),
            CrossPodMergeResult::ShardPoisoned,
            "poisoned shard must return ShardPoisoned"
        );
    }

    // ── Blocker 2: JTI replay-bound boundary tests ────────────────────────────

    #[test]
    fn concurrent_same_issuer_reservations_do_not_exceed_jti_budget() {
        use std::sync::Barrier;

        // capacity=2 → JTI ceiling = 4.  One pre-denied target key so the
        // deny-entry capacity cannot become the limiting factor.
        let m = Arc::new(NipFiDenyMap::new(
            2,
            vec![IssuerCapacity {
                issuer: iss().to_owned(),
                capacity: 2,
            }],
        ));
        let now = Utc::now();
        let until = now + Duration::seconds(300);

        // Pre-deny one key so all subsequent threads are updates (bypass entry cap).
        let pre_key = key();
        m.atomic_reserve_and_insert(iss(), "jti-pre", until, &pre_key, until, now)
            .expect("pre-insert");

        let n_threads: usize = 8;
        let barrier = Arc::new(Barrier::new(n_threads));
        let mut handles = Vec::with_capacity(n_threads);

        for i in 0..n_threads {
            let m_clone = Arc::clone(&m);
            let b = Arc::clone(&barrier);
            let k_clone = pre_key;
            let jti = format!("concurrent-jti-{i}");
            handles.push(std::thread::spawn(move || {
                b.wait(); // release all threads simultaneously
                m_clone.atomic_reserve_and_insert(iss(), &jti, until, &k_clone, until, now)
            }));
        }

        let results: Vec<_> = handles
            .into_iter()
            .map(|h| h.join().expect("thread must not panic"))
            .collect();

        let successes = results.iter().filter(|r| r.is_ok()).count();
        let capacity_exceeded = results
            .iter()
            .filter(|r| matches!(r, Err(ReserveError::CapacityExceeded)))
            .count();

        // The pre-insert consumes 1 JTI slot; ceiling is 4; so exactly 3 of the
        // concurrent threads succeed (4 - 1 = 3 remaining slots).
        assert_eq!(
            successes,
            3,
            "exactly 3 concurrent successes allowed (4 JTI ceiling - 1 pre-used = 3), got {successes}"
        );
        assert_eq!(
            successes + capacity_exceeded,
            n_threads,
            "every result must be Ok or CapacityExceeded"
        );

        // Confirm the shard's JTI count is exactly at the ceiling.
        let live_jtis = m.shards.get(iss()).unwrap().lock().unwrap().jtis.len();
        assert_eq!(
            live_jtis, 4,
            "shard must have exactly 4 live JTIs (ceiling), found {live_jtis}"
        );
    }

    #[test]
    fn jti_budget_rejection_is_unapplied_and_exact_jti_retries_after_expiry() {
        use chrono::TimeZone;

        let cap = 2usize;
        // capacity=2 → JTI ceiling=4.  Use fixed synthetic timestamps.
        let m = NipFiDenyMap::new(
            cap,
            vec![IssuerCapacity {
                issuer: iss().to_owned(),
                capacity: cap,
            }],
        );
        let t0 = Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap();
        let k = key();
        let deny_deadline = t0 + Duration::seconds(10); // existing deny until t0+10s

        // Fill all 4 JTI slots:
        //  jti-0: expiry t0+2s (the short one — will expire first)
        //  jti-1..3: expiry t0+20s
        m.atomic_reserve_and_insert(
            iss(),
            "jti-0",
            t0 + Duration::seconds(2),
            &k,
            deny_deadline,
            t0,
        )
        .expect("slot 0");
        m.atomic_reserve_and_insert(
            iss(),
            "jti-1",
            t0 + Duration::seconds(20),
            &k,
            deny_deadline,
            t0,
        )
        .expect("slot 1");
        m.atomic_reserve_and_insert(
            iss(),
            "jti-2",
            t0 + Duration::seconds(20),
            &k,
            deny_deadline,
            t0,
        )
        .expect("slot 2");
        m.atomic_reserve_and_insert(
            iss(),
            "jti-3",
            t0 + Duration::seconds(20),
            &k,
            deny_deadline,
            t0,
        )
        .expect("slot 3");

        // Attempt candidate JTI X with a long deny deadline (t0+30s).
        // JTI budget is full (4/4) → must fail with CapacityExceeded.
        let x_jti_expiry = t0 + Duration::seconds(25);
        let x_deny_deadline = t0 + Duration::seconds(30);
        let result =
            m.atomic_reserve_and_insert(iss(), "jti-X", x_jti_expiry, &k, x_deny_deadline, t0);
        assert_eq!(
            result,
            Err(ReserveError::CapacityExceeded),
            "JTI budget full → CapacityExceeded"
        );

        // jti-X must NOT be recorded in the JTI set.
        {
            let shard = m.shards.get(iss()).unwrap();
            let guard = shard.lock().unwrap();
            assert!(
                !guard.jtis.contains_key("jti-X"),
                "jti-X must be absent from JTI set after budget rejection"
            );
            // The deny deadline must NOT have been extended (still t0+10s from
            // the last successful insert — the max-merge should not have applied X).
            let stored_until = *guard.entries.get(&k.to_hex()).unwrap();
            assert_eq!(
                stored_until, deny_deadline,
                "deny deadline must not be extended by a rejected JTI-budget command"
            );
        }

        // At t0+15s: the deny deadline (t0+10s) has passed → key should be admitted.
        let t_after_deny = t0 + Duration::seconds(15);
        assert!(
            !m.is_denied(iss(), &k, t_after_deny),
            "key must be admitted at t0+15s (deny deadline t0+10s expired)"
        );

        // Now retry jti-X at t0+3s: jti-0 (expiry t0+2s) has expired, freeing a slot.
        // jti-X itself is still valid (expiry t0+25s > t0+3s).
        // Retry uses the same deny_deadline=t0+30s.
        let t_retry = t0 + Duration::seconds(3);
        let result_retry =
            m.atomic_reserve_and_insert(iss(), "jti-X", x_jti_expiry, &k, x_deny_deadline, t_retry);
        assert!(
            result_retry.is_ok(),
            "retry of jti-X after jti-0 expired must succeed, got: {result_retry:?}"
        );

        // jti-X is now in the JTI set.
        {
            let shard = m.shards.get(iss()).unwrap();
            let guard = shard.lock().unwrap();
            assert!(
                guard.jtis.contains_key("jti-X"),
                "jti-X must be present after successful retry"
            );
        }

        // At t0+15s: deny deadline is now t0+30s (max of t0+10s and t0+30s) → denied.
        assert!(
            m.is_denied(iss(), &k, t_after_deny),
            "key must be denied at t0+15s after successful retry (deadline now t0+30s)"
        );
    }
}
