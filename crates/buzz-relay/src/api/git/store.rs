//! Object-store backend for git-on-object-storage.
//!
//! Implements the create-only, content-addressed write discipline (axiom A1)
//! and the CAS pointer swap (axiom A3) described in
//! `docs/git-on-object-storage.md`.
//!
//! This is a domain facade over [`buzz_object_store::ObjectStore`]: it owns
//! the git key layout, digest verification, and the conformance probe, and
//! knows nothing about which provider is underneath. Compare-and-swap tokens
//! are [`Revision`]s, not ETags — the S3 provider is the only place an ETag
//! exists.
//!
//! ## Classified vs. unknown outcomes
//!
//! The pointer-CAS path treats a failed precondition as a *semantic* result
//! ([`ConditionalWrite::Conflict`]), not an error. A request that never
//! produced a classified provider response is different again: its outcome is
//! *unknown*, so the probe drops it from the observer set rather than counting
//! it as a lost race. That distinction lives in
//! [`buzz_object_store::ObjectStoreError::is_ambiguous`]; see the S3 provider
//! module for how each provider failure is classified.
//!
//! ## Content addressing (A1)
//!
//! Pack and manifest keys are the SHA-256 of their bytes. Writes are
//! create-only so the same key is never overwritten. Readers verify object
//! bytes against the expected digest on `get_verified`; any mismatch is
//! *detectable*, not silent — that is what A1's "create-only + content-address"
//! discipline buys us, independent of bucket immutability features.

#![allow(dead_code)] // wired in by the push path in a follow-up commit

use std::sync::Arc;
use std::time::{Duration, Instant};

use buzz_object_store::{
    ConditionalWrite, ImmutableWrite, ObjectStore, ObjectStoreError, ProviderKind, Revision,
    S3AddressingStyle, S3ObjectStore, S3StoreConfig, WriteCondition,
};
use bytes::Bytes;
use sha2::{Digest, Sha256};

/// Errors that are *actually* errors — a lost CAS race is not one.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The requested key does not exist.
    #[error("object not found: {0}")]
    NotFound(String),
    /// The object is larger than the caller's bounded read budget.
    #[error("object too large: {key} is {size} bytes (max {max})")]
    ObjectTooLarge {
        /// Object key that was read.
        key: String,
        /// Object size reported by the backend.
        size: u64,
        /// Maximum bytes the caller allows for this read.
        max: u64,
    },
    /// A1 detectability fired: the bytes at `key` do not hash to `expected`.
    #[error("digest mismatch on {key}: expected {expected}, got {actual}")]
    DigestMismatch {
        /// Object key that was read.
        key: String,
        /// Digest the caller expected (the content-addressed key).
        expected: String,
        /// Digest computed from the returned bytes.
        actual: String,
    },
    /// Any other backend / transport error.
    #[error("object store error: {0}")]
    Backend(ObjectStoreError),
    /// Invalid storage configuration detected at client construction.
    #[error("git store config error: {0}")]
    Config(String),
    /// Conformance probe failed — backend does not satisfy A1/A2/A3.
    #[error(transparent)]
    Probe(ProbeFailure),
}

impl From<ObjectStoreError> for StoreError {
    /// Lift the provider errors that git treats as domain outcomes into their
    /// own variants, so call sites keep matching on `NotFound` /
    /// `ObjectTooLarge` / `DigestMismatch` directly.
    fn from(error: ObjectStoreError) -> Self {
        match error {
            ObjectStoreError::NotFound { key } => Self::NotFound(key),
            ObjectStoreError::ObjectTooLarge { key, size, max } => {
                Self::ObjectTooLarge { key, size, max }
            }
            ObjectStoreError::DigestMismatch {
                key,
                expected,
                actual,
            } => Self::DigestMismatch {
                key,
                expected,
                actual,
            },
            other => Self::Backend(other),
        }
    }
}

/// Build a backend error for a git-layer invariant the provider cannot express.
fn backend(operation: &'static str, message: String) -> StoreError {
    StoreError::Backend(ObjectStoreError::Provider { operation, message })
}

/// Configuration for `GitStore::run_conformance_probe`.
///
/// The probe is a deployment gate — run at startup, fail-closed. See
/// `docs/git-on-object-storage.md` §Conformance.
///
/// Defaults are per-provider ([`ProbeConfig::for_provider`]) because the two
/// profiles are proving the same axiom against backends with very different
/// admission costs: an S3-compatible store answers a wide race cheaply, while
/// Cloud Storage publishes a one-write-per-second ceiling per object name, so a
/// wide, tightly-spaced race there measures the rate limiter rather than the
/// conditional-write semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeConfig {
    /// How many tasks race per round. Must be ≥ 2.
    pub race_width: usize,
    /// How many rounds to run each race phase.
    pub race_rounds: usize,
    /// How many times a round that proved nothing may be re-run before the
    /// probe gives up.
    ///
    /// A round proves nothing when no racer's outcome distinguishes a
    /// conforming backend from a broken one — every racer throttled, or too few
    /// racers were classified to have witnessed a race at all. Retrying is not
    /// leniency: an unproven round is neither pass nor fail, and exhausting the
    /// budget without ever proving a round fails the probe.
    pub unproven_round_retries: usize,
    /// Minimum wall-clock spacing between mutations of the same key.
    ///
    /// Zero for backends with no documented per-object write ceiling. Cloud
    /// Storage documents one write per second per object name, so its profile
    /// spaces same-key rounds beyond that interval — the probe proves the
    /// store's conditional-write semantics, and deliberately violating the
    /// published rate limit would only prove that the rate limiter works.
    pub same_key_spacing: Duration,
}

impl ProbeConfig {
    /// Defaults for `provider`.
    pub fn for_provider(provider: ProviderKind) -> Self {
        match provider {
            ProviderKind::S3 => Self {
                race_width: 32,
                race_rounds: 3,
                unproven_round_retries: 3,
                same_key_spacing: Duration::ZERO,
            },
            ProviderKind::Gcs => Self {
                race_width: 3,
                race_rounds: 2,
                unproven_round_retries: 3,
                // Just past Cloud Storage's documented one-write-per-second
                // per-object ceiling.
                same_key_spacing: Duration::from_millis(1_100),
            },
        }
    }
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self::for_provider(ProviderKind::S3)
    }
}

/// Returned on a successful probe run. Kept intentionally thin — failure
/// detail lives in `ProbeFailure` (the error variant).
#[derive(Debug, Clone)]
pub struct ProbeReport {
    /// Which provider profile ran.
    pub profile: ProviderKind,
    /// Concurrency used.
    pub race_width: usize,
    /// Rounds executed per race phase.
    pub race_rounds: usize,
    /// Racers the backend answered with throttling across all race rounds.
    ///
    /// A throttled racer is *never* a lost race — the write was refused before
    /// the precondition was evaluated, so it is evidence about request rate and
    /// about nothing else. Counting them separately is what keeps backpressure
    /// from masquerading as conformance. Non-zero here on a passing probe means
    /// "admitted, and the backend was pacing us", which is the expected shape on
    /// a provider with a per-object write ceiling.
    pub throttled_racers: usize,
    /// Race rounds that proved nothing and were re-run.
    ///
    /// See [`ProbeConfig::unproven_round_retries`]. Non-zero on a passing probe
    /// means every round eventually proved itself, but the backend needed more
    /// attempts than a quiet one would.
    pub throttled_rounds_retried: usize,
    /// Shortest observed interval between two same-key mutation rounds, when
    /// the profile spaces them.
    ///
    /// Reported so a passing probe can be checked against the spacing it
    /// claimed to honour rather than trusted to have slept.
    pub min_same_key_gap: Option<Duration>,
    /// Probe objects the cleanup pass could not remove.
    ///
    /// Cleanup failure does not fail the probe — a store that satisfies every
    /// conformance axiom is admitted even if a tidy-up delete flaked — but it
    /// is surfaced because silent probe-key accumulation in a shared bucket is
    /// exactly the kind of leak that is invisible until it is large.
    pub cleanup_failures: usize,
    /// Total number of *transport-unknown* per-racer outcomes across all
    /// race rounds (sum of both `if_match_race` and `if_none_match_race`
    /// phases). A "transport-unknown" is a pre-classification failure —
    /// [`ObjectStoreError::is_ambiguous`] — that means the racer never got a
    /// classified response from the backend, so its outcome is neither
    /// evidence for nor against A3 linearizability. Such racers are
    /// dropped from the observer set (see the race phases for the
    /// invariant: `classified >= 2` and `winners == 1` *among classified
    /// observers*).
    ///
    /// Surfaced on the admission log line so a slowly-degrading backend
    /// shows up before it's a probe failure: a passing probe with
    /// non-zero `transport_drops` is "admitted with degraded
    /// observation count," not silently flaky.
    pub transport_drops: usize,
}

/// Failure carrying the phase that failed plus enough context to diagnose.
#[derive(Debug, thiserror::Error)]
#[error("conformance probe failed in phase '{phase}' (round {round}, key {key}): {reason}")]
pub struct ProbeFailure {
    /// The profile phase that failed.
    ///
    /// S3 profile: `sequential`, `if_match_race`, `if_none_match_race`,
    /// `revision_consistency`. Cloud Storage profile: `immutable`,
    /// `pointer_create`, `pointer_read`, `cas_replace`, `stale_cas`,
    /// `cas_race`, `generation_roundtrip`.
    pub phase: &'static str,
    /// Round index (0-based) when this phase ran multiple rounds.
    pub round: usize,
    /// Object key the failure concerns (or `""` if not key-specific).
    pub key: String,
    /// Human-readable detail.
    pub reason: String,
}

impl From<ProbeFailure> for StoreError {
    fn from(f: ProbeFailure) -> Self {
        StoreError::Probe(f)
    }
}

/// Marks a body written by the Cloud Storage profile's racing writers.
///
/// Public to the crate so a test double can recognise a racer's write without
/// hard-coding the probe's body format.
pub(crate) const GCS_RACE_BODY_PREFIX: &str = "probe-gcs-race:";

/// Build a Cloud Storage profile failure.
fn gcs_failure(phase: &'static str, round: usize, key: &str, reason: String) -> ProbeFailure {
    ProbeFailure {
        phase,
        round,
        key: key.to_string(),
        reason,
    }
}

