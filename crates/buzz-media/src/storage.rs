//! Media storage facade.
//!
//! Thin domain layer over [`buzz_object_store::ObjectStore`]: it owns the
//! media key vocabulary (content-addressed blobs, community-scoped metadata
//! sidecars) and the media error surface, and knows nothing about which
//! provider is underneath.

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use buzz_core::tenant::{CommunityId, TenantContext};
use buzz_object_store::{ObjectStore, ObjectStoreError};

use crate::error::MediaError;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

pub use buzz_object_store::{
    BulkDeleteOutcome, ObjectVersionEntry, ObjectVersionKind, ObjectVersionRef, ObjectVersionsPage,
};

/// A stream of byte chunks from object storage, usable with
/// `axum::body::Body::from_stream()`.
pub type ByteStream = Pin<Box<dyn futures_core::Stream<Item = Result<Bytes, MediaError>> + Send>>;

/// Media object storage client.
pub struct MediaStorage {
    store: Arc<dyn ObjectStore>,
}

impl MediaStorage {
    /// Wrap an already-constructed object store.
    ///
    /// The relay builds one provider per process and shares it between media
    /// and Git storage rather than opening a second client against the same
    /// bucket.
    pub fn with_store(store: Arc<dyn ObjectStore>) -> Self {
        Self { store }
    }

    /// The shared object store behind this facade, for handing to another
    /// domain facade (see [`MediaStorage::with_store`]).
    pub fn object_store(&self) -> Arc<dyn ObjectStore> {
        Arc::clone(&self.store)
    }

    /// Store an object from a byte slice.
    ///
    /// Used for images, sidecars, and thumbnails. For large video files use
    /// [`MediaStorage::put_file`] to avoid loading the entire blob into RAM.
    pub async fn put(&self, key: &str, bytes: &[u8], content_type: &str) -> Result<(), MediaError> {
        self.store
            .put(key, bytes, content_type)
            .await
            .map_err(storage_error)
    }

    /// Stream a file from disk into object storage without loading it into RAM.
    ///
    /// The full file is never held in memory simultaneously. Intended for
    /// video blobs (up to 500 MB).
    pub async fn put_file(
        &self,
        key: &str,
        path: &Path,
        content_type: &str,
    ) -> Result<(), MediaError> {
        self.store
            .put_file(key, path, content_type)
            .await
            .map_err(storage_error)
    }

    /// Retrieve an object's bytes.
    pub async fn get(&self, key: &str) -> Result<Vec<u8>, MediaError> {
        Ok(self.store.get(key).await?.to_vec())
    }

    /// Retrieve a byte range from an object via a native ranged GET.
    ///
    /// `start` and `end` are inclusive byte offsets. Only the requested slice
    /// is transferred — the full object is never loaded into RAM. Intended for
    /// HTTP 206 range responses on large video blobs.
    pub async fn get_range(&self, key: &str, start: u64, end: u64) -> Result<Vec<u8>, MediaError> {
        Ok(self.store.get_range(key, start, end).await?.to_vec())
    }

    /// Stream an object's bytes without loading them into RAM.
    ///
    /// Returns a pinned stream of `Result<Bytes, MediaError>` chunks.
    /// The full object is never buffered — intended for streaming large
    /// blobs (video) directly into HTTP responses via `Body::from_stream()`.
    pub async fn get_stream(&self, key: &str) -> Result<ByteStream, MediaError> {
        let stream = self.store.get_stream(key).await?;
        Ok(Box::pin(futures_util::StreamExt::map(stream, |chunk| {
            chunk.map_err(storage_error)
        })))
    }

    /// Check if an object exists. Returns false when absent.
    pub async fn head(&self, key: &str) -> Result<bool, MediaError> {
        Ok(self.store.head(key).await.map_err(storage_error)?.is_some())
    }

    /// Delete an object. Returns an error on failure — callers decide whether to propagate.
    pub async fn delete(&self, key: &str) -> Result<(), MediaError> {
        self.store.delete(key).await.map_err(storage_error)
    }

    /// HEAD with metadata — returns the object size.
    pub async fn head_with_metadata(&self, key: &str) -> Result<Option<BlobHeadMeta>, MediaError> {
        Ok(self
            .store
            .head(key)
            .await
            .map_err(storage_error)?
            .map(|meta| BlobHeadMeta { size: meta.size }))
    }

    /// Detect whether the bucket retains non-current object versions.
    ///
    /// Deletion refuses versioned buckets because bulk deletes without a
    /// version qualifier would only insert delete markers, not prove logical
    /// absence.
    pub async fn bucket_versioning_detected(&self) -> Result<bool, MediaError> {
        self.store
            .versioning_detected()
            .await
            .map_err(storage_error)
    }

