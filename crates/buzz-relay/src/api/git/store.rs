//! Object-store backend for git-on-object-storage.
//!
//! Implements the create-only, content-addressed write discipline (axiom A1)
//! and the CAS pointer swap (axiom A3) described in
//! `docs/git-on-object-storage.md`.
//!
//! ## The 412 sharp edge
//!
//! `rust-s3 = "0.37"` is shared across the workspace with `buzz-media`. The
//! `fail-on-err` Cargo feature is unified ON across the build graph, which
//! means non-2xx responses arrive here as `S3Error::HttpFailWithBody(code,
//! body)` *before* the caller sees `ResponseData`. The pointer-CAS path treats
//! the precondition-failure status (412) as a *semantic* result (`LostRace`),
//! not an error — see `classify_cas`. Empirically verified against MinIO in
//! `probe::probe_412_surfacing`.
//!
//! ## Content addressing (A1)
//!
//! Pack and manifest keys are the SHA-256 of their bytes. Writes use
//! `If-None-Match: *` so the same key is never overwritten. Readers verify
//! object bytes against the expected digest on `get_verified`; any mismatch is
//! *detectable*, not silent — that is what A1's "create-only + content-address"
//! discipline buys us, independent of bucket immutability features.

#![allow(dead_code)] // wired in by the push path in a follow-up commit

use std::sync::Arc;

use bytes::Bytes;
use s3::creds::Credentials;
use s3::error::S3Error;
use s3::{Bucket, Region};
use sha2::{Digest, Sha256};

/// Opaque object-store ETag (used for `If-Match` on pointer CAS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ETag(pub String);

/// Precondition for `put_pointer`.
#[derive(Debug, Clone)]
pub enum Precond {
    /// Create-only: succeed iff the pointer does not yet exist.
    IfNoneMatchStar,
    /// CAS: succeed iff the current ETag matches.
    IfMatch(ETag),
}

/// Result of a CAS pointer write.
///
/// `LostRace` is *not* an error — it is the standard outcome of a losing CAS
/// and must be classified here so callers can decide retry vs. non-ff. On
/// `Won`, the returned `ETag` is the PUT response's ETag and can be fed
/// directly into the next `IfMatch` round (verified empirically against MinIO
/// in `probe::probe_full_roundtrip`). A backend that succeeds on the CAS PUT
/// but omits the response ETag is treated as non-conforming and fails the
/// operation with `StoreError::Backend` — see `classify_cas`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CasOutcome {
    /// CAS succeeded; the new pointer ETag (suitable for the next `IfMatch`).
    Won(ETag),
    /// CAS lost the race (server returned 412).
    LostRace,
}

/// Errors that are *actually* errors — `LostRace` is not one.
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
    #[error("s3 backend error: {0}")]
    Backend(#[from] S3Error),
    /// Invalid storage configuration detected at client construction.
    #[error("git store config error: {0}")]
    Config(String),
    /// Conformance probe failed — backend does not satisfy A1/A2/A3.
    #[error(transparent)]
    Probe(ProbeFailure),
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
    /// `S3Error::{Reqwest, Http, Io}` — that means the racer never got a
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
    /// One of `sequential`, `if_match_race`, `if_none_match_race`, `etag_consistency`.
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
    bucket: Arc<Bucket>,
}