/// Read the object generation out of a revision the store reported committing.
///
/// A commit with no generation is fatal, not cosmetic: the generation is the
/// only token that can predicate the next write, so a caller handed nothing
/// would have to either stop writing or drop its precondition — and dropping it
/// turns the pointer swap into a blind overwrite. Cloud Storage has no live
/// generation `0`, so zero *is* the absent case.
fn gcs_generation(
    phase: &'static str,
    round: usize,
    key: &str,
    revision: &Revision,
) -> Result<i64, ProbeFailure> {
    match revision.expect_gcs_generation() {
        Ok(generation) if generation > 0 => Ok(generation),
        Ok(_) => Err(gcs_failure(
            phase,
            round,
            key,
            "the store reported a successful write with no object generation".to_string(),
        )),
        Err(error) => Err(gcs_failure(
            phase,
            round,
            key,
            format!("expected an object generation: {error}"),
        )),
    }
}

/// What one Cloud Storage race round established.
enum GcsRaceOutcome {
    /// Exactly one racer committed, witnessed by at least one other classified
    /// racer.
    Committed {
        /// Generation the winner committed.
        generation: i64,
        /// Bytes the winner wrote.
        body: Vec<u8>,
    },
    /// The round is neither pass nor fail — nothing in it distinguishes a
    /// conforming store from a broken one — so it must be re-run.
    Unproven(String),
}

/// Counters and pacing state carried across the Cloud Storage phases.
#[derive(Default)]
struct GcsProbeState {
    transport_drops: usize,
    throttled_racers: usize,
    throttled_rounds_retried: usize,
    min_same_key_gap: Option<Duration>,
    last_same_key_write: Option<Instant>,
}

impl GcsProbeState {
    /// Wait until `spacing` has elapsed since the previous same-key mutation,
    /// and record the interval actually observed.
    ///
    /// The recorded gap is measured, not assumed: the report carries it so a
    /// passing probe can be checked against the spacing it claimed to honour.
    async fn pace(&mut self, spacing: Duration) {
        let now = match self.last_same_key_write {
            None => Instant::now(),
            Some(previous) => {
                let elapsed = previous.elapsed();
                if elapsed < spacing {
                    tokio::time::sleep(spacing - elapsed).await;
                }
                let gap = previous.elapsed();
                self.min_same_key_gap = Some(
                    self.min_same_key_gap
                        .map_or(gap, |shortest| shortest.min(gap)),
                );
                Instant::now()
            }
        };
        self.last_same_key_write = Some(now);
    }
}

/// Object-store client for git refs.
#[derive(Clone)]
pub struct GitStore {
    store: Arc<dyn ObjectStore>,
}

impl GitStore {
    /// Build a git store over an existing object-store client.
    ///
    /// The relay constructs exactly one provider per process and hands the
    /// same client to media storage and to this facade.
    pub fn new(store: Arc<dyn ObjectStore>) -> Self {
        Self { store }
    }

    /// Build a git store over a freshly constructed S3 provider.
    ///
    /// Convenience for tests and the backend conformance probe, which connect
    /// to a bare S3-compatible endpoint without the rest of the relay.
    /// Production shares one client via [`GitStore::new`].
    pub fn from_s3_config(
        endpoint: &str,
        access_key: &str,
        secret_key: &str,
        bucket_name: &str,
        region: &str,
        addressing_style: S3AddressingStyle,
    ) -> Result<Self, StoreError> {
        let store = S3ObjectStore::new(&S3StoreConfig {
            endpoint: endpoint.to_string(),
            access_key: access_key.to_string(),
            secret_key: secret_key.to_string(),
            bucket: bucket_name.to_string(),
            region: region.to_string(),
            addressing_style,
        })
        .map_err(|e| match e {
            ObjectStoreError::Config(message) => StoreError::Config(message),
            other => StoreError::Backend(other),
        })?;
        Ok(Self::new(Arc::new(store)))
    }

    /// Compute the hex SHA-256 of `bytes`. The content-addressed key.
    pub fn content_key(prefix: &str, bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        format!("{prefix}/{}", hex::encode(h.finalize()))
    }