    /// Bulk-delete up to one manifest chunk of keys.
    ///
    /// Never fails on per-key outcomes: they are folded into
    /// [`BulkDeleteOutcome`] so the caller owns retry/fail-closed policy.
    pub async fn delete_objects(&self, keys: &[String]) -> Result<BulkDeleteOutcome, MediaError> {
        self.store.delete_objects(keys).await.map_err(storage_error)
    }

    /// Non-destructively verify that exact-version listing is reachable.
    pub async fn preflight_version_listing(&self, prefix: &str) -> Result<(), MediaError> {
        self.list_prefix_versions_page(prefix, None, None, 1)
            .await
            .map(|_| ())
    }

    /// Permanently delete exact provider versions.
    ///
    /// The provider translates the opaque `version_id` to an S3 version ID or
    /// a GCS generation. Domain deletion code never performs that translation.
    pub async fn delete_object_versions(
        &self,
        versions: &[ObjectVersionRef],
    ) -> Result<BulkDeleteOutcome, MediaError> {
        self.store
            .delete_versions(versions)
            .await
            .map_err(storage_error)
    }

    /// Build the community-scoped sidecar key for a given sha256 (bare hash).
    ///
    /// Raw media bytes remain shared content-addressed CAS (`{sha}.{ext}`), but
    /// the metadata sidecar is the tenant read gate. A blob in another
    /// community must never be observable through a global `_meta/{sha}.json`
    /// lookup.
    pub fn sidecar_key(community: CommunityId, sha256: &str) -> String {
        format!("_meta/{community}/{sha256}.json")
    }

    /// Build the community-scoped sidecar key from the resolved request tenant.
    pub fn ctx_sidecar_key(ctx: &TenantContext, sha256: &str) -> String {
        Self::sidecar_key(ctx.community(), sha256)
    }

    /// Read community-scoped sidecar JSON for a given sha256 (bare hash).
    pub async fn get_sidecar(
        &self,
        ctx: &TenantContext,
        sha256: &str,
    ) -> Result<BlobMeta, MediaError> {
        let key = Self::ctx_sidecar_key(ctx, sha256);
        let bytes = self.store.get(&key).await.map_err(storage_error)?;
        let meta: BlobMeta = serde_json::from_slice(&bytes)?;
        Ok(meta)
    }

    /// Write community-scoped sidecar JSON for a given sha256 (bare hash).
    ///
    /// `ctx` must be the server-resolved request tenant. Callers must never
    /// derive the community from client-supplied blob metadata, URLs, or event
    /// tags; this sidecar key is the tenant read gate for otherwise shared CAS
    /// bytes.
    pub async fn put_sidecar(
        &self,
        ctx: &TenantContext,
        sha256: &str,
        meta: &BlobMeta,
    ) -> Result<(), MediaError> {
        let key = Self::ctx_sidecar_key(ctx, sha256);
        let meta_json = serde_json::to_vec(meta)?;
        self.put(&key, &meta_json, "application/json").await
    }

    /// Convenience: read just the MIME type from the community sidecar.
    ///
    /// Returns `None` for both absent sidecars and storage read failures. Public
    /// read handlers intentionally collapse that distinction to 404 so an
    /// A-bound request cannot distinguish a B-only blob from a missing blob.
    pub async fn read_sidecar_mime(&self, ctx: &TenantContext, sha256_ext: &str) -> Option<String> {
        let sha256 = sha256_ext.split('.').next().unwrap_or(sha256_ext);
        self.get_sidecar(ctx, sha256)
            .await
            .ok()
            .map(|m| m.mime_type)
    }

    /// Probe object-store connectivity and bucket access.
    pub async fn ping(&self) -> Result<(), MediaError> {
        self.store.ping().await.map_err(storage_error)
    }

    /// One page of a full-bucket listing, for the storage sweep. Converts the
    /// provider listing into the storage-agnostic [`crate::bucket_index::Page`]
    /// shape the pure fold consumes.
    ///
    /// `max_keys` bounds one provider response, not the sweep's total object
    /// cap — the caller (`fold_bucket_listing`) enforces the cumulative cap
    /// across pages.
    pub async fn list_page(
        &self,
        continuation_token: Option<String>,
        max_keys: usize,
    ) -> Result<crate::bucket_index::Page, MediaError> {
        self.list_prefix_page("", continuation_token, max_keys)
            .await
    }

    /// One page of a prefix-scoped listing.
    ///
    /// Deletion enumerates the target community's exact key prefixes with
    /// this instead of listing the whole fleet bucket: cost stays
    /// O(tenant objects) regardless of fleet size. Listings return keys in
    /// ascending UTF-8 binary order, which callers rely on for streaming
    /// key-stream digests.
    pub async fn list_prefix_page(
        &self,
        prefix: &str,
        continuation_token: Option<String>,
        max_keys: usize,
    ) -> Result<crate::bucket_index::Page, MediaError> {
        let page = self
            .store
            .list_page(prefix, continuation_token, max_keys)
            .await
            .map_err(storage_error)?;
        Ok(crate::bucket_index::Page {
            objects: page.objects,
            next_continuation_token: page.next_continuation_token,
            is_truncated: page.is_truncated,
        })
    }