impl GitStore {
    /// Build a client against an S3-compatible endpoint (e.g. MinIO).
    ///
    /// Uses path-style addressing for MinIO compatibility; AWS S3 accepts both.
    ///
    /// Credential selection mirrors [`buzz_media::MediaStorage::new`]:
    /// - both `access_key` and `secret_key` non-empty → static credentials
    ///   (MinIO/local/dev, or any static-key deployment);
    /// - both empty → the AWS default credential chain via
    ///   [`Credentials::default`] (env, profile, web-identity/IRSA, container,
    ///   instance metadata), so the relay can use its pod IAM role;
    /// - exactly one empty → a config error, to surface a half-configured
    ///   static deployment instead of silently falling back to the chain.
    pub fn new(
        endpoint: &str,
        access_key: &str,
        secret_key: &str,
        bucket_name: &str,
        region: &str,
    ) -> Result<Self, StoreError> {
        let region = Region::Custom {
            region: region.into(),
            endpoint: endpoint.into(),
        };
        let creds = match (access_key.is_empty(), secret_key.is_empty()) {
            (false, false) => Credentials::new(Some(access_key), Some(secret_key), None, None, None),
            (true, true) => {
                // No static keys configured: resolve from the AWS credential
                // chain (IRSA web-identity, env, profile, instance metadata).
                Credentials::default()
            }
            _ => {
                return Err(StoreError::Config(
                    "s3 access_key and secret_key must be configured together, or both empty to use the AWS credential chain".to_string(),
                ));
            }
        }
        .map_err(|e| StoreError::Backend(S3Error::Credentials(e)))?;
        let bucket = Bucket::new(bucket_name, region, creds)
            .map_err(StoreError::Backend)?
            .with_path_style();
        Ok(Self {
            bucket: Arc::from(bucket),
        })
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
            return Err(StoreError::Backend(S3Error::HttpFailWithBody(
                400,
                format!("invalid pack digest for idx sidecar: {pack_digest:?}"),
            )));
        }
        Ok(format!("idx/{pack_digest}"))
    }

    /// Create-only write of a content-addressed object (pack or manifest).
    ///
    /// **The caller does not choose the key.** It is derived as
    /// `<prefix>/<hex sha256(bytes)>` inside this method. This makes the
    /// idempotency claim *constructive*: a 412 collision means the key already
    /// holds bytes whose digest equals `sha256(these bytes)`, so by A1
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
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::IF_NONE_MATCH, "*".parse().unwrap());
        match self
            .bucket
            .put_object_with_content_type_and_headers(&key, bytes, content_type, Some(headers))
            .await
        {
            Ok(resp) if (200..300).contains(&resp.status_code()) => Ok(key),
            // 412 on a content-addressed key means the key already holds the
            // same bytes (by construction — the key is the digest). A1 is
            // preserved without a defensive GET.
            Err(S3Error::HttpFailWithBody(412, _)) => Ok(key),
            Ok(resp) => Err(StoreError::Backend(S3Error::HttpFailWithBody(
                resp.status_code(),
                "unexpected status".into(),
            ))),
            Err(e) => Err(StoreError::Backend(e)),
        }
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
    /// bytes. A 412 is idempotent success for the cache layer: the first writer
    /// already produced the sidecar for this pack, and hydrate validates before
    /// trusting it.
    pub async fn put_idx(&self, pack_digest: &str, idx_bytes: &[u8]) -> Result<String, StoreError> {
        let key = Self::idx_key_for_pack_digest(pack_digest)?;
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::IF_NONE_MATCH, "*".parse().unwrap());
        match self
            .bucket
            .put_object_with_content_type_and_headers(
                &key,
                idx_bytes,
                "application/x-git-index",
                Some(headers),
            )
            .await
        {
            Ok(resp) if (200..300).contains(&resp.status_code()) => Ok(key),
            Err(S3Error::HttpFailWithBody(412, _)) => Ok(key),
            Ok(resp) => Err(StoreError::Backend(S3Error::HttpFailWithBody(
                resp.status_code(),
                "unexpected status".into(),
            ))),
            Err(e) => Err(StoreError::Backend(e)),
        }
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
        match self.bucket.get_object(key).await {
            Ok(resp) => Ok(Bytes::from(resp.to_vec())),
            Err(S3Error::HttpFailWithBody(404, _)) => Err(StoreError::NotFound(key.into())),
            Err(e) => Err(StoreError::Backend(e)),
        }
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

    /// GET an immutable object after rejecting objects larger than `max_bytes`.
    ///
    /// Pack and manifest objects are content addressed and create-only, so the
    /// HEAD result cannot race with a different body at the same key. The
    /// second length check protects against a backend that reports a bad
    /// Content-Length header.
    pub async fn get_verified_limited(
        &self,
        key: &str,
        expected_digest: &str,
        max_bytes: u64,
    ) -> Result<Bytes, StoreError> {
        let bytes = self.get_limited(key, max_bytes).await?;
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

    /// GET an object after rejecting bodies larger than `max_bytes`.
    pub async fn get_limited(&self, key: &str, max_bytes: u64) -> Result<Bytes, StoreError> {
        let (head, status) = self.bucket.head_object(key).await.map_err(|e| match e {
            S3Error::HttpFailWithBody(404, _) => StoreError::NotFound(key.into()),
            other => StoreError::Backend(other),
        })?;
        if status == 404 {
            return Err(StoreError::NotFound(key.into()));
        }
        if !(200..300).contains(&status) {
            return Err(StoreError::Backend(S3Error::HttpFailWithBody(
                status,
                "unexpected status".into(),
            )));
        }
        if let Some(content_length) = head.content_length {
            let size = u64::try_from(content_length).unwrap_or(u64::MAX);
            if size > max_bytes {
                return Err(StoreError::ObjectTooLarge {
                    key: key.into(),
                    size,
                    max: max_bytes,
                });
            }
        }

        let bytes = self.get(key).await?;
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if size > max_bytes {
            return Err(StoreError::ObjectTooLarge {
                key: key.into(),
                size,
                max: max_bytes,
            });
        }
        Ok(bytes)
    }

    /// GET the pointer object, returning its ETag and bytes *from the same
    /// response* — atomic snapshot.
    ///
    /// Returns `Ok(None)` if the pointer does not exist (first-push case).
    ///
    /// **Why one GET, not HEAD-then-GET.** A separate HEAD followed by GET
    /// can straddle a concurrent writer: the HEAD's ETag and the GET's body
    /// would describe different pointer versions, and a caller that later
    /// did `IfMatch(etag_from_head)` would be predicating on a version it
    /// never actually read. Reading both fields from the GET response keeps
    /// the snapshot consistent (A2: a single GET observes a single committed
    /// object). Verified empirically in `probe::probe_get_exposes_etag`.
    pub async fn get_pointer(&self, key: &str) -> Result<Option<(ETag, Bytes)>, StoreError> {
        match self.bucket.get_object(key).await {
            Ok(resp) => {
                let headers = resp.headers();
                let etag = headers
                    .get("etag")
                    .or_else(|| headers.get("ETag"))
                    .cloned()
                    .ok_or_else(|| {
                        StoreError::Backend(S3Error::HttpFailWithBody(
                            500,
                            "GET pointer: response missing ETag".into(),
                        ))
                    })?;
                Ok(Some((ETag(etag), Bytes::from(resp.to_vec()))))
            }
            Err(S3Error::HttpFailWithBody(404, _)) => Ok(None),
            Err(e) => Err(StoreError::Backend(e)),
        }
    }

    /// Write the pointer under a precondition (§Push step 7 — the CAS).
    ///
    /// Returns `CasOutcome::LostRace` on 412 (the standard losing outcome).
    /// On `CasOutcome::Won`, the returned `ETag` is read from the response
    /// headers — callers use it as the `If-Match` value for the next CAS.
    pub async fn put_pointer(
        &self,
        key: &str,
        body: &[u8],
        precond: Precond,
    ) -> Result<CasOutcome, StoreError> {
        let mut headers = axum::http::HeaderMap::new();
        match &precond {
            Precond::IfNoneMatchStar => {
                headers.insert(axum::http::header::IF_NONE_MATCH, "*".parse().unwrap());
            }
            Precond::IfMatch(ETag(tag)) => {
                headers.insert(
                    axum::http::header::IF_MATCH,
                    tag.parse().map_err(|_| {
                        StoreError::Backend(S3Error::HttpFailWithBody(
                            400,
                            format!("invalid etag {tag}"),
                        ))
                    })?,
                );
            }
        }
        let result = self
            .bucket
            .put_object_with_content_type_and_headers(key, body, "application/json", Some(headers))
            .await;
        Self::classify_cas(result)
    }

    /// Map a rust-s3 PUT outcome to a `CasOutcome`.
    ///
    /// 412 → `LostRace`. 2xx → `Won(etag)` (etag read from response headers,
    /// empty if missing — callers must tolerate empty etag and re-HEAD if they
    /// need it strictly). Everything else bubbles as `StoreError::Backend`.
    fn classify_cas(
        result: Result<s3::request::ResponseData, S3Error>,
    ) -> Result<CasOutcome, StoreError> {
        match result {
            Ok(resp) if (200..300).contains(&resp.status_code()) => {
                let headers = resp.headers();
                let etag = headers
                    .get("etag")
                    .or_else(|| headers.get("ETag"))
                    .cloned()
                    .ok_or_else(|| {
                        // Fail closed: a CAS that we can't chain (because the
                        // backend didn't return an ETag) is not a `Won` — it's
                        // a non-conforming backend. The conformance probe will
                        // catch this; in production we'd rather refuse than
                        // hand the caller `ETag("")` and force-fail the next CAS.
                        StoreError::Backend(S3Error::HttpFailWithBody(
                            resp.status_code(),
                            "CAS succeeded but response missing ETag header \
                             (backend does not satisfy ETag-token consistency)"
                                .into(),
                        ))
                    })?;
                Ok(CasOutcome::Won(ETag(etag)))
            }
            Err(S3Error::HttpFailWithBody(412, _)) => Ok(CasOutcome::LostRace),
            Ok(resp) => Err(StoreError::Backend(S3Error::HttpFailWithBody(
                resp.status_code(),
                "unexpected status".into(),
            ))),
            Err(e) => Err(StoreError::Backend(e)),
        }
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
    ///    predicated on the same ETag. Exactly one must `Won`; the rest must
    ///    `LostRace`. Tests A3.
    /// 3. **`if_none_match_race`** — `race_width` parallel create-only writes
    ///    against the same digest-shaped key (the same `put_immutable` path
    ///    `put_pack`/`put_manifest` use). Tests A1 + A3 on the create-only
    ///    primitive. Counts raw HTTP outcomes (exactly one 2xx, rest 412) and
    ///    asserts final stored bytes equal the racers' bytes.
    /// 4. **`etag_consistency`** — round-trip an ETag from `get_pointer` into
    ///    `put_pointer(IfMatch(...))` and assert `Won`. Tests that the token
    ///    is opaque and stable between read and CAS.
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
        // Seed the pointer with a known value, then race N IfMatch updates.
        let seed = b"probe-pointer-seed".to_vec();
        let _ = self.bucket.delete_object(&pointer_key).await; // ignore 404
        let seed_outcome = self
            .put_pointer(&pointer_key, &seed, Precond::IfNoneMatchStar)
            .await?;
        let mut etag = match seed_outcome {
            CasOutcome::Won(e) => e,
            CasOutcome::LostRace => {
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
                let et = etag.clone();
                let body = format!("round={round},racer={i},nonce={nonce}").into_bytes();
                tasks.push(async move { me.put_pointer(&pkey, &body, Precond::IfMatch(et)).await });
            }
            let outcomes = futures_util::future::join_all(tasks).await;
            // Drop-and-floor classification. A `Reqwest`/`Http`/`Io` error
            // means the racer never got a classified response from the
            // backend (couldn't open a socket, send flaked, etc.); its
            // outcome is *unknown*, not negative. A3 is a claim about
            // **observers**: dropping unknowns from the observer set
            // sharpens the assertion ("exactly one winner among observers")
            // and avoids smuggling a network-stack test into the
            // conformance probe. Parse/decode errors (`Utf8`,
            // `ReqwestHeaderToStr`, `SerdeXml`, ...) and `HttpFailWithBody`
            // stay in the catch-all — those mean the backend *did* answer
            // but not in the contract shape, which is a real conformance
            // signal.
            let mut classified = 0usize;
            let mut winners = 0usize;
            let mut new_etag: Option<ETag> = None;
            for (i, outcome) in outcomes.into_iter().enumerate() {
                match outcome {
                    Ok(CasOutcome::Won(e)) => {
                        classified += 1;
                        winners += 1;
                        new_etag = Some(e);
                    }
                    Ok(CasOutcome::LostRace) => {
                        classified += 1;
                    }
                    Err(StoreError::Backend(
                        S3Error::Reqwest(_) | S3Error::Http(_) | S3Error::Io(_),
                    )) => {
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
            etag = new_etag.expect("winner exists");
        }

        // -- Phase 3: if_none_match_race ------------------------------------------
        // N parallel create-only writes targeting the same digest-shaped key.
        // Bypass `put_immutable`'s 412-swallow to count raw outcomes.
        for round in 0..cfg.race_rounds {
            let body = format!("probe-inm-race-{nonce}-{round}").into_bytes();
            let key = Self::content_key("probe/inm-race", &body);
            // Clean slate.
            let _ = self.bucket.delete_object(&key).await;
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
            // `Reqwest`/`Http`/`Io` (pre-classification — socket/send
            // failure); count 2xx + 412 as the classified observers. Any
            // other status or any non-transport `StoreError` is a real
            // conformance signal and fails closed.
            let mut classified = 0usize;
            let mut twos = 0usize;
            let mut twelves = 0usize;
            for (i, r) in results.into_iter().enumerate() {
                match r {
                    Ok(200..=299) => {
                        classified += 1;
                        twos += 1;
                    }
                    Ok(412) => {
                        classified += 1;
                        twelves += 1;
                    }
                    Ok(code) => {
                        return Err(ProbeFailure {
                            phase: "if_none_match_race",
                            round,
                            key,
                            reason: format!("racer {i}: unexpected status {code}"),
                        }
                        .into())
                    }
                    Err(StoreError::Backend(
                        S3Error::Reqwest(_) | S3Error::Http(_) | S3Error::Io(_),
                    )) => {
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
            // Create-only contract: exactly 1×2xx + (classified − 1)×412
            // *among observers*. The previous fixed `race_width − 1` would
            // false-positive on any transport drop; this expression honors
            // the drop-and-floor invariant.
            if twos != 1 || twelves != classified - 1 {
                return Err(ProbeFailure {
                    phase: "if_none_match_race",
                    round,
                    key,
                    reason: format!(
                        "expected 1×2xx + {}×412 among {classified} classified observers, got {twos}×2xx + {twelves}×412",
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

        // -- Phase 4: etag_consistency --------------------------------------------
        // GET pointer, take its ETag, CAS-update with that ETag, expect Won.
        // Proves the token round-trips opaquely between read and write.
        for round in 0..cfg.race_rounds {
            let (et, _bytes) =
                self.get_pointer(&pointer_key)
                    .await?
                    .ok_or_else(|| ProbeFailure {
                        phase: "etag_consistency",
                        round,
                        key: pointer_key.clone(),
                        reason: "pointer vanished mid-probe".into(),
                    })?;
            let body = format!("probe-etag-{round}-{nonce}").into_bytes();
            match self
                .put_pointer(&pointer_key, &body, Precond::IfMatch(et))
                .await?
            {
                CasOutcome::Won(_) => {}
                CasOutcome::LostRace => {
                    return Err(ProbeFailure {
                        phase: "etag_consistency",
                        round,
                        key: pointer_key,
                        reason: "GET-ETag → IfMatch chain lost race in a quiescent probe".into(),
                    }
                    .into())
                }
            }
        }

        // Cleanup pointer (immutable probe writes accumulate by design; the
        // bucket's retention policy handles them, not the probe).
        let _ = self.bucket.delete_object(&pointer_key).await;

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
    /// we need to *see* 412 outcomes rather than swallow them as idempotent.
    /// Returns the HTTP status code on success-or-412; bubbles other errors.
    async fn put_immutable_raw(&self, key: &str, bytes: &[u8]) -> Result<u16, StoreError> {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::IF_NONE_MATCH, "*".parse().unwrap());
        match self
            .bucket
            .put_object_with_content_type_and_headers(
                key,
                bytes,
                "application/octet-stream",
                Some(headers),
            )
            .await
        {
            Ok(resp) => Ok(resp.status_code()),
            Err(S3Error::HttpFailWithBody(412, _)) => Ok(412),
            Err(e) => Err(StoreError::Backend(e)),
        }
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

    #[test]
    fn classify_cas_412_is_lost_race() {
        let r = Err(S3Error::HttpFailWithBody(412, "PreconditionFailed".into()));
        assert_eq!(GitStore::classify_cas(r).unwrap(), CasOutcome::LostRace);
    }

    #[test]
    fn classify_cas_other_4xx_bubbles() {
        let r = Err(S3Error::HttpFailWithBody(403, "AccessDenied".into()));
        assert!(matches!(
            GitStore::classify_cas(r),
            Err(StoreError::Backend(S3Error::HttpFailWithBody(403, _)))
        ));
    }

    #[test]
    fn static_keys_build_store_with_configured_region() {
        let store = GitStore::new(
            "http://localhost:9000",
            "buzz_dev",
            "buzz_dev_secret",
            "buzz-git",
            "us-west-2",
        )
        .expect("static creds should build a git store");
        match store.bucket.region {
            Region::Custom { ref region, .. } => assert_eq!(region, "us-west-2"),
            ref other => panic!("expected Custom region, got {other:?}"),
        }
    }

    #[test]
    fn partial_static_keys_are_rejected() {
        for (access, secret) in [("buzz_dev", ""), ("", "buzz_dev_secret")] {
            let err = match GitStore::new(
                "http://localhost:9000",
                access,
                secret,
                "buzz-git",
                "us-east-1",
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
    //! Empirical probe of rust-s3 + `fail-on-err` + MinIO surfacing of 412.
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
        GitStore::new(
            "http://localhost:9000",
            "buzz_dev",
            "buzz_dev_secret",
            "buzz-git",
            "us-east-1",
        )
        .expect("connect minio")
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
        let mut hdrs = axum::http::HeaderMap::new();
        hdrs.insert(axum::http::header::IF_NONE_MATCH, "*".parse().unwrap());
        let r1 = st
            .bucket
            .put_object_with_content_type_and_headers(
                &key,
                b"first",
                "text/plain",
                Some(hdrs.clone()),
            )
            .await;
        assert!((200..300).contains(&r1.expect("first ok").status_code()));
        let r2 = st
            .bucket
            .put_object_with_content_type_and_headers(&key, b"second", "text/plain", Some(hdrs))
            .await;
        assert!(matches!(r2, Err(S3Error::HttpFailWithBody(412, _))));
        let _ = st.bucket.delete_object(&key).await;
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

        // 4. pointer lifecycle: get_pointer (None) → put_pointer(IfNoneMatchStar)
        //    → get_pointer (Some) → put_pointer(IfMatch correct) → put_pointer(IfMatch wrong, LostRace).
        let pkey = format!("pointers/{}.json", uuid::Uuid::new_v4());
        assert!(st.get_pointer(&pkey).await.expect("get none").is_none());

        let p1 = br#"{"manifest":"d1"}"#;
        let r = st
            .put_pointer(&pkey, p1, Precond::IfNoneMatchStar)
            .await
            .expect("first cas");
        let e1 = match r {
            CasOutcome::Won(e) => e,
            CasOutcome::LostRace => panic!("first INM* should win"),
        };
        eprintln!("Won.etag from PUT response: {:?}", e1.0);

        // Second INM* must lose.
        let r = st
            .put_pointer(&pkey, b"{}", Precond::IfNoneMatchStar)
            .await
            .expect("second cas");
        assert_eq!(r, CasOutcome::LostRace, "second INM* must lose");

        // Chain CAS directly on the PUT-returned ETag (no HEAD round-trip).
        // MinIO returns the ETag in the PUT response; this proves callers can
        // chain `Won → IfMatch → Won` without re-reading the pointer.
        assert!(!e1.0.is_empty(), "MinIO should populate PUT response ETag");
        let p2 = br#"{"manifest":"d2"}"#;
        let r = st
            .put_pointer(&pkey, p2, Precond::IfMatch(e1.clone()))
            .await
            .expect("cas2");
        let e2 = match r {
            CasOutcome::Won(e) => e,
            CasOutcome::LostRace => panic!("IfMatch with fresh etag should win"),
        };

        // Stale IfMatch (reuse the *first* etag, which has been superseded) → LostRace.
        let r = st
            .put_pointer(&pkey, b"{}", Precond::IfMatch(e1))
            .await
            .expect("cas3");
        assert_eq!(r, CasOutcome::LostRace, "stale IfMatch must lose");

        // get_pointer's etag matches the most recent PUT-returned etag.
        let (etag_now, _body) = st.get_pointer(&pkey).await.expect("get").expect("exists");
        assert_eq!(etag_now, e2, "get_pointer etag matches PUT-response etag");

        // Cleanup.
        let _ = st.bucket.delete_object(&pkey).await;
        let _ = st.bucket.delete_object(&key).await;
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

    /// Quick probe: confirm rust-s3's `get_object` exposes ETag on the response.
    #[tokio::test]
    async fn probe_get_exposes_etag() {
        if !probe_enabled() {
            return;
        }
        let st = store();
        let key = format!("probe/etag-{}.txt", uuid::Uuid::new_v4());
        st.bucket
            .put_object_with_content_type(&key, b"hi", "text/plain")
            .await
            .expect("put");
        let resp = st.bucket.get_object(&key).await.expect("get");
        let headers = resp.headers();
        eprintln!("GET headers: {headers:?}");
        let etag = headers.get("etag").or_else(|| headers.get("ETag")).cloned();
        assert!(etag.is_some(), "GET response must carry ETag header");
        eprintln!("ETag from GET: {etag:?}");
        let _ = st.bucket.delete_object(&key).await;
    }
}