    /// Derive the idx sidecar key for a content-addressed pack digest.
    ///
    /// The idx is a pure cache derived from `packs/<pack_digest>`, so it is
    /// keyed by the pack digest rather than by the idx bytes. This keeps the
    /// manifest schema unchanged: readers can derive `idx/<pack_digest>` from
    /// each manifest pack key.
    pub fn idx_key_for_pack_digest(pack_digest: &str) -> Result<String, StoreError> {
        if pack_digest.len() != 64 || !pack_digest.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(backend(
                "idx_key_for_pack_digest",
                format!("invalid pack digest for idx sidecar: {pack_digest:?}"),
            ));
        }
        Ok(format!("idx/{pack_digest}"))
    }

    /// Create-only write of a content-addressed object (pack or manifest).
    ///
    /// **The caller does not choose the key.** It is derived as
    /// `<prefix>/<hex sha256(bytes)>` inside this method. This makes the
    /// idempotency claim *constructive*: a precondition collision means the key
    /// already holds bytes whose digest equals `sha256(these bytes)`, so by A1
    /// (content-addressing) the stored bytes equal these bytes. Without this
    /// enforcement, a buggy caller passing the wrong key would silently break
    /// A1 detectability on read.
    ///
    /// Returns the key under which the object was written.
    async fn put_immutable(
        &self,
        prefix: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<String, StoreError> {
        let key = Self::content_key(prefix, bytes);
        // Both outcomes are success: by construction the key holds these bytes.
        self.store.put_immutable(&key, bytes, content_type).await?;
        Ok(key)
    }

    /// Write a pack object. Returns the content-addressed key (`packs/<hex>`).
    pub async fn put_pack(&self, bytes: &[u8]) -> Result<String, StoreError> {
        self.put_immutable("packs", bytes, "application/x-git-pack")
            .await
    }

    /// Best-effort create-only write of an idx sidecar for `packs/<pack_digest>`.
    ///
    /// Unlike packs/manifests, the key is not the SHA-256 of `idx_bytes`; it is
    /// `idx/<pack_digest>` so hydrates can derive it without changing manifest
    /// bytes. A precondition collision is idempotent success for the cache
    /// layer: the first writer already produced the sidecar for this pack, and
    /// hydrate validates before trusting it.
    pub async fn put_idx(&self, pack_digest: &str, idx_bytes: &[u8]) -> Result<String, StoreError> {
        let key = Self::idx_key_for_pack_digest(pack_digest)?;
        self.store
            .put_immutable(&key, idx_bytes, "application/x-git-index")
            .await?;
        Ok(key)
    }

    /// Read an idx sidecar for `packs/<pack_digest>`.
    ///
    /// A missing idx is a cache miss, not a hydrate failure; callers should
    /// regenerate with `git index-pack`. Other backend failures are surfaced so
    /// callers can decide whether to fall back or fail.
    pub async fn get_idx(
        &self,
        pack_digest: &str,
        max_bytes: u64,
    ) -> Result<Option<Bytes>, StoreError> {
        let key = Self::idx_key_for_pack_digest(pack_digest)?;
        match self.get_limited(&key, max_bytes).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(StoreError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Write a manifest object. Returns the content-addressed key (`manifests/<hex>`).
    pub async fn put_manifest(&self, bytes: &[u8]) -> Result<String, StoreError> {
        self.put_immutable("manifests", bytes, "application/json")
            .await
    }

    /// GET an object without digest verification.
    ///
    /// Prefer `get_verified` for pack/manifest reads — that is what enforces A1
    /// detectability. This raw `get` exists for the pointer (whose key is not a
    /// digest).
    pub async fn get(&self, key: &str) -> Result<Bytes, StoreError> {
        Ok(self.store.get(key).await?)
    }

    /// GET an object and verify its bytes hash to `expected_digest` (hex SHA-256).
    ///
    /// This is the read-side enforcement of A1 — any deviation from the
    /// content-addressed invariant becomes a `DigestMismatch` error, never a
    /// silent corruption.
    pub async fn get_verified(
        &self,
        key: &str,
        expected_digest: &str,
    ) -> Result<Bytes, StoreError> {
        let bytes = self.get(key).await?;
        Self::verify_digest(key, expected_digest, bytes)
    }

    /// GET an immutable object after rejecting objects larger than `max_bytes`.
    ///
    /// Pack and manifest objects are content addressed and create-only, so the
    /// HEAD result cannot race with a different body at the same key. The
    /// second length check protects against a backend that reports a bad
    /// content length.
    pub async fn get_verified_limited(
        &self,
        key: &str,
        expected_digest: &str,
        max_bytes: u64,
    ) -> Result<Bytes, StoreError> {
        let bytes = self.get_limited(key, max_bytes).await?;
        Self::verify_digest(key, expected_digest, bytes)
    }

    /// GET an object after rejecting bodies larger than `max_bytes`.
    pub async fn get_limited(&self, key: &str, max_bytes: u64) -> Result<Bytes, StoreError> {
        Ok(self.store.get_limited(key, max_bytes).await?)
    }

    /// GET the pointer object, returning its revision and bytes *from the same
    /// response* — atomic snapshot.
    ///
    /// Returns `Ok(None)` if the pointer does not exist (first-push case).
    ///
    /// **Why one GET, not HEAD-then-GET.** A separate HEAD followed by GET
    /// can straddle a concurrent writer: the HEAD's revision and the GET's
    /// body would describe different pointer versions, and a caller that
    /// later predicated a CAS on the HEAD revision would be predicating on a
    /// version it never actually read. Reading both fields from the GET
    /// response keeps the snapshot consistent (A2: a single GET observes a
    /// single committed object).
    pub async fn get_pointer(&self, key: &str) -> Result<Option<(Revision, Bytes)>, StoreError> {
        Ok(self.store.get_with_revision(key).await?)
    }

    /// Write the pointer under a precondition (§Push step 7 — the CAS).
    ///
    /// Returns [`ConditionalWrite::Conflict`] when the precondition did not
    /// hold (the standard losing outcome). On
    /// [`ConditionalWrite::Committed`], the returned [`Revision`] is read from
    /// the write response — callers use it to predicate the next CAS.
    pub async fn put_pointer(
        &self,
        key: &str,
        body: &[u8],
        condition: WriteCondition,
    ) -> Result<ConditionalWrite, StoreError> {
        Ok(self
            .store
            .put_conditional(key, body, "application/json", condition)
            .await?)
    }

    /// Hash `bytes` and reject anything that does not match `expected_digest`.
    fn verify_digest(key: &str, expected_digest: &str, bytes: Bytes) -> Result<Bytes, StoreError> {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if actual != expected_digest {
            return Err(StoreError::DigestMismatch {
                key: key.into(),
                expected: expected_digest.into(),
                actual,
            });
        }
        Ok(bytes)
    }

    /// Conformance probe — deployment gate per `docs/git-on-object-storage.md`
    /// §Conformance. Fail-closed: any phase failure returns
    /// `StoreError::Probe(ProbeFailure)` and the caller (relay startup) MUST
    /// refuse to come up.
    ///
    /// The profile is chosen by the configured provider, not by the caller: the
    /// axioms are the same for every backend, but the evidence that admits one
    /// is provider-shaped. An S3-compatible store is admitted by a wide race on
    /// ETag preconditions; Cloud Storage is admitted by a paced race on object
    /// generations, where a refused (throttled) write is not a lost race.
    pub async fn run_conformance_probe(&self, cfg: ProbeConfig) -> Result<ProbeReport, StoreError> {
        if cfg.race_width < 2 || cfg.race_rounds == 0 {
            return Err(ProbeFailure {
                phase: "config",
                round: 0,
                key: String::new(),
                reason: format!(
                    "race_width must be ≥ 2 and race_rounds ≥ 1, got {}/{}",
                    cfg.race_width, cfg.race_rounds
                ),
            }
            .into());
        }
        match self.store.provider() {
            ProviderKind::S3 => self.run_s3_conformance_probe(cfg).await,
            ProviderKind::Gcs => self.run_gcs_conformance_probe(cfg).await,
        }
    }

    /// The S3 profile: revision-token compare-and-swap under a wide race.
    ///
    /// Four phases:
    ///
    /// 1. **`sequential`** — write a content-addressed object, read it back,
    ///    verify bytes. Tests A1 (content-addressed write) + A2
    ///    (read-after-write).
    /// 2. **`if_match_race`** — `race_width` parallel `put_pointer` calls
    ///    predicated on the same revision. Exactly one must commit; the rest
    ///    must conflict. Tests A3.
    /// 3. **`if_none_match_race`** — `race_width` parallel create-only writes
    ///    against the same digest-shaped key (the same `put_immutable` path
    ///    `put_pack`/`put_manifest` use). Tests A1 + A3 on the create-only
    ///    primitive. Counts raw outcomes (exactly one create, rest already
    ///    present) and asserts final stored bytes equal the racers' bytes.
    /// 4. **`revision_consistency`** — round-trip a revision from
    ///    `get_pointer` into `put_pointer(Matches(...))` and assert it
    ///    commits. Tests that the token is opaque and stable between read and
    ///    CAS.
    async fn run_s3_conformance_probe(&self, cfg: ProbeConfig) -> Result<ProbeReport, StoreError> {
        use std::sync::Arc;
        let nonce = uuid::Uuid::new_v4();
        let pointer_key = format!("probe/pointer-{nonce}");
        // Accumulator for *transport-unknown* per-racer outcomes across both
        // race phases. See `ProbeReport::transport_drops` for the rationale.
        let mut transport_drops = 0usize;

        // -- Phase 1: sequential --------------------------------------------------
        for round in 0..cfg.race_rounds {
            let body = format!("probe-sequential-{nonce}-{round}").into_bytes();
            let key = self.put_pack(&body).await?;
            let got = self
                .get_verified(&key, &Self::digest_hex(&body))
                .await
                .map_err(|e| ProbeFailure {
                    phase: "sequential",
                    round,
                    key: key.clone(),
                    reason: format!("read-after-write failed: {e}"),
                })?;
            if got[..] != body[..] {
                return Err(ProbeFailure {
                    phase: "sequential",
                    round,
                    key,
                    reason: "read-after-write bytes mismatch".into(),
                }
                .into());
            }
        }

        // -- Phase 2: if_match_race -----------------------------------------------
        // Seed the pointer with a known value, then race N CAS updates.
        let seed = b"probe-pointer-seed".to_vec();
        let _ = self.store.delete(&pointer_key).await; // ignore absence
        let seed_outcome = self
            .put_pointer(&pointer_key, &seed, WriteCondition::Absent)
            .await?;
        let mut revision = match seed_outcome {
            ConditionalWrite::Committed(revision) => revision,
            ConditionalWrite::Conflict => {
                return Err(ProbeFailure {
                    phase: "if_match_race",
                    round: 0,
                    key: pointer_key,
                    reason: "could not seed pointer (lost race against self)".into(),
                }
                .into())
            }
        };
        for round in 0..cfg.race_rounds {
            let arc_self: Arc<&Self> = Arc::new(self);
            let mut tasks = Vec::with_capacity(cfg.race_width);
            for i in 0..cfg.race_width {
                let me = Arc::clone(&arc_self);
                let pkey = pointer_key.clone();
                let condition = WriteCondition::Matches(revision.clone());
                let body = format!("round={round},racer={i},nonce={nonce}").into_bytes();
                tasks.push(async move { me.put_pointer(&pkey, &body, condition).await });
            }
            let outcomes = futures_util::future::join_all(tasks).await;
            // Drop-and-floor classification. An ambiguous provider outcome
            // means the racer never got a classified response from the
            // backend (couldn't open a socket, send flaked, etc.); its
            // outcome is *unknown*, not negative. A3 is a claim about
            // **observers**: dropping unknowns from the observer set
            // sharpens the assertion ("exactly one winner among observers")
            // and avoids smuggling a network-stack test into the
            // conformance probe. Every other failure — a malformed
            // response, an unexpected status — means the backend *did*
            // answer but not in the contract shape, which is a real
            // conformance signal and fails closed.
            let mut classified = 0usize;
            let mut winners = 0usize;
            let mut new_revision: Option<Revision> = None;
            for (i, outcome) in outcomes.into_iter().enumerate() {
                match outcome {
                    Ok(ConditionalWrite::Committed(committed)) => {
                        classified += 1;
                        winners += 1;
                        new_revision = Some(committed);
                    }
                    Ok(ConditionalWrite::Conflict) => {
                        classified += 1;
                    }
                    Err(StoreError::Backend(ref e)) if e.is_ambiguous() => {
                        transport_drops += 1;
                        tracing::warn!(
                            phase = "if_match_race",
                            round,
                            racer = i,
                            "transport drop (pre-classification: socket/send failure)"
                        );
                    }
                    Err(e) => {
                        return Err(ProbeFailure {
                            phase: "if_match_race",
                            round,
                            key: pointer_key,
                            reason: format!("racer {i}: {e}"),
                        }
                        .into())
                    }
                }
            }
            // A3 needs ≥2 observers to *see* a race. With 31/32 classified
            // and 1 transport drop, the race is well-observed; with 0/32
            // classified the probe didn't run at all — fail closed.
            if classified < 2 {
                return Err(ProbeFailure {
                    phase: "if_match_race",
                    round,
                    key: pointer_key,
                    reason: format!(
                        "race not observed: classified={classified}, transport_drops={}",
                        cfg.race_width - classified
                    ),
                }
                .into());
            }
            if winners != 1 {
                return Err(ProbeFailure {
                    phase: "if_match_race",
                    round,
                    key: pointer_key,
                    reason: format!(
                        "expected exactly 1 winner among {classified} classified observers, got {winners}"
                    ),
                }
                .into());
            }
            revision = new_revision.expect("winner exists");
        }

        // -- Phase 3: if_none_match_race ------------------------------------------
        // N parallel create-only writes targeting the same digest-shaped key.
        // Bypass `put_immutable`'s collision-swallow to count raw outcomes.
        for round in 0..cfg.race_rounds {
            let body = format!("probe-inm-race-{nonce}-{round}").into_bytes();
            let key = Self::content_key("probe/inm-race", &body);
            // Clean slate.
            let _ = self.store.delete(&key).await;
            let arc_self: Arc<&Self> = Arc::new(self);
            let mut tasks = Vec::with_capacity(cfg.race_width);
            for _ in 0..cfg.race_width {
                let me = Arc::clone(&arc_self);
                let k = key.clone();
                let b = body.clone();
                tasks.push(async move { me.put_immutable_raw(&k, &b).await });
            }
            let results = futures_util::future::join_all(tasks).await;
            // Drop-and-floor: same classification rule as Phase 2. Drop
            // ambiguous pre-classification failures; count created +
            // already-present as the classified observers. Any other
            // `StoreError` is a real conformance signal and fails closed.
            let mut classified = 0usize;
            let mut created = 0usize;
            let mut collisions = 0usize;
            for (i, r) in results.into_iter().enumerate() {
                match r {
                    Ok(ImmutableWrite::Created) => {
                        classified += 1;
                        created += 1;
                    }
                    Ok(ImmutableWrite::AlreadyPresent) => {
                        classified += 1;
                        collisions += 1;
                    }
                    Err(StoreError::Backend(ref e)) if e.is_ambiguous() => {
                        transport_drops += 1;
                        tracing::warn!(
                            phase = "if_none_match_race",
                            round,
                            racer = i,
                            "transport drop (pre-classification: socket/send failure)"
                        );
                    }
                    Err(e) => {
                        return Err(ProbeFailure {
                            phase: "if_none_match_race",
                            round,
                            key,
                            reason: format!("racer {i} backend error: {e}"),
                        }
                        .into())
                    }
                }
            }
            // Floor: A3 needs ≥2 observers to *see* a race.
            if classified < 2 {
                return Err(ProbeFailure {
                    phase: "if_none_match_race",
                    round,
                    key,
                    reason: format!(
                        "race not observed: classified={classified}, transport_drops={}",
                        cfg.race_width - classified
                    ),
                }
                .into());
            }
            // Create-only contract: exactly 1 create + (classified − 1)
            // collisions *among observers*. A fixed `race_width − 1` would
            // false-positive on any transport drop; this expression honors
            // the drop-and-floor invariant.
            if created != 1 || collisions != classified - 1 {
                return Err(ProbeFailure {
                    phase: "if_none_match_race",
                    round,
                    key,
                    reason: format!(
                        "expected 1 create + {} collisions among {classified} classified observers, got {created} creates + {collisions} collisions",
                        classified - 1
                    ),
                }
                .into());
            }
            // Final bytes must equal the racers' bytes (content-addressed: any
            // winner stored the same bytes by construction).
            let read = self
                .get_verified(&key, &Self::digest_hex(&body))
                .await
                .map_err(|e| ProbeFailure {
                    phase: "if_none_match_race",
                    round,
                    key: key.clone(),
                    reason: format!("post-race verified read failed: {e}"),
                })?;
            if read[..] != body[..] {
                return Err(ProbeFailure {
                    phase: "if_none_match_race",
                    round,
                    key,
                    reason: "post-race bytes mismatch".into(),
                }
                .into());
            }
        }

        // -- Phase 4: revision_consistency ----------------------------------------
        // GET pointer, take its revision, CAS-update with that revision, expect
        // a commit. Proves the token round-trips opaquely between read and write.
        for round in 0..cfg.race_rounds {
            let (observed, _bytes) =
                self.get_pointer(&pointer_key)
                    .await?
                    .ok_or_else(|| ProbeFailure {
                        phase: "revision_consistency",
                        round,
                        key: pointer_key.clone(),
                        reason: "pointer vanished mid-probe".into(),
                    })?;
            let body = format!("probe-revision-{round}-{nonce}").into_bytes();
            match self
                .put_pointer(&pointer_key, &body, WriteCondition::Matches(observed))
                .await?
            {
                ConditionalWrite::Committed(_) => {}
                ConditionalWrite::Conflict => {
                    return Err(ProbeFailure {
                        phase: "revision_consistency",
                        round,
                        key: pointer_key,
                        reason: "GET-revision → CAS chain lost race in a quiescent probe".into(),
                    }
                    .into())
                }
            }
        }

        // Cleanup pointer (immutable probe writes accumulate by design; the
        // bucket's retention policy handles them, not the probe).
        let _ = self.store.delete(&pointer_key).await;

        Ok(ProbeReport {
            profile: ProviderKind::S3,
            race_width: cfg.race_width,
            race_rounds: cfg.race_rounds,
            transport_drops,
            throttled_racers: 0,
            throttled_rounds_retried: 0,
            min_same_key_gap: None,
            cleanup_failures: 0,
        })
    }

    /// The Cloud Storage profile: object-generation compare-and-swap, paced to
    /// the provider's published per-object write ceiling.
    ///
    /// Same axioms, different evidence. Cloud Storage answers a stale
    /// precondition with 412 (an ordinary conflict) and an over-rate write with
    /// 429 (a refusal to evaluate the precondition at all), and it publishes a
    /// maximum of one write per second to a single object name. A profile that
    /// ignored either fact would be measuring the rate limiter: a burst of 429s
    /// would either be miscounted as lost races — turning "the backend paced us"
    /// into "the backend admitted two winners' worth of losers" — or would make
    /// a round pass with no race in it. So this profile races narrowly, spaces
    /// same-key rounds past the ceiling, classifies throttles separately from
    /// conflicts, and re-runs a round that proved nothing rather than scoring
    /// it.
    ///
    /// Phases, in order:
    ///
    /// 1. **`immutable`** — create-only write of a content-addressed object,
    ///    read back, digest verified. A1 plus read-after-write.
    /// 2. **`pointer_create`** — create the pointer under the create-only
    ///    precondition, which Cloud Storage spells as generation `0`.
    /// 3. **`pointer_read`** — read body and generation from one response and
    ///    check both against what was just committed. A2.
    /// 4. **`cas_replace`** — replace under the observed generation; the commit
    ///    must report a *different* generation.
    /// 5. **`stale_cas`** — replay the superseded generation; must conflict. A
    ///    store that commits here is performing blind overwrites.
    /// 6. **`cas_race`** — `race_width` writers on one generation, `race_rounds`
    ///    times: exactly one commit, every other classified racer a conflict or
    ///    a throttle, and the stored object equal to the winner's.
    /// 7. **`generation_roundtrip`** — the winning generation predicates the
    ///    next successful compare-and-swap, closing the loop the push path
    ///    depends on.
    ///
    /// Probe objects are removed afterwards on both the success and the failure
    /// path; a cleanup failure is reported, not fatal.
    async fn run_gcs_conformance_probe(&self, cfg: ProbeConfig) -> Result<ProbeReport, StoreError> {
        let nonce = uuid::Uuid::new_v4();
        let mut written = Vec::new();
        let mut state = GcsProbeState::default();

        let outcome = self
            .gcs_probe_phases(&cfg, nonce, &mut written, &mut state)
            .await;
        let cleanup_failures = self.remove_probe_objects(&written).await;
        outcome?;

        Ok(ProbeReport {
            profile: ProviderKind::Gcs,
            race_width: cfg.race_width,
            race_rounds: cfg.race_rounds,
            transport_drops: state.transport_drops,
            throttled_racers: state.throttled_racers,
            throttled_rounds_retried: state.throttled_rounds_retried,
            min_same_key_gap: state.min_same_key_gap,
            cleanup_failures,
        })
    }

    /// The Cloud Storage phases, factored out so cleanup runs on every path.
    async fn gcs_probe_phases(
        &self,
        cfg: &ProbeConfig,
        nonce: uuid::Uuid,
        written: &mut Vec<String>,
        state: &mut GcsProbeState,
    ) -> Result<(), StoreError> {
        // -- Phase 1: immutable ------------------------------------------------
        // The nonce makes this key new, so the create-only write must report a
        // create rather than a collision.
        let body = format!("probe-gcs-immutable-{nonce}").into_bytes();
        let key = Self::content_key("probe/gcs-immutable", &body);
        written.push(key.clone());
        let outcome = self
            .put_immutable_raw(&key, &body)
            .await
            .map_err(|e| gcs_failure("immutable", 0, &key, format!("create-only write: {e}")))?;
        if outcome != ImmutableWrite::Created {
            return Err(gcs_failure(
                "immutable",
                0,
                &key,
                "a freshly nonced key reported a collision, so the create-only \
                 precondition is not being evaluated"
                    .to_string(),
            )
            .into());
        }
        let read = self
            .get_verified(&key, &Self::digest_hex(&body))
            .await
            .map_err(|e| gcs_failure("immutable", 0, &key, format!("verified read: {e}")))?;
        if read[..] != body[..] {
            return Err(gcs_failure(
                "immutable",
                0,
                &key,
                "read-after-write returned different bytes".to_string(),
            )
            .into());
        }

        // -- Phase 2: pointer_create -------------------------------------------
        let pointer_key = format!("probe/gcs-pointer-{nonce}");
        written.push(pointer_key.clone());
        let seed = format!("probe-gcs-pointer-seed-{nonce}").into_bytes();
        state.pace(cfg.same_key_spacing).await;
        let created = self
            .put_pointer(&pointer_key, &seed, WriteCondition::Absent)
            .await
            .map_err(|e| {
                gcs_failure(
                    "pointer_create",
                    0,
                    &pointer_key,
                    format!("create-only pointer write: {e}"),
                )
            })?;
        let mut generation = match created {
            ConditionalWrite::Committed(revision) => {
                gcs_generation("pointer_create", 0, &pointer_key, &revision)?
            }
            ConditionalWrite::Conflict => {
                return Err(gcs_failure(
                    "pointer_create",
                    0,
                    &pointer_key,
                    "a freshly nonced pointer key was already taken".to_string(),
                )
                .into())
            }
        };

        // -- Phase 3: pointer_read ---------------------------------------------
        // Body and generation must describe the same committed object, or the
        // generation a caller predicates its next write on names a version it
        // never read.
        let (revision, stored) = self
            .get_pointer(&pointer_key)
            .await
            .map_err(|e| gcs_failure("pointer_read", 0, &pointer_key, format!("read: {e}")))?
            .ok_or_else(|| {
                gcs_failure(
                    "pointer_read",
                    0,
                    &pointer_key,
                    "the pointer just committed does not exist".to_string(),
                )
            })?;
        let observed = gcs_generation("pointer_read", 0, &pointer_key, &revision)?;
        if observed != generation {
            return Err(gcs_failure(
                "pointer_read",
                0,
                &pointer_key,
                format!(
                    "read reported generation {observed} for the object committed as {generation}"
                ),
            )
            .into());
        }
        if stored[..] != seed[..] {
            return Err(gcs_failure(
                "pointer_read",
                0,
                &pointer_key,
                "read returned bytes other than the committed body".to_string(),
            )
            .into());
        }

        // -- Phase 4: cas_replace ----------------------------------------------
        let replacement = format!("probe-gcs-replace-{nonce}").into_bytes();
        state.pace(cfg.same_key_spacing).await;
        let superseded = generation;
        generation = match self
            .put_pointer(
                &pointer_key,
                &replacement,
                WriteCondition::Matches(Revision::GcsGeneration(generation)),
            )
            .await
            .map_err(|e| gcs_failure("cas_replace", 0, &pointer_key, format!("write: {e}")))?
        {
            ConditionalWrite::Committed(revision) => {
                let committed = gcs_generation("cas_replace", 0, &pointer_key, &revision)?;
                if committed == superseded {
                    return Err(gcs_failure(
                        "cas_replace",
                        0,
                        &pointer_key,
                        format!(
                            "the replacement reported the same generation {superseded} it \
                             replaced, so the token cannot distinguish versions"
                        ),
                    )
                    .into());
                }
                committed
            }
            ConditionalWrite::Conflict => {
                return Err(gcs_failure(
                    "cas_replace",
                    0,
                    &pointer_key,
                    "a compare-and-swap on the just-read generation conflicted with no \
                     competing writer"
                        .to_string(),
                )
                .into())
            }
        };

        // -- Phase 5: stale_cas ------------------------------------------------
        let stale_body = format!("probe-gcs-stale-{nonce}").into_bytes();
        state.pace(cfg.same_key_spacing).await;
        match self
            .put_pointer(
                &pointer_key,
                &stale_body,
                WriteCondition::Matches(Revision::GcsGeneration(superseded)),
            )
            .await
            .map_err(|e| gcs_failure("stale_cas", 0, &pointer_key, format!("write: {e}")))?
        {
            ConditionalWrite::Conflict => {}
            ConditionalWrite::Committed(_) => {
                return Err(gcs_failure(
                    "stale_cas",
                    0,
                    &pointer_key,
                    format!(
                        "a write predicated on superseded generation {superseded} committed: \
                         the precondition is not being enforced, so every pointer update is a \
                         blind overwrite"
                    ),
                )
                .into())
            }
        }

        // -- Phase 6: cas_race -------------------------------------------------
        for round in 0..cfg.race_rounds {
            let mut attempt = 0usize;
            let winner = loop {
                state.pace(cfg.same_key_spacing).await;
                match self
                    .gcs_race_round(round, attempt, &pointer_key, nonce, generation, cfg, state)
                    .await?
                {
                    GcsRaceOutcome::Committed { generation, body } => {
                        break (generation, body);
                    }
                    GcsRaceOutcome::Unproven(reason) => {
                        if attempt >= cfg.unproven_round_retries {
                            return Err(gcs_failure(
                                "cas_race",
                                round,
                                &pointer_key,
                                format!(
                                    "no round proved a race in {} attempts: {reason}",
                                    attempt + 1
                                ),
                            )
                            .into());
                        }
                        attempt += 1;
                        state.throttled_rounds_retried += 1;
                        tracing::warn!(
                            phase = "cas_race",
                            round,
                            attempt,
                            reason = %reason,
                            "conformance race round proved nothing; re-running"
                        );
                    }
                }
            };
            let (committed, body) = winner;

            // The stored object must be the winner's, at the winner's
            // generation: a loser's payload surviving the race is the failure
            // mode the whole pointer protocol exists to exclude.
            let (revision, stored) = self
                .get_pointer(&pointer_key)
                .await
                .map_err(|e| {
                    gcs_failure(
                        "cas_race",
                        round,
                        &pointer_key,
                        format!("post-race read: {e}"),
                    )
                })?
                .ok_or_else(|| {
                    gcs_failure(
                        "cas_race",
                        round,
                        &pointer_key,
                        "the pointer vanished during the race".to_string(),
                    )
                })?;
            let settled = gcs_generation("cas_race", round, &pointer_key, &revision)?;
            if settled != committed {
                return Err(gcs_failure(
                    "cas_race",
                    round,
                    &pointer_key,
                    format!(
                        "the winner committed generation {committed} but the object settled at \
                         {settled}"
                    ),
                )
                .into());
            }
            if stored[..] != body[..] {
                return Err(gcs_failure(
                    "cas_race",
                    round,
                    &pointer_key,
                    "the object holds bytes no racer reported committing".to_string(),
                )
                .into());
            }
            generation = committed;
        }

        // -- Phase 7: generation_roundtrip -------------------------------------
        // The generation a racer won with must predicate the next write; that
        // chain — commit, then compare-and-swap on the returned token — is
        // exactly what the push path does between two pushes.
        let final_body = format!("probe-gcs-roundtrip-{nonce}").into_bytes();
        state.pace(cfg.same_key_spacing).await;
        match self
            .put_pointer(
                &pointer_key,
                &final_body,
                WriteCondition::Matches(Revision::GcsGeneration(generation)),
            )
            .await
            .map_err(|e| {
                gcs_failure(
                    "generation_roundtrip",
                    0,
                    &pointer_key,
                    format!("write: {e}"),
                )
            })? {
            ConditionalWrite::Committed(revision) => {
                let committed = gcs_generation("generation_roundtrip", 0, &pointer_key, &revision)?;
                if committed == generation {
                    return Err(gcs_failure(
                        "generation_roundtrip",
                        0,
                        &pointer_key,
                        format!("the write reported the same generation {generation} it replaced"),
                    )
                    .into());
                }
            }
            ConditionalWrite::Conflict => {
                return Err(gcs_failure(
                    "generation_roundtrip",
                    0,
                    &pointer_key,
                    "the generation a racer committed did not predicate the next write, so a \
                     winner cannot chain its own pushes"
                        .to_string(),
                )
                .into())
            }
        }

        Ok(())
    }

    /// One race round: `cfg.race_width` writers on the same generation.
    ///
    /// Returns the winner when the round proved something, and
    /// [`GcsRaceOutcome::Unproven`] when it did not. Only a semantic violation —
    /// two winners, a commit with no generation, an unclassifiable backend
    /// answer — fails here.
    #[allow(clippy::too_many_arguments)]
    async fn gcs_race_round(
        &self,
        round: usize,
        attempt: usize,
        pointer_key: &str,
        nonce: uuid::Uuid,
        generation: i64,
        cfg: &ProbeConfig,
        state: &mut GcsProbeState,
    ) -> Result<GcsRaceOutcome, StoreError> {
        let mut tasks = Vec::with_capacity(cfg.race_width);
        for racer in 0..cfg.race_width {
            let body =
                format!("{GCS_RACE_BODY_PREFIX}{round}:{attempt}:{racer}:{nonce}").into_bytes();
            let condition = WriteCondition::Matches(Revision::GcsGeneration(generation));
            tasks.push(async move {
                let outcome = self.put_pointer(pointer_key, &body, condition).await;
                (racer, body, outcome)
            });
        }

        let mut winners: Vec<(i64, Vec<u8>)> = Vec::new();
        let mut conflicts = 0usize;
        let mut throttled = 0usize;
        let mut drops = 0usize;
        for (racer, body, outcome) in futures_util::future::join_all(tasks).await {
            match outcome {
                Ok(ConditionalWrite::Committed(revision)) => {
                    let committed = gcs_generation("cas_race", round, pointer_key, &revision)?;
                    winners.push((committed, body));
                }
                Ok(ConditionalWrite::Conflict) => conflicts += 1,
                // Throttling is a refusal to evaluate the precondition, so it
                // says nothing about who won. It is counted, never scored.
                Err(StoreError::Backend(ObjectStoreError::Throttled { .. })) => {
                    throttled += 1;
                    state.throttled_racers += 1;
                }
                Err(StoreError::Backend(ref e)) if e.is_ambiguous() => {
                    drops += 1;
                    state.transport_drops += 1;
                    tracing::warn!(
                        phase = "cas_race",
                        round,
                        racer,
                        "transport drop (pre-classification: socket/send failure)"
                    );
                }
                Err(e) => {
                    return Err(gcs_failure(
                        "cas_race",
                        round,
                        pointer_key,
                        format!("racer {racer}: {e}"),
                    )
                    .into())
                }
            }
        }

        if winners.len() > 1 {
            return Err(gcs_failure(
                "cas_race",
                round,
                pointer_key,
                format!(
                    "{} racers committed on one generation: the store is not linearizing \
                     conditional writes",
                    winners.len()
                ),
            )
            .into());
        }

        // Classified observers. A throttled racer is one: it proves the round
        // ran, even though it proves nothing about the precondition.
        let classified = winners.len() + conflicts + throttled;
        if let Some((committed, body)) = winners.pop() {
            if classified < 2 {
                return Ok(GcsRaceOutcome::Unproven(format!(
                    "only {classified} of {} racers were classified, so no race was witnessed \
                     ({drops} transport drops)",
                    cfg.race_width
                )));
            }
            return Ok(GcsRaceOutcome::Committed {
                generation: committed,
                body,
            });
        }

        if conflicts == 0 {
            return Ok(GcsRaceOutcome::Unproven(format!(
                "no racer committed and none saw the generation move ({throttled} throttled, \
                 {drops} transport drops)"
            )));
        }
        if drops > 0 {
            return Ok(GcsRaceOutcome::Unproven(format!(
                "{conflicts} racers saw the generation move but the committing racer's outcome \
                 was never classified ({drops} transport drops)"
            )));
        }
        Err(gcs_failure(
            "cas_race",
            round,
            pointer_key,
            format!(
                "{conflicts} racers were told the generation had moved, but no racer committed \
                 and every outcome was classified: the object changed without an acknowledged \
                 writer"
            ),
        )
        .into())
    }

    /// Delete the probe's objects, returning how many could not be removed.
    ///
    /// Runs on the failure path too: a failed probe is the one that gets
    /// re-run, so that is exactly when leaking keys into the deployment's own
    /// bucket must not happen.
    async fn remove_probe_objects(&self, keys: &[String]) -> usize {
        let mut failures = 0usize;
        for key in keys {
            if let Err(error) = self.store.delete(key).await {
                failures += 1;
                tracing::warn!(%error, "conformance probe could not remove its object");
            }
        }
        failures
    }

    /// Helper: hex SHA-256 of bytes.
    fn digest_hex(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        hex::encode(h.finalize())
    }

    /// Raw create-only PUT exposed for the probe's race-counting phase, where
    /// we need to *see* collision outcomes rather than swallow them as
    /// idempotent. Bubbles anything that is not a create-or-collision.
    async fn put_immutable_raw(
        &self,
        key: &str,
        bytes: &[u8],
    ) -> Result<ImmutableWrite, StoreError> {
        Ok(self
            .store
            .put_immutable(key, bytes, "application/octet-stream")
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idx_key_uses_pack_digest_namespace() {
        let digest = "a".repeat(64);
        assert_eq!(
            GitStore::idx_key_for_pack_digest(&digest).unwrap(),
            format!("idx/{digest}")
        );
    }

    #[test]
    fn idx_key_rejects_non_digest_input() {
        for bad in ["packs/abc", "abc", "../escape", &"g".repeat(64)] {
            assert!(GitStore::idx_key_for_pack_digest(bad).is_err());
        }
    }

    /// The provider errors git treats as domain outcomes must surface as their
    /// own variants — call sites match on `NotFound` / `ObjectTooLarge` /
    /// `DigestMismatch`, not on a wrapped backend error.
    #[test]
    fn provider_errors_lift_into_domain_variants() {
        assert!(matches!(
            StoreError::from(ObjectStoreError::NotFound {
                key: "packs/x".into()
            }),
            StoreError::NotFound(ref key) if key == "packs/x"
        ));
        assert!(matches!(
            StoreError::from(ObjectStoreError::ObjectTooLarge {
                key: "packs/x".into(),
                size: 9,
                max: 4,
            }),
            StoreError::ObjectTooLarge {
                size: 9,
                max: 4,
                ..
            }
        ));
        assert!(matches!(
            StoreError::from(ObjectStoreError::DigestMismatch {
                key: "packs/x".into(),
                expected: "a".into(),
                actual: "b".into(),
            }),
            StoreError::DigestMismatch { .. }
        ));
    }

    /// Everything else stays a backend error, and only pre-classification
    /// failures are ambiguous — the probe's drop-and-floor rule reads exactly
    /// this predicate to decide which racers leave the observer set.
    #[test]
    fn other_provider_errors_stay_backend_and_keep_their_classification() {
        let permanent = StoreError::from(ObjectStoreError::Provider {
            operation: "put_conditional",
            message: "AccessDenied".into(),
        });
        match permanent {
            StoreError::Backend(ref e) => assert!(!e.is_ambiguous()),
            other => panic!("expected Backend, got {other:?}"),
        }

        let unknown = StoreError::from(ObjectStoreError::TransportAmbiguous {
            operation: "put_conditional",
            message: "connection reset".into(),
        });
        match unknown {
            StoreError::Backend(ref e) => assert!(e.is_ambiguous()),
            other => panic!("expected Backend, got {other:?}"),
        }
    }

    #[test]
    fn partial_static_keys_are_rejected() {
        for (access, secret) in [("buzz_dev", ""), ("", "buzz_dev_secret")] {
            let err = match GitStore::from_s3_config(
                "http://localhost:9000",
                access,
                secret,
                "buzz-git",
                "us-east-1",
                S3AddressingStyle::Path,
            ) {
                Ok(_) => {
                    panic!("partial static creds must not silently use the credential chain")
                }
                Err(err) => err,
            };
            assert!(
                matches!(err, StoreError::Config(_)),
                "expected Config error, got {err:?}"
            );
        }
    }
}

#[cfg(test)]
mod profiles {
    //! Profile behaviour against a scripted store.
    //!
    //! The probe consumes the object-store seam, so a test double can answer a
    //! race any way a real backend could — including ways no conforming backend
    //! ever would. That is the point: a conformance gate is only worth its boot
    //! time if it *fails* on the answers it claims to reject, and a live bucket
    //! cannot be asked to commit two writers on one generation.

    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Mutex;

    use async_trait::async_trait;
    use buzz_object_store::{BulkDeleteOutcome, ByteStream, ListPage, ObjectMeta};

    use super::*;

    /// How a scripted racer answers.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum RacerOutcome {
        /// Commit regardless of the precondition — the only way to stage two
        /// winners on one generation.
        Win,
        /// Commit, but report no object generation.
        WinWithoutGeneration,
        /// The ordinary lost-race answer.
        Conflict,
        /// Refuse the write for request rate.
        Throttle,
        /// Never produce a classified answer.
        Drop,
    }

    /// An in-memory object store with real compare-and-swap semantics, plus a
    /// script that can override the answers to the probe's racing writers.
    struct ScriptedStore {
        provider: ProviderKind,
        objects: Mutex<HashMap<String, (i64, Bytes)>>,
        next_generation: AtomicI64,
        race_script: Mutex<VecDeque<RacerOutcome>>,
        /// Blind-overwrite bug: commit even when the precondition is stale.
        accept_stale_precondition: bool,
        /// Report a created object as committed with no generation.
        zero_generation_on_create: bool,
    }

    impl ScriptedStore {
        fn new(provider: ProviderKind) -> Self {
            Self {
                provider,
                objects: Mutex::new(HashMap::new()),
                // No live object has generation 0; that value means "absent".
                next_generation: AtomicI64::new(1),
                race_script: Mutex::new(VecDeque::new()),
                accept_stale_precondition: false,
                zero_generation_on_create: false,
            }
        }

        /// Script consecutive race rounds; rounds past the script race for real.
        fn scripting(self, rounds: impl IntoIterator<Item = Vec<RacerOutcome>>) -> Self {
            self.race_script
                .lock()
                .unwrap()
                .extend(rounds.into_iter().flatten());
            self
        }

        fn accepting_stale_preconditions(mut self) -> Self {
            self.accept_stale_precondition = true;
            self
        }

        fn without_create_generation(mut self) -> Self {
            self.zero_generation_on_create = true;
            self
        }

        fn keys(&self) -> Vec<String> {
            let mut keys: Vec<_> = self.objects.lock().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        }

        /// Mint the revision token this provider would report.
        fn revision(&self, generation: i64) -> Revision {
            match self.provider {
                ProviderKind::S3 => Revision::S3Etag(format!("\"{generation}\"")),
                ProviderKind::Gcs => Revision::GcsGeneration(generation),
            }
        }

        /// Read a caller's revision back, rejecting one from another provider
        /// exactly as a real provider does.
        fn generation_of(&self, revision: &Revision) -> Result<i64, ObjectStoreError> {
            match self.provider {
                ProviderKind::S3 => revision
                    .expect_s3_etag()
                    .map(|tag| tag.trim_matches('"').parse().unwrap_or(-1)),
                ProviderKind::Gcs => revision.expect_gcs_generation(),
            }
        }

        fn commit(&self, key: &str, bytes: &[u8]) -> i64 {
            let generation = self.next_generation.fetch_add(1, Ordering::SeqCst);
            self.objects
                .lock()
                .unwrap()
                .insert(key.to_string(), (generation, Bytes::copy_from_slice(bytes)));
            generation
        }

        fn current_generation(&self, key: &str) -> i64 {
            self.objects
                .lock()
                .unwrap()
                .get(key)
                .map(|(generation, _)| *generation)
                .unwrap_or(0)
        }
    }

    #[async_trait]
    impl ObjectStore for ScriptedStore {
        fn provider(&self) -> ProviderKind {
            self.provider
        }

        async fn put(
            &self,
            key: &str,
            bytes: &[u8],
            _content_type: &str,
        ) -> Result<(), ObjectStoreError> {
            self.commit(key, bytes);
            Ok(())
        }

        async fn put_file(
            &self,
            _key: &str,
            _path: &std::path::Path,
            _content_type: &str,
        ) -> Result<(), ObjectStoreError> {
            Err(ObjectStoreError::Provider {
                operation: "put_file",
                message: "unused by the conformance probe".into(),
            })
        }

        async fn put_immutable(
            &self,
            key: &str,
            bytes: &[u8],
            content_type: &str,
        ) -> Result<ImmutableWrite, ObjectStoreError> {
            match self
                .put_conditional(key, bytes, content_type, WriteCondition::Absent)
                .await?
            {
                ConditionalWrite::Committed(_) => Ok(ImmutableWrite::Created),
                ConditionalWrite::Conflict => Ok(ImmutableWrite::AlreadyPresent),
            }
        }

        async fn put_conditional(
            &self,
            key: &str,
            bytes: &[u8],
            _content_type: &str,
            condition: WriteCondition,
        ) -> Result<ConditionalWrite, ObjectStoreError> {
            let expected = match &condition {
                WriteCondition::Absent => 0,
                WriteCondition::Matches(revision) => self.generation_of(revision)?,
            };

            // Only the profile's racing writers are scripted; every other write
            // gets real compare-and-swap semantics, so the phases around the
            // race behave like a conforming store unless a test says otherwise.
            if bytes.starts_with(GCS_RACE_BODY_PREFIX.as_bytes()) {
                let scripted = self.race_script.lock().unwrap().pop_front();
                match scripted {
                    Some(RacerOutcome::Win) => {
                        return Ok(ConditionalWrite::Committed(
                            self.revision(self.commit(key, bytes)),
                        ))
                    }
                    Some(RacerOutcome::WinWithoutGeneration) => {
                        self.commit(key, bytes);
                        return Ok(ConditionalWrite::Committed(Revision::GcsGeneration(0)));
                    }
                    Some(RacerOutcome::Conflict) => return Ok(ConditionalWrite::Conflict),
                    Some(RacerOutcome::Throttle) => {
                        return Err(ObjectStoreError::Throttled {
                            operation: "put_conditional",
                            retry_after: None,
                        })
                    }
                    Some(RacerOutcome::Drop) => {
                        return Err(ObjectStoreError::TransportAmbiguous {
                            operation: "put_conditional",
                            message: "connection reset by peer".into(),
                        })
                    }
                    None => {}
                }
            }

            let current = self.current_generation(key);
            let honour_stale =
                self.accept_stale_precondition && matches!(condition, WriteCondition::Matches(_));
            if current != expected && !honour_stale {
                return Ok(ConditionalWrite::Conflict);
            }
            let generation = self.commit(key, bytes);
            if self.zero_generation_on_create && current == 0 {
                return Ok(ConditionalWrite::Committed(Revision::GcsGeneration(0)));
            }
            Ok(ConditionalWrite::Committed(self.revision(generation)))
        }

        async fn get(&self, key: &str) -> Result<Bytes, ObjectStoreError> {
            self.objects
                .lock()
                .unwrap()
                .get(key)
                .map(|(_, bytes)| bytes.clone())
                .ok_or_else(|| ObjectStoreError::NotFound { key: key.into() })
        }

        async fn get_range(
            &self,
            key: &str,
            start: u64,
            end: u64,
        ) -> Result<Bytes, ObjectStoreError> {
            let bytes = self.get(key).await?;
            Ok(bytes.slice(start as usize..=(end as usize)))
        }

        async fn get_stream(&self, _key: &str) -> Result<ByteStream, ObjectStoreError> {
            Err(ObjectStoreError::Provider {
                operation: "get_stream",
                message: "unused by the conformance probe".into(),
            })
        }

        async fn get_with_revision(
            &self,
            key: &str,
        ) -> Result<Option<(Revision, Bytes)>, ObjectStoreError> {
            let found = self
                .objects
                .lock()
                .unwrap()
                .get(key)
                .map(|(generation, bytes)| (*generation, bytes.clone()));
            Ok(found.map(|(generation, bytes)| (self.revision(generation), bytes)))
        }

        async fn head(&self, key: &str) -> Result<Option<ObjectMeta>, ObjectStoreError> {
            let found = self
                .objects
                .lock()
                .unwrap()
                .get(key)
                .map(|(generation, bytes)| (*generation, bytes.len() as u64));
            Ok(found.map(|(generation, size)| ObjectMeta {
                size,
                revision: Some(self.revision(generation)),
            }))
        }

        async fn list_page(
            &self,
            _prefix: &str,
            _continuation_token: Option<String>,
            _max_keys: usize,
        ) -> Result<ListPage, ObjectStoreError> {
            Ok(ListPage::default())
        }

        async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
            self.objects.lock().unwrap().remove(key);
            Ok(())
        }

        async fn delete_objects(
            &self,
            keys: &[String],
        ) -> Result<BulkDeleteOutcome, ObjectStoreError> {
            let mut objects = self.objects.lock().unwrap();
            let mut outcome = BulkDeleteOutcome::default();
            for key in keys {
                objects.remove(key);
                outcome.deleted += 1;
            }
            Ok(outcome)
        }

        async fn ping(&self) -> Result<(), ObjectStoreError> {
            Ok(())
        }

        async fn versioning_detected(&self) -> Result<bool, ObjectStoreError> {
            Ok(false)
        }
    }

    /// A Cloud Storage profile config with the sleeps taken out, so the tests
    /// that are not about pacing run at memory speed.
    fn unpaced(race_rounds: usize) -> ProbeConfig {
        ProbeConfig {
            race_width: 3,
            race_rounds,
            unproven_round_retries: 3,
            same_key_spacing: Duration::ZERO,
        }
    }

    fn probe_failure(error: StoreError) -> ProbeFailure {
        match error {
            StoreError::Probe(failure) => failure,
            other => panic!("expected a probe failure, got {other:?}"),
        }
    }

    /// Each profile's defaults are the ones the provider can actually answer.
    /// The S3 numbers are unchanged — this profile is already admitted in
    /// production and a conformance gate that quietly narrows is worse than one
    /// that never widened.
    #[test]
    fn profile_defaults_follow_the_provider() {
        let s3 = ProbeConfig::for_provider(ProviderKind::S3);
        assert_eq!(s3, ProbeConfig::default());
        assert_eq!(s3.race_width, 32);
        assert_eq!(s3.race_rounds, 3);
        assert_eq!(s3.same_key_spacing, Duration::ZERO);

        let gcs = ProbeConfig::for_provider(ProviderKind::Gcs);
        assert!(gcs.race_width >= 2 && gcs.race_width < s3.race_width);
        assert!(gcs.race_rounds >= 1);
        assert!(
            gcs.same_key_spacing > Duration::from_secs(1),
            "same-key rounds must be spaced past Cloud Storage's one-write-per-second ceiling"
        );
        assert!(gcs.unproven_round_retries >= 1);
    }

    /// A conforming store passes, and the probe leaves nothing behind.
    #[tokio::test]
    async fn a_conforming_store_is_admitted_and_cleans_up() {
        let backend = Arc::new(ScriptedStore::new(ProviderKind::Gcs));
        let store = GitStore::new(backend.clone());

        let report = store
            .run_conformance_probe(unpaced(2))
            .await
            .expect("a conforming store is admitted");

        assert_eq!(report.profile, ProviderKind::Gcs);
        assert_eq!(report.race_width, 3);
        assert_eq!(report.race_rounds, 2);
        assert_eq!(report.throttled_racers, 0);
        assert_eq!(report.throttled_rounds_retried, 0);
        assert_eq!(report.transport_drops, 0);
        assert_eq!(report.cleanup_failures, 0);
        assert!(
            backend.keys().is_empty(),
            "probe objects left behind: {:?}",
            backend.keys()
        );
    }

    /// The failure the pointer protocol exists to exclude. Two winners is never
    /// a pacing artefact, a retryable round, or a degraded observation — it is
    /// the store admitting a lost update.
    #[tokio::test]
    async fn two_committed_racers_fail_the_probe() {
        let backend = Arc::new(ScriptedStore::new(ProviderKind::Gcs).scripting([vec![
            RacerOutcome::Win,
            RacerOutcome::Win,
            RacerOutcome::Conflict,
        ]]));
        let failure = probe_failure(
            GitStore::new(backend)
                .run_conformance_probe(unpaced(1))
                .await
                .expect_err("two winners on one generation must fail closed"),
        );
        assert_eq!(failure.phase, "cas_race");
        assert!(
            failure.reason.contains("2 racers committed"),
            "unexpected reason: {}",
            failure.reason
        );
    }

    /// A round in which every racer was refused for rate proves nothing: no
    /// precondition was ever evaluated. Scoring it as "no winner" would fail a
    /// conforming store for being paced, so the round is re-run instead.
    #[tokio::test]
    async fn a_fully_throttled_round_is_re_run_rather_than_scored() {
        let backend = Arc::new(
            ScriptedStore::new(ProviderKind::Gcs).scripting([vec![RacerOutcome::Throttle; 3]]),
        );
        let report = GitStore::new(backend)
            .run_conformance_probe(unpaced(1))
            .await
            .expect("a throttled round is re-run, and the re-run proves the race");

        assert_eq!(report.throttled_rounds_retried, 1);
        assert_eq!(report.throttled_racers, 3);
    }

    /// Re-running is bounded. A store that only ever throttles is never
    /// admitted — the probe fails rather than waiting forever or passing on no
    /// evidence.
    #[tokio::test]
    async fn re_runs_are_bounded_and_an_unproven_race_fails_closed() {
        let mut cfg = unpaced(1);
        cfg.unproven_round_retries = 2;
        let backend = Arc::new(ScriptedStore::new(ProviderKind::Gcs).scripting(
            vec![vec![RacerOutcome::Throttle; 3]; cfg.unproven_round_retries + 1],
        ));

        let failure = probe_failure(
            GitStore::new(backend)
                .run_conformance_probe(cfg)
                .await
                .expect_err("a store that only throttles is never admitted"),
        );
        assert_eq!(failure.phase, "cas_race");
        assert!(
            failure
                .reason
                .contains("no round proved a race in 3 attempts"),
            "unexpected reason: {}",
            failure.reason
        );
    }

    /// One winner among a mix of conflicts and throttles is a pass: the
    /// throttled racer is counted, not treated as a loser, and one classified
    /// witness is enough to have seen the race.
    #[tokio::test]
    async fn a_throttled_racer_is_never_a_lost_race() {
        let backend = Arc::new(ScriptedStore::new(ProviderKind::Gcs).scripting([vec![
            RacerOutcome::Win,
            RacerOutcome::Conflict,
            RacerOutcome::Throttle,
        ]]));
        let report = GitStore::new(backend)
            .run_conformance_probe(unpaced(1))
            .await
            .expect("a round with one winner, one conflict and one throttle is proven");

        assert_eq!(report.throttled_racers, 1);
        assert_eq!(report.throttled_rounds_retried, 0);
    }

    /// A commit the store cannot name is unusable: the caller has nothing to
    /// predicate its next write on. Fatal wherever it appears.
    #[tokio::test]
    async fn a_commit_without_a_generation_fails_the_probe() {
        let on_create = Arc::new(ScriptedStore::new(ProviderKind::Gcs).without_create_generation());
        let failure = probe_failure(
            GitStore::new(on_create)
                .run_conformance_probe(unpaced(1))
                .await
                .expect_err("a create with no generation must fail closed"),
        );
        assert_eq!(failure.phase, "pointer_create");
        assert!(
            failure.reason.contains("no object generation"),
            "unexpected reason: {}",
            failure.reason
        );

        let on_race = Arc::new(ScriptedStore::new(ProviderKind::Gcs).scripting([vec![
            RacerOutcome::WinWithoutGeneration,
            RacerOutcome::Conflict,
            RacerOutcome::Conflict,
        ]]));
        let failure = probe_failure(
            GitStore::new(on_race)
                .run_conformance_probe(unpaced(1))
                .await
                .expect_err("a race winner with no generation must fail closed"),
        );
        assert_eq!(failure.phase, "cas_race");
        assert!(
            failure.reason.contains("no object generation"),
            "unexpected reason: {}",
            failure.reason
        );
    }

    /// A store that commits on a superseded generation is doing blind
    /// overwrites, which silently loses pushes. The stale phase is what catches
    /// it, and it must catch it before any race runs.
    #[tokio::test]
    async fn a_store_that_honours_a_stale_generation_fails_the_probe() {
        let backend =
            Arc::new(ScriptedStore::new(ProviderKind::Gcs).accepting_stale_preconditions());
        let failure = probe_failure(
            GitStore::new(backend)
                .run_conformance_probe(unpaced(1))
                .await
                .expect_err("an unenforced precondition must fail closed"),
        );
        assert_eq!(failure.phase, "stale_cas");
        assert!(
            failure.reason.contains("blind overwrite"),
            "unexpected reason: {}",
            failure.reason
        );
    }

    /// Every racer was told the generation had moved, and every outcome was
    /// classified — so the object changed with no writer acknowledged. That is
    /// a lost update announcing itself, not an unproven round: the probe must
    /// not retry its way past it.
    #[tokio::test]
    async fn a_round_where_every_racer_loses_fails_the_probe() {
        let backend = Arc::new(
            ScriptedStore::new(ProviderKind::Gcs).scripting([vec![RacerOutcome::Conflict; 3]]),
        );
        let failure = probe_failure(
            GitStore::new(backend)
                .run_conformance_probe(unpaced(1))
                .await
                .expect_err("conflicts with no acknowledged winner must fail closed"),
        );
        assert_eq!(failure.phase, "cas_race");
        assert!(
            failure
                .reason
                .contains("the object changed without an acknowledged writer"),
            "unexpected reason: {}",
            failure.reason
        );
    }

    /// Every racer's outcome was unknown, so the round is unproven rather than
    /// a failure — the probe admits stores, not networks.
    #[tokio::test]
    async fn transport_drops_leave_the_observer_set_rather_than_failing() {
        let backend = Arc::new(
            ScriptedStore::new(ProviderKind::Gcs).scripting([vec![RacerOutcome::Drop; 3]]),
        );
        let report = GitStore::new(backend)
            .run_conformance_probe(unpaced(1))
            .await
            .expect("a dropped round is re-run");

        assert_eq!(report.transport_drops, 3);
        assert_eq!(report.throttled_rounds_retried, 1);
    }

    /// Pacing is measured, not asserted: the probe sleeps between same-key
    /// mutations and reports the shortest interval it actually observed.
    #[tokio::test]
    async fn same_key_mutations_are_spaced_by_the_configured_interval() {
        let spacing = Duration::from_millis(40);
        let mut cfg = unpaced(2);
        cfg.race_width = 2;
        cfg.same_key_spacing = spacing;

        // create, replace, stale, two race rounds, round-trip: six same-key
        // mutations, so five paced gaps.
        let paced_gaps = 5;
        let backend = Arc::new(ScriptedStore::new(ProviderKind::Gcs));
        let started = Instant::now();
        let report = GitStore::new(backend)
            .run_conformance_probe(cfg)
            .await
            .expect("a conforming store is admitted");
        let elapsed = started.elapsed();

        let observed = report
            .min_same_key_gap
            .expect("a paced profile reports the interval it observed");
        assert!(
            observed >= spacing,
            "shortest observed gap {observed:?} is under the configured {spacing:?}"
        );
        assert!(
            elapsed >= spacing * paced_gaps,
            "the whole probe took {elapsed:?}, less than {paced_gaps} gaps of {spacing:?}"
        );
    }

    /// The provider selects the profile. An S3 store still runs the ETag
    /// profile, unchanged, and reports none of the Cloud Storage counters.
    #[tokio::test]
    async fn an_s3_store_runs_the_s3_profile() {
        let backend = Arc::new(ScriptedStore::new(ProviderKind::S3));
        let report = GitStore::new(backend)
            .run_conformance_probe(ProbeConfig {
                race_width: 3,
                race_rounds: 1,
                ..ProbeConfig::default()
            })
            .await
            .expect("a conforming S3 store is admitted");

        assert_eq!(report.profile, ProviderKind::S3);
        assert_eq!(report.transport_drops, 0);
        assert_eq!(report.throttled_racers, 0);
        assert_eq!(report.min_same_key_gap, None);
    }

    /// The width floor is a property of the gate, not of a profile: one writer
    /// cannot witness a race whichever provider is underneath.
    #[tokio::test]
    async fn a_race_narrower_than_two_writers_is_rejected() {
        for provider in [ProviderKind::S3, ProviderKind::Gcs] {
            let backend = Arc::new(ScriptedStore::new(provider));
            let failure = probe_failure(
                GitStore::new(backend)
                    .run_conformance_probe(ProbeConfig {
                        race_width: 1,
                        race_rounds: 1,
                        ..ProbeConfig::for_provider(provider)
                    })
                    .await
                    .expect_err("a single writer cannot witness a race"),
            );
            assert_eq!(failure.phase, "config");
        }
    }
}

#[cfg(test)]
mod probe {
    //! Empirical probe of the S3 provider's precondition surfacing against
    //! live MinIO.
    //!
    //! Run manually:
    //!   BUZZ_GIT_S3_PROBE=1 cargo test -p buzz-relay --lib \
    //!     api::git::store::probe -- --nocapture --test-threads=1
    //!
    //! Pre-req: `docker compose up minio` and the `buzz-git` bucket exists.

    use super::*;

    fn probe_enabled() -> bool {
        std::env::var("BUZZ_GIT_S3_PROBE").as_deref() == Ok("1")
    }

    fn store() -> GitStore {
        // This is the dedicated backend conformance path, so all connection and
        // signing inputs are overridable for a real provider such as Railway.
        // The hydrate/CAS live tests use explicit local MinIO fixtures instead.
        let endpoint =
            std::env::var("BUZZ_S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".into());
        let access_key = std::env::var("BUZZ_S3_ACCESS_KEY").unwrap_or_else(|_| "buzz_dev".into());
        let secret_key =
            std::env::var("BUZZ_S3_SECRET_KEY").unwrap_or_else(|_| "buzz_dev_secret".into());
        let bucket = std::env::var("BUZZ_S3_BUCKET").unwrap_or_else(|_| "buzz-git".into());
        let region = std::env::var("BUZZ_S3_REGION").unwrap_or_else(|_| "us-east-1".into());
        let addressing_style = std::env::var("BUZZ_S3_ADDRESSING_STYLE")
            .unwrap_or_else(|_| "path".into())
            .parse()
            .expect("BUZZ_S3_ADDRESSING_STYLE must be path or virtual");
        GitStore::from_s3_config(
            &endpoint,
            &access_key,
            &secret_key,
            &bucket,
            &region,
            addressing_style,
        )
        .expect("connect S3-compatible storage")
    }

    fn sha256_hex(b: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(b);
        hex::encode(h.finalize())
    }

    #[tokio::test]
    async fn probe_412_surfacing() {
        if !probe_enabled() {
            eprintln!("skipping: set BUZZ_GIT_S3_PROBE=1 to run against live MinIO");
            return;
        }
        let st = store();
        let key = format!("probe/cas-{}.txt", uuid::Uuid::new_v4());
        let first = st
            .put_immutable_raw(&key, b"first")
            .await
            .expect("first create-only write");
        assert_eq!(first, ImmutableWrite::Created);
        let second = st
            .put_immutable_raw(&key, b"second")
            .await
            .expect("second create-only write must classify, not error");
        assert_eq!(second, ImmutableWrite::AlreadyPresent);
        let _ = st.store.delete(&key).await;
    }

    #[tokio::test]
    async fn probe_full_roundtrip() {
        if !probe_enabled() {
            return;
        }
        let st = store();

        // 1. put_pack returns the content-addressed key; get_verified happy path.
        let bytes = b"hello, git on object store".to_vec();
        let key = st.put_pack(&bytes).await.expect("put_pack");
        assert_eq!(key, format!("packs/{}", sha256_hex(&bytes)));
        let got = st
            .get_verified(&key, &sha256_hex(&bytes))
            .await
            .expect("verified read");
        assert_eq!(&got[..], &bytes[..]);

        // 2. put_pack is idempotent — second call returns the same key.
        let key2 = st.put_pack(&bytes).await.expect("idempotent");
        assert_eq!(key, key2);

        // 3. get_verified detects corruption — wrong expected digest fails.
        let bogus = "0".repeat(64);
        let err = st.get_verified(&key, &bogus).await.unwrap_err();
        assert!(matches!(err, StoreError::DigestMismatch { .. }));

        // 4. pointer lifecycle: get_pointer (None) → put_pointer(Absent)
        //    → get_pointer (Some) → put_pointer(Matches correct)
        //    → put_pointer(Matches stale, Conflict).
        let pkey = format!("pointers/{}.json", uuid::Uuid::new_v4());
        assert!(st.get_pointer(&pkey).await.expect("get none").is_none());

        let p1 = br#"{"manifest":"d1"}"#;
        let r = st
            .put_pointer(&pkey, p1, WriteCondition::Absent)
            .await
            .expect("first cas");
        let r1 = match r {
            ConditionalWrite::Committed(revision) => revision,
            ConditionalWrite::Conflict => panic!("first create-only write should commit"),
        };
        eprintln!("committed revision from PUT response: {r1:?}");

        // Second create-only write must conflict.
        let r = st
            .put_pointer(&pkey, b"{}", WriteCondition::Absent)
            .await
            .expect("second cas");
        assert_eq!(
            r,
            ConditionalWrite::Conflict,
            "second create-only write must conflict"
        );

        // Chain CAS directly on the PUT-returned revision (no HEAD round-trip).
        // MinIO returns the ETag in the PUT response; this proves callers can
        // chain commit → CAS → commit without re-reading the pointer.
        let p2 = br#"{"manifest":"d2"}"#;
        let r = st
            .put_pointer(&pkey, p2, WriteCondition::Matches(r1.clone()))
            .await
            .expect("cas2");
        let r2 = match r {
            ConditionalWrite::Committed(revision) => revision,
            ConditionalWrite::Conflict => panic!("CAS with fresh revision should commit"),
        };

        // Stale CAS (reuse the *first* revision, which has been superseded).
        let r = st
            .put_pointer(&pkey, b"{}", WriteCondition::Matches(r1))
            .await
            .expect("cas3");
        assert_eq!(r, ConditionalWrite::Conflict, "stale CAS must conflict");

        // get_pointer's revision matches the most recent PUT-returned revision.
        let (revision_now, _body) = st.get_pointer(&pkey).await.expect("get").expect("exists");
        assert_eq!(
            revision_now, r2,
            "get_pointer revision matches PUT-response revision"
        );

        // Cleanup.
        let _ = st.store.delete(&pkey).await;
        let _ = st.store.delete(&key).await;
    }

    /// End-to-end conformance probe against MinIO. This is the same code path
    /// that will run at relay startup as a deployment gate.
    #[tokio::test]
    async fn probe_conformance() {
        if !probe_enabled() {
            return;
        }
        let st = store();
        let report = st
            .run_conformance_probe(ProbeConfig {
                race_width: 8,
                race_rounds: 2,
                ..ProbeConfig::for_provider(ProviderKind::S3)
            })
            .await
            .expect("conformance probe");
        eprintln!("✓ probe report: {report:?}");
        assert_eq!(report.race_width, 8);
        assert_eq!(report.race_rounds, 2);
    }

    /// Quick probe: confirm a plain read exposes the object's revision.
    #[tokio::test]
    async fn probe_get_exposes_revision() {
        if !probe_enabled() {
            return;
        }
        let st = store();
        let key = format!("probe/revision-{}.txt", uuid::Uuid::new_v4());
        st.store.put(&key, b"hi", "text/plain").await.expect("put");
        let observed = st.get_pointer(&key).await.expect("get").expect("exists");
        eprintln!("revision from GET: {:?}", observed.0);
        let _ = st.store.delete(&key).await;
    }
}
