//! Provider-neutral object storage for Buzz.
//!
//! Buzz keeps two very different workloads on one bucket: media blobs
//! (`buzz-media`) and the content-addressed Git object store with its
//! compare-and-swap ref pointer (`buzz-relay`'s `api::git::store`). Both used
//! to hold their own `rust-s3` client and speak in S3-shaped vocabulary —
//! ETags, `If-Match`, `If-None-Match: *`. That made the storage provider a
//! property of every call site rather than a property of the deployment.
//!
//! This crate is the seam. It owns:
//!
//! - the [`ObjectStore`] trait — exactly the operations Buzz performs, no more;
//! - provider-safe [`Revision`] / [`WriteCondition`] / [`ConditionalWrite`]
//!   types, so a compare-and-swap token is never a bare string with implied
//!   semantics;
//! - the [`ObjectStoreError`] taxonomy, which separates a *classified*
//!   provider answer from an *unknown* transport outcome;
//! - the S3 provider ([`providers::s3`]), which is the only place an ETag
//!   exists, and the Google Cloud Storage provider ([`providers::gcs`]), which
//!   is the only place an object generation exists.
//!
//! Domain code above this seam — `MediaStorage`, `GitStore` — is a thin facade
//! that adds Buzz semantics (tenant-scoped sidecar keys, content addressing,
//! digest verification) and never names a provider.

pub mod error;
pub mod providers;
pub mod revision;

use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;

pub use error::ObjectStoreError;
pub use providers::gcs::{GcsObjectStore, GcsRetryConfig, GcsStoreConfig};
pub use providers::s3::{S3AddressingStyle, S3ObjectStore, S3StoreConfig};
pub use revision::{ConditionalWrite, ProviderKind, Revision, WriteCondition};

/// Which provider a deployment selects, before its settings are resolved.
///
/// Split from [`ObjectStoreConfig`] because the two callers that build a store
/// — the relay and the deletion tool — have different defaults for the S3
/// settings but must agree on how the provider itself is chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderSelection {
    /// S3 or an S3-compatible backend, configured by the `BUZZ_S3_*` settings.
    S3,
    /// Google Cloud Storage, authenticated with Application Default
    /// Credentials and addressed by bucket name alone.
    Gcs {
        /// Bucket name from `BUZZ_OBJECT_STORE_BUCKET`.
        bucket: String,
    },
}

impl ProviderSelection {
    /// Read the provider selection from the environment.
    ///
    /// `BUZZ_OBJECT_STORE_PROVIDER` defaults to `s3`, so a deployment that has
    /// never heard of this variable keeps its existing behavior. Selecting
    /// `gcs` requires `BUZZ_OBJECT_STORE_BUCKET`: a Cloud Storage deployment
    /// sets no `BUZZ_S3_*` values at all, so there is no bucket to fall back
    /// to and guessing one would address the wrong data.
    pub fn from_env() -> Result<Self, String> {
        let provider = match std::env::var("BUZZ_OBJECT_STORE_PROVIDER") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => return Ok(Self::S3),
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(
                    "BUZZ_OBJECT_STORE_PROVIDER must be valid Unicode and one of 's3' or 'gcs'"
                        .to_string(),
                );
            }
        };
        match provider.parse::<ProviderKind>()? {
            ProviderKind::S3 => Ok(Self::S3),
            ProviderKind::Gcs => {
                let bucket = std::env::var("BUZZ_OBJECT_STORE_BUCKET")
                    .ok()
                    .filter(|bucket| !bucket.trim().is_empty())
                    .ok_or_else(|| {
                        "BUZZ_OBJECT_STORE_BUCKET must be set when \
                         BUZZ_OBJECT_STORE_PROVIDER=gcs"
                            .to_string()
                    })?;
                Ok(Self::Gcs {
                    bucket: bucket.trim().to_string(),
                })
            }
        }
    }
}

impl FromStr for ProviderKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "s3" => Ok(Self::S3),
            "gcs" => Ok(Self::Gcs),
            _ => Err(format!(
                "BUZZ_OBJECT_STORE_PROVIDER must be 's3' or 'gcs', got {value:?}"
            )),
        }
    }
}

/// A fully resolved provider configuration, ready to connect.
#[derive(Debug, Clone)]
pub enum ObjectStoreConfig {
    /// S3 or an S3-compatible backend.
    S3(S3StoreConfig),
    /// Google Cloud Storage.
    Gcs(GcsStoreConfig),
}

/// Build the single object-store client this process shares between media and
/// Git storage.
///
/// Connecting is async because a provider may have admission checks to run: the
/// Cloud Storage provider reads bucket metadata here and refuses to return a
/// client for a bucket whose configuration would break deletion.
pub async fn connect(config: &ObjectStoreConfig) -> Result<Arc<dyn ObjectStore>, ObjectStoreError> {
    match config {
        ObjectStoreConfig::S3(s3) => Ok(Arc::new(S3ObjectStore::new(s3)?)),
        ObjectStoreConfig::Gcs(gcs) => Ok(Arc::new(GcsObjectStore::connect(gcs).await?)),
    }
}

