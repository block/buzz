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

use buzz_object_store::{
    ConditionalWrite, ImmutableWrite, ObjectStore, ObjectStoreError, Revision, S3AddressingStyle,
    S3ObjectStore, S3StoreConfig, WriteCondition,
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
/// Defaults: 32-way concurrency, 3 rounds. The probe is a deployment gate —
/// run at startup, fail-closed. See `docs/git-on-object-storage.md` §Conformance.
#[derive(Debug, Clone)]
pub struct ProbeConfig {
    /// How many tasks race per round. Must be ≥ 2.
    pub race_width: usize,
    /// How many rounds to run each race phase.
    pub race_rounds: usize,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            race_width: 32,
            race_rounds: 3,
        }
    }
}

/// Returned on a successful probe run. Kept intentionally thin — failure
/// detail lives in `ProbeFailure` (the error variant).
#[derive(Debug, Clone)]
pub struct ProbeReport {
    /// Concurrency used.
    pub race_width: usize,
    /// Rounds executed per race phase.
    pub race_rounds: usize,
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
    /// One of `sequential`, `if_match_race`, `if_none_match_race`, `revision_consistency`.
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
    pub async fn run_conformance_probe(&self, cfg: ProbeConfig) -> Result<ProbeReport, StoreError> {
        use std::sync::Arc;
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
            race_width: cfg.race_width,
            race_rounds: cfg.race_rounds,
            transport_drops,
        })
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