    /// One page of exact object versions under a prefix.
    ///
    /// Both cursor fields are opaque and must be replayed together. S3 uses
    /// the pair directly; GCS places its page token in `key_marker` and leaves
    /// `version_id_marker` empty.
    pub async fn list_prefix_versions_page(
        &self,
        prefix: &str,
        key_marker: Option<String>,
        version_id_marker: Option<String>,
        max_keys: usize,
    ) -> Result<ObjectVersionsPage, MediaError> {
        self.store
            .list_versions_page(prefix, key_marker, version_id_marker, max_keys)
            .await
            .map_err(storage_error)
    }
}

/// Collapse any provider failure — including a missing object — into the
/// generic media storage error.
///
/// Used on the paths that historically surfaced a backend 404 as a storage
/// failure rather than a media-level `NotFound`: sidecar reads are guarded by
/// an explicit HEAD, and a bare absence there means the bucket disagreed with
/// the guard.
fn storage_error(error: ObjectStoreError) -> MediaError {
    MediaError::StorageError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn tenant(n: u128) -> TenantContext {
        TenantContext::resolved(
            CommunityId::from_uuid(uuid::Uuid::from_u128(n)),
            "media.example",
        )
    }

    #[test]
    fn tenant_key_writers_are_covered_by_deletion_taxonomy() {
        let ctx = tenant(1);
        let community = *ctx.community().as_uuid();
        let sha = "a".repeat(64);
        let event_id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let sidecar = MediaStorage::ctx_sidecar_key(&ctx, &sha);
        let upload = crate::upload_record::upload_record_key(&ctx, &sha, event_id);
        let prefixes = crate::bucket_index::tenant_prefixes(community);

        for key in [sidecar, upload] {
            assert!(
                prefixes.iter().any(|prefix| key.starts_with(prefix)),
                "tenant writer key {key} is outside deletion prefixes"
            );
            assert!(
                crate::bucket_index::is_tenant_owned_key(community, &key),
                "tenant writer key {key} is not recognized by deletion taxonomy"
            );
        }
    }

    #[test]
    fn sidecar_keys_are_community_scoped() {
        let a = tenant(1);
        let b = tenant(2);
        let sha = "f".repeat(64);

        assert_eq!(
            MediaStorage::ctx_sidecar_key(&a, &sha),
            format!("_meta/{}/{sha}.json", a.community())
        );
        assert_ne!(
            MediaStorage::ctx_sidecar_key(&a, &sha),
            MediaStorage::ctx_sidecar_key(&b, &sha)
        );
        assert_ne!(
            MediaStorage::ctx_sidecar_key(&a, &sha),
            format!("_meta/{sha}.json")
        );
    }

    /// Mutate-bite shape for the media substrate: same CAS bytes/hash can be
    /// known in A and B, but the sidecar is the read/existence gate. If the
    /// community segment is dropped from `sidecar_key`, B's metadata overwrites
    /// A's in this map and A observes B's MIME (wrong answer, not absence).
    #[test]
    fn same_sha_sidecars_do_not_bleed_between_communities() {
        let a = tenant(1);
        let b = tenant(2);
        let sha = "a".repeat(64);
        let mut sidecars = HashMap::new();

        sidecars.insert(MediaStorage::ctx_sidecar_key(&a, &sha), "image/png");
        sidecars.insert(MediaStorage::ctx_sidecar_key(&b, &sha), "video/mp4");

        assert_eq!(
            sidecars[&MediaStorage::ctx_sidecar_key(&a, &sha)],
            "image/png"
        );
        assert_eq!(
            sidecars[&MediaStorage::ctx_sidecar_key(&b, &sha)],
            "video/mp4"
        );
    }
}

/// Metadata returned by HEAD — just enough for BUD-01 response headers.
pub struct BlobHeadMeta {
    pub size: u64,
}

/// Full blob metadata — stored as sidecar JSON in `_meta/{community}/{sha256}.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlobMeta {
    /// Pixel dimensions ("WxH").
    pub dim: String,
    /// Blurhash string.
    pub blurhash: String,
    /// Full URL to thumbnail.
    pub thumb_url: String,
    /// File extension (e.g. "jpg").
    pub ext: String,
    /// MIME type (e.g. "image/jpeg").
    pub mime_type: String,
    /// File size in bytes.
    pub size: u64,
    /// Unix timestamp when the blob was first uploaded.
    #[serde(default)]
    pub uploaded_at: i64,
    /// Video duration in seconds. `None` for non-video blobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
}