/// A stream of object byte chunks, usable with `axum::body::Body::from_stream()`.
pub type ByteStream =
    Pin<Box<dyn futures_core::Stream<Item = Result<Bytes, ObjectStoreError>> + Send>>;

/// Outcome of a create-only write of an immutable, content-addressed object.
///
/// Distinct from [`ConditionalWrite`] because content addressing makes the
/// committed revision uninteresting: the key *is* the digest, so a key that
/// already exists already holds these exact bytes. Callers treat both variants
/// as success and only the conformance probe distinguishes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImmutableWrite {
    /// This call wrote the object.
    Created,
    /// The key already held an object — by content addressing, the same bytes.
    AlreadyPresent,
}

/// Object metadata from a HEAD, as much as every provider can supply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMeta {
    /// Object size in bytes.
    pub size: u64,
    /// Current revision, when the provider reports one on HEAD.
    pub revision: Option<Revision>,
}

/// One page of a prefix-scoped listing.
///
/// Keys arrive in ascending UTF-8 binary order; callers rely on that for
/// streaming key-stream digests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListPage {
    /// `(key, size_in_bytes)` for each object in this page.
    pub objects: Vec<(String, u64)>,
    /// Token that fetches the next page, when one exists.
    pub next_continuation_token: Option<String>,
    /// Whether the provider truncated the listing.
    pub is_truncated: bool,
}

/// Per-key outcomes of one bulk delete.
///
/// Bulk deletion never fails on per-key outcomes: they are folded in here so
/// the caller owns retry and fail-closed policy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BulkDeleteOutcome {
    /// Keys the backend reported deleted (providers report already-missing
    /// keys as deleted too — the API is idempotent by design).
    pub deleted: u64,
    /// Keys reported absent through a per-key "no such key" error; equivalent
    /// to deleted for retry purposes.
    pub already_missing: u64,
    /// Keys whose deletion produced a version artifact (delete marker or
    /// version id) — evidence of bucket versioning, which deletion must fail
    /// closed on.
    pub versioned_keys: Vec<String>,
    /// Remaining per-key failures as `(key, code, message)`.
    pub failed: Vec<(String, String, String)>,
}

/// Provider-neutral kind of retained object version.
///
/// S3 reports concrete versions and delete markers. GCS reports generations;
/// because Buzz admits only GCS buckets with versioning and soft delete off,
/// those entries are always concrete objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectVersionKind {
    /// A byte-bearing object version.
    Object,
    /// A marker that hides older bytes without removing them.
    DeleteMarker,
}

/// One provider version under an object prefix.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ObjectVersionEntry {
    /// Object key.
    pub key: String,
    /// Opaque provider version token (S3 version ID or GCS generation).
    pub version_id: String,
    /// Whether this is a byte-bearing version or a delete marker.
    pub kind: ObjectVersionKind,
    /// Byte size for object versions; zero for delete markers.
    pub size: u64,
}

/// Exact provider version identifier used for permanent deletion.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ObjectVersionRef {
    /// Object key.
    pub key: String,
    /// Opaque provider version token (S3 version ID or GCS generation).
    pub version_id: String,
}

/// One page of prefix-scoped provider versions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectVersionsPage {
    /// Object versions and delete markers returned by this page.
    pub entries: Vec<ObjectVersionEntry>,
    /// First half of the next-page cursor.
    ///
    /// S3 uses a key marker; GCS stores its opaque page token here.
    pub next_key_marker: Option<String>,
    /// Second half of the next-page cursor, used by S3 when several versions
    /// of one key straddle a page boundary. GCS leaves it empty.
    pub next_version_id_marker: Option<String>,
    /// Whether more pages remain.
    pub is_truncated: bool,
}

/// The object-store operations Buzz actually performs.
///
/// Implementations are shared across the process behind an `Arc`: the relay
/// constructs exactly one provider and hands it to both the media facade and
/// the Git facade.
#[async_trait]
pub trait ObjectStore: Send + Sync {
    /// Which provider backs this client.
    fn provider(&self) -> ProviderKind;

