//! Object-store backend for git-on-object-storage.
//!
//! Implements the create-only, content-addressed write discipline (axiom A1)
//! and the CAS pointer swap (axiom A3) described in
//! `docs/git-on-object-storage.md`.
//!
//! ## The 412 sharp edge
//!
//! `rust-s3 = "0.37"` is shared across the workspace with `sprout-media`. The
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
/// in `probe::probe_full_roundtrip`). If a backend ever omits the response
/// ETag, the value will be empty and the next `IfMatch(empty)` will be
/// rejected as a normal `LostRace` — no silent corruption, only a forced
/// retry via `get_pointer`.
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
}

/// Object-store client for git refs.
pub struct GitStore {
    bucket: Box<Bucket>,
}

impl GitStore {
    /// Build a client against an S3-compatible endpoint (e.g. MinIO).
    ///
    /// Uses path-style addressing for MinIO compatibility; AWS S3 accepts both.
    pub fn new(
        endpoint: &str,
        access_key: &str,
        secret_key: &str,
        bucket_name: &str,
    ) -> Result<Self, StoreError> {
        let region = Region::Custom {
            region: "us-east-1".into(),
            endpoint: endpoint.into(),
        };
        let creds = Credentials::new(Some(access_key), Some(secret_key), None, None, None)
            .map_err(|e| StoreError::Backend(S3Error::Credentials(e)))?;
        let bucket = Bucket::new(bucket_name, region, creds)
            .map_err(StoreError::Backend)?
            .with_path_style();
        Ok(Self { bucket })
    }

    /// Create-only write of a content-addressed object (pack or manifest).
    ///
    /// Idempotent: a collision on the same content-addressed key is treated as
    /// success, because the bytes are guaranteed to match (the key is their
    /// digest). This is the §Push step 2 / step 6 primitive.
    pub async fn put_immutable(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<(), StoreError> {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::IF_NONE_MATCH, "*".parse().unwrap());
        match self
            .bucket
            .put_object_with_content_type_and_headers(key, bytes, content_type, Some(headers))
            .await
        {
            Ok(resp) if (200..300).contains(&resp.status_code()) => Ok(()),
            // 412 on a content-addressed key means the key already holds the same
            // bytes (by construction). Treat as success — A1 is preserved.
            Err(S3Error::HttpFailWithBody(412, _)) => Ok(()),
            Ok(resp) => Err(StoreError::Backend(S3Error::HttpFailWithBody(
                resp.status_code(),
                "unexpected status".into(),
            ))),
            Err(e) => Err(StoreError::Backend(e)),
        }
    }

    /// `put_immutable` for a pack object (content-type `application/x-git-pack`).
    pub async fn put_pack(&self, key: &str, bytes: &[u8]) -> Result<(), StoreError> {
        self.put_immutable(key, bytes, "application/x-git-pack")
            .await
    }

    /// `put_immutable` for a manifest object (JSON).
    pub async fn put_manifest(&self, key: &str, bytes: &[u8]) -> Result<(), StoreError> {
        self.put_immutable(key, bytes, "application/json").await
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

    /// GET the pointer object, returning its ETag and bytes.
    ///
    /// Returns `Ok(None)` if the pointer does not exist (first-push case).
    pub async fn get_pointer(&self, key: &str) -> Result<Option<(ETag, Bytes)>, StoreError> {
        // `get_object` does not surface the response ETag in 0.37; do a HEAD first
        // to capture the ETag, then GET. We accept the extra round-trip — pointer
        // objects are tiny and the read path already pays one network hop for the
        // manifest after this. Alternative would be `request_with_url`-level access,
        // which entangles us with rust-s3 internals.
        let head = match self.bucket.head_object(key).await {
            Ok((info, status)) if (200..300).contains(&status) => info,
            Ok((_, 404)) => return Ok(None),
            Err(S3Error::HttpFailWithBody(404, _)) => return Ok(None),
            Ok((_, status)) => {
                return Err(StoreError::Backend(S3Error::HttpFailWithBody(
                    status,
                    "unexpected head status".into(),
                )))
            }
            Err(e) => return Err(StoreError::Backend(e)),
        };
        let etag = head
            .e_tag
            .ok_or_else(|| StoreError::Backend(S3Error::HttpFail))?;
        let bytes = self.get(key).await?;
        Ok(Some((ETag(etag), bytes)))
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
                    .unwrap_or_default();
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

#[cfg(test)]
mod probe {
    //! Empirical probe of rust-s3 + `fail-on-err` + MinIO surfacing of 412.
    //!
    //! Run manually:
    //!   SPROUT_GIT_S3_PROBE=1 cargo test -p sprout-relay --lib \
    //!     api::git::store::probe -- --nocapture --test-threads=1
    //!
    //! Pre-req: `docker compose up minio` and the `sprout-git` bucket exists.

    use super::*;

    fn probe_enabled() -> bool {
        std::env::var("SPROUT_GIT_S3_PROBE").as_deref() == Ok("1")
    }

    fn store() -> GitStore {
        GitStore::new(
            "http://localhost:9000",
            "sprout_dev",
            "sprout_dev_secret",
            "sprout-git",
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
            eprintln!("skipping: set SPROUT_GIT_S3_PROBE=1 to run against live MinIO");
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

        // 1. put_immutable + get_verified happy path.
        let bytes = b"hello, git on object store".to_vec();
        let key = format!("packs/{}", sha256_hex(&bytes));
        st.put_pack(&key, &bytes).await.expect("put_pack");
        let got = st
            .get_verified(&key, &sha256_hex(&bytes))
            .await
            .expect("verified read");
        assert_eq!(&got[..], &bytes[..]);

        // 2. put_immutable is idempotent (no error on second call).
        st.put_pack(&key, &bytes).await.expect("idempotent");

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
}