    /// Store an object from a byte slice.
    ///
    /// For large blobs prefer [`ObjectStore::put_file`], which never holds the
    /// whole body in memory.
    async fn put(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<(), ObjectStoreError>;

    /// Stream a file from disk into the store without loading it into RAM.
    async fn put_file(
        &self,
        key: &str,
        path: &Path,
        content_type: &str,
    ) -> Result<(), ObjectStoreError>;

    /// Create-only write of an immutable, content-addressed object.
    ///
    /// A precondition failure is [`ImmutableWrite::AlreadyPresent`], not an
    /// error: the key is the digest of the bytes, so a collision means the
    /// stored bytes already equal these bytes.
    async fn put_immutable(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<ImmutableWrite, ObjectStoreError>;

    /// Write an object under a precondition.
    ///
    /// A failed precondition is [`ConditionalWrite::Conflict`], not an error.
    /// On [`ConditionalWrite::Committed`] the returned [`Revision`] is read
    /// from the write response and predicates the next conditional write; a
    /// provider that commits without returning a revision is non-conforming
    /// and must fail the operation rather than return an unusable token.
    async fn put_conditional(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: &str,
        condition: WriteCondition,
    ) -> Result<ConditionalWrite, ObjectStoreError>;

    /// Read an object's full body.
    async fn get(&self, key: &str) -> Result<Bytes, ObjectStoreError>;

    /// Read an inclusive byte range from an object.
    ///
    /// Only the requested slice is transferred; the full object is never
    /// loaded into memory.
    async fn get_range(&self, key: &str, start: u64, end: u64) -> Result<Bytes, ObjectStoreError>;

    /// Read an object as a chunk stream, without buffering the whole body.
    async fn get_stream(&self, key: &str) -> Result<ByteStream, ObjectStoreError>;

    /// Read an object's body **and** its revision from a single response.
    ///
    /// Returns `Ok(None)` when the object does not exist.
    ///
    /// A HEAD followed by a GET can straddle a concurrent writer: the HEAD's
    /// revision and the GET's body would describe different versions, and a
    /// caller that predicated its next write on the HEAD revision would be
    /// predicating on a version it never read. Both fields come from one
    /// response so the snapshot stays consistent.
    async fn get_with_revision(
        &self,
        key: &str,
    ) -> Result<Option<(Revision, Bytes)>, ObjectStoreError>;

    /// Read an object's metadata. Returns `Ok(None)` when it does not exist.
    async fn head(&self, key: &str) -> Result<Option<ObjectMeta>, ObjectStoreError>;

    /// Fetch one page of a prefix-scoped listing.
    ///
    /// `max_keys` bounds a single provider response, not the caller's total
    /// object budget; callers enforce the cumulative cap across pages.
    async fn list_page(
        &self,
        prefix: &str,
        continuation_token: Option<String>,
        max_keys: usize,
    ) -> Result<ListPage, ObjectStoreError>;

    /// Delete a single object. Deleting an absent object is not an error.
    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError>;

    /// Delete a bounded batch of objects, reporting per-key outcomes.
    ///
    /// Providers with a native batch API issue one request; providers without
    /// one issue bounded-concurrency individual deletes. Either way the caller
    /// sees the same per-key fold and decides retry policy.
    async fn delete_objects(&self, keys: &[String]) -> Result<BulkDeleteOutcome, ObjectStoreError>;

    /// Fetch one page of exact object versions under a prefix.
    ///
    /// Cursor fields are opaque to callers and must be replayed together.
    async fn list_versions_page(
        &self,
        prefix: &str,
        key_marker: Option<String>,
        version_id_marker: Option<String>,
        max_keys: usize,
    ) -> Result<ObjectVersionsPage, ObjectStoreError>;

    /// Permanently delete exact provider versions.
    async fn delete_versions(
        &self,
        versions: &[ObjectVersionRef],
    ) -> Result<BulkDeleteOutcome, ObjectStoreError>;

    /// Probe connectivity and bucket access.
    async fn ping(&self) -> Result<(), ObjectStoreError>;

    /// Whether the bucket retains non-current object versions.
    ///
    /// Deletion refuses versioned buckets: an ordinary delete on one inserts a
    /// delete marker rather than proving logical absence, so a bulk delete
    /// could report success while the bytes remain reachable.
    async fn versioning_detected(&self) -> Result<bool, ObjectStoreError>;

    /// Read an object after rejecting bodies larger than `max_bytes`.
    ///
    /// The HEAD bound is checked first so an oversized object is never
    /// transferred, and the returned length is re-checked in case the provider
    /// reported a bad size.
    async fn get_limited(&self, key: &str, max_bytes: u64) -> Result<Bytes, ObjectStoreError> {
        let meta = self
            .head(key)
            .await?
            .ok_or_else(|| ObjectStoreError::NotFound { key: key.into() })?;
        if meta.size > max_bytes {
            return Err(ObjectStoreError::ObjectTooLarge {
                key: key.into(),
                size: meta.size,
                max: max_bytes,
            });
        }

        let bytes = self.get(key).await?;
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if size > max_bytes {
            return Err(ObjectStoreError::ObjectTooLarge {
                key: key.into(),
                size,
                max: max_bytes,
            });
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_names_parse() {
        assert_eq!("s3".parse::<ProviderKind>(), Ok(ProviderKind::S3));
        assert_eq!("gcs".parse::<ProviderKind>(), Ok(ProviderKind::Gcs));
    }

    /// Unknown or near-miss spellings fail rather than silently selecting the
    /// default provider: a typo in `BUZZ_OBJECT_STORE_PROVIDER` must not
    /// quietly point a Cloud Storage deployment at S3.
    #[test]
    fn unknown_provider_names_are_rejected() {
        for invalid in ["", "S3", "GCS", "google", "gs", "minio"] {
            let error = invalid
                .parse::<ProviderKind>()
                .expect_err("must reject unknown provider");
            assert!(
                error.contains("must be 's3' or 'gcs'"),
                "unexpected error for {invalid:?}: {error}"
            );
        }
    }
}
