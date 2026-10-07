//! S3/MinIO storage client.

use std::path::Path;
use std::pin::Pin;

use buzz_core::tenant::{CommunityId, TenantContext};

use crate::config::{MediaConfig, S3AddressingStyle};
use crate::error::MediaError;
use bytes::Bytes;
use s3::creds::Credentials;
use s3::{Bucket, Region};
use serde::{Deserialize, Serialize};

/// A stream of byte chunks from S3, usable with `axum::body::Body::from_stream()`.
pub type ByteStream = Pin<Box<dyn futures_core::Stream<Item = Result<Bytes, MediaError>> + Send>>;

/// S3-compatible object storage client.
pub struct MediaStorage {
    bucket: Box<Bucket>,
}

impl MediaStorage {
    /// Create a new storage client from config.
    ///
    /// Credential selection:
    /// - If both `s3_access_key` and `s3_secret_key` are non-empty, use them as
    ///   static credentials (MinIO/local/dev, or any static-key deployment).
    /// - Otherwise, fall back to the AWS default credential chain via
    ///   [`Credentials::default`]: environment, shared profile, web-identity
    ///   token (IRSA on EKS — `AssumeRoleWithWebIdentity`), container, and
    ///   instance-metadata providers, in that order. This lets the relay use
    ///   the pod's IAM role without long-lived static keys.
    pub fn new(config: &MediaConfig) -> Result<Self, MediaError> {
        let region = Region::Custom {
            region: config.s3_region.clone(),
            endpoint: config.s3_endpoint.clone(),
        };
        let creds = match (
            config.s3_access_key.is_empty(),
            config.s3_secret_key.is_empty(),
        ) {
            (false, false) => Credentials::new(
                Some(&config.s3_access_key),
                Some(&config.s3_secret_key),
                None,
                None,
                None,
            ),
            (true, true) => {
                // No static keys configured: resolve from the AWS credential chain
                // (IRSA web-identity, env, profile, instance metadata).
                Credentials::default()
            }
            _ => {
                return Err(MediaError::StorageError(
                    "s3_access_key and s3_secret_key must be configured together, or both empty to use the AWS credential chain"
                        .to_string(),
                ));
            }
        }
        .map_err(|e| MediaError::StorageError(e.to_string()))?;
        let bucket = Bucket::new(&config.s3_bucket, region, creds)
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        let bucket = match config.s3_addressing_style {
            S3AddressingStyle::Path => bucket.with_path_style(),
            S3AddressingStyle::Virtual => bucket,
        };
        Ok(Self { bucket })
    }

    /// Store an object from a byte slice.
    ///
    /// Used for images, sidecars, and thumbnails. For large video files use
    /// [`put_file`] to avoid loading the entire blob into RAM.
    pub async fn put(&self, key: &str, bytes: &[u8], content_type: &str) -> Result<(), MediaError> {
        self.bucket
            .put_object_with_content_type(key, bytes, content_type)
            .await?;
        Ok(())
    }

    /// Stream a file from disk into S3 without loading it into RAM.
    ///
    /// Uses rust-s3's `put_object_stream_with_content_type` which reads from
    /// the file incrementally via an 8 MiB `BufReader`. The full file is never
    /// held in memory simultaneously. Intended for video blobs (up to 500 MB).
    pub async fn put_file(
        &self,
        key: &str,
        path: &Path,
        content_type: &str,
    ) -> Result<(), MediaError> {
        const BUF: usize = 8 * 1024 * 1024; // 8 MiB read buffer

        let file = tokio::fs::File::open(path)
            .await
            .map_err(|e| MediaError::Io(e.to_string()))?;
        let mut reader = tokio::io::BufReader::with_capacity(BUF, file);

        self.bucket
            .put_object_stream_with_content_type(&mut reader, key, content_type)
            .await?;
        Ok(())
    }

    /// Retrieve an object's bytes.
    pub async fn get(&self, key: &str) -> Result<Vec<u8>, MediaError> {
        match self.bucket.get_object(key).await {
            Ok(response) => Ok(response.to_vec()),
            Err(s3::error::S3Error::HttpFailWithBody(404, _)) => Err(MediaError::NotFound),
            Err(e) => Err(MediaError::StorageError(e.to_string())),
        }
    }

    /// Retrieve a byte range from an object via S3-native `Range` GET.
    ///
    /// `start` and `end` are inclusive byte offsets. Only the requested slice
    /// is transferred from S3 — the full object is never loaded into RAM.
    /// Intended for HTTP 206 range responses on large video blobs.
    pub async fn get_range(&self, key: &str, start: u64, end: u64) -> Result<Vec<u8>, MediaError> {
        match self.bucket.get_object_range(key, start, Some(end)).await {
            Ok(response) => Ok(response.to_vec()),
            Err(s3::error::S3Error::HttpFailWithBody(404, _)) => Err(MediaError::NotFound),
            Err(e) => Err(MediaError::StorageError(e.to_string())),
        }
    }

    /// Stream an object's bytes from S3 without loading into RAM.
    ///
    /// Returns a pinned stream of `Result<Bytes, MediaError>` chunks.
    /// The full object is never buffered — intended for streaming large
    /// blobs (video) directly into HTTP responses via `Body::from_stream()`.
    pub async fn get_stream(&self, key: &str) -> Result<ByteStream, MediaError> {
        let response = self
            .bucket
            .get_object_stream(key)
            .await
            .map_err(|e| MediaError::StorageError(e.to_string()))?;

        if response.status_code == 404 {
            return Err(MediaError::NotFound);
        }

        let stream = futures_util::StreamExt::map(response.bytes, |chunk| {
            chunk.map_err(|e| MediaError::StorageError(e.to_string()))
        });
        Ok(Box::pin(stream))
    }

    /// Check if an object exists. Returns false on 404.
    pub async fn head(&self, key: &str) -> Result<bool, MediaError> {
        match self.bucket.head_object(key).await {
            Ok(_) => Ok(true),
            Err(s3::error::S3Error::HttpFailWithBody(404, _)) => Ok(false),
            Err(e) => Err(MediaError::StorageError(e.to_string())),
        }
    }

    /// Delete an object. Returns an error on failure — callers decide whether to propagate.
    pub async fn delete(&self, key: &str) -> Result<(), MediaError> {
        self.bucket
            .delete_object(key)
            .await
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        Ok(())
    }

    /// HEAD with metadata — returns Content-Length (size).
    pub async fn head_with_metadata(&self, key: &str) -> Result<Option<BlobHeadMeta>, MediaError> {
        match self.bucket.head_object(key).await {
            Ok((result, _)) => Ok(Some(BlobHeadMeta {
                size: result.content_length.unwrap_or(0) as u64,
            })),
            Err(s3::error::S3Error::HttpFailWithBody(404, _)) => Ok(None),
            Err(e) => Err(MediaError::StorageError(e.to_string())),
        }
    }

    /// Detect whether the bucket has ever had versioning enabled.
    ///
    /// rust-s3 exposes no GetBucketVersioning, so this writes and inspects a
    /// short-lived fleet probe object instead: versioning-enabled (and
    /// versioning-suspended) buckets stamp new writes with a version id.
    /// Deletion refuses versioned buckets because bulk deletes without a
    /// VersionId would only insert delete markers, not prove logical absence.
    pub async fn bucket_versioning_detected(&self) -> Result<bool, MediaError> {
        let key = format!("probe/deletion-versioning-{}", uuid::Uuid::new_v4());
        self.put(&key, b"buzz deletion versioning probe", "text/plain")
            .await?;
        let inspected = self.bucket.head_object(&key).await;
        let removed = self.bucket.delete_object(&key).await;
        let (head, _) = inspected.map_err(|e| MediaError::StorageError(e.to_string()))?;
        removed.map_err(|e| MediaError::StorageError(e.to_string()))?;
        Ok(head.version_id.is_some())
    }

    /// Bulk-delete up to one manifest chunk of keys via S3 `DeleteObjects`.
    ///
    /// Never fails on per-key outcomes: they are folded into
    /// [`BulkDeleteOutcome`] so the caller owns retry/fail-closed policy.
    /// Historical MinIO releases report already-absent keys as
    /// `NoSuchKey`/`NoSuchVersion` errors instead of deleted; both map to
    /// `already_missing` to keep checkpointed retry idempotent.
    pub async fn delete_objects(&self, keys: &[String]) -> Result<BulkDeleteOutcome, MediaError> {
        if keys.is_empty() {
            return Ok(BulkDeleteOutcome::default());
        }
        let identifiers = keys
            .iter()
            .map(|key| s3::serde_types::ObjectIdentifier::new(key.clone()))
            .collect::<Vec<_>>();
        let result = self
            .bucket
            .delete_objects(identifiers)
            .await
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        Ok(fold_bulk_delete_result(result))
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
        let resp = self.bucket.get_object(&key).await?;
        let meta: BlobMeta = serde_json::from_slice(&resp.to_vec())?;
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
        self.list_page(None, 1).await.map(|_| ())
    }

    /// One page of a full-bucket listing, for the storage sweep. Wraps
    /// rust-s3's manual `list_page` (NOT the auto-paginating `list`, which
    /// has no cap) and converts the result into the storage-agnostic
    /// [`crate::bucket_index::Page`] shape the pure fold consumes.
    ///
    /// `max_keys` bounds one HTTP response, not the sweep's total object
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
    /// O(tenant objects) regardless of fleet size. `ListObjectsV2` returns
    /// keys in ascending UTF-8 binary order, which callers rely on for
    /// streaming key-stream digests.
    pub async fn list_prefix_page(
        &self,
        prefix: &str,
        continuation_token: Option<String>,
        max_keys: usize,
    ) -> Result<crate::bucket_index::Page, MediaError> {
        let (result, _status) = self
            .bucket
            .list_page(
                prefix.to_string(),
                None,
                continuation_token,
                None,
                Some(max_keys),
            )
            .await?;
        Ok(crate::bucket_index::Page {
            objects: result
                .contents
                .into_iter()
                .map(|obj| (obj.key, obj.size))
                .collect(),
            next_continuation_token: result.next_continuation_token,
            is_truncated: result.is_truncated,
        })
    }
}

/// Per-key outcomes of one bulk `DeleteObjects` call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BulkDeleteOutcome {
    /// Keys the backend reported deleted (S3 reports already-missing keys as
    /// deleted too — the API is idempotent by design).
    pub deleted: u64,
    /// Keys reported absent via legacy MinIO `NoSuchKey`/`NoSuchVersion`
    /// per-key errors; equivalent to deleted for retry purposes.
    pub already_missing: u64,
    /// Keys whose deletion produced a version artifact (delete marker or
    /// version id) — evidence of bucket versioning, which deletion must
    /// fail closed on.
    pub versioned_keys: Vec<String>,
    /// Remaining per-key failures as `(key, code, message)`.
    pub failed: Vec<(String, String, String)>,
}

fn fold_bulk_delete_result(result: s3::serde_types::DeleteObjectsResult) -> BulkDeleteOutcome {
    let mut outcome = BulkDeleteOutcome::default();
    for deleted in result.deleted {
        if deleted.delete_marker == Some(true)
            || deleted.delete_marker_version_id.is_some()
            || deleted.version_id.is_some()
        {
            outcome.versioned_keys.push(deleted.key);
        } else {
            outcome.deleted += 1;
        }
    }
    for error in result.errors {
        if error.code == "NoSuchKey" || error.code == "NoSuchVersion" {
            outcome.already_missing += 1;
        } else {
            outcome.failed.push((error.key, error.code, error.message));
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The bulk-delete fold is the retry-idempotence contract: legacy MinIO
    /// absent-key errors count as success, version artifacts are surfaced for
    /// fail-closed handling, and anything else stays a per-key failure.
    #[test]
    fn bulk_delete_fold_maps_absent_keys_and_version_artifacts() {
        use s3::serde_types::{DeleteError, DeleteObjectsResult, DeletedObject};
        let deleted_object = |key: &str, marker: bool| DeletedObject {
            key: key.to_string(),
            version_id: None,
            delete_marker: marker.then_some(true),
            delete_marker_version_id: marker.then(|| "v1".to_string()),
        };
        let delete_error = |key: &str, code: &str, message: &str| DeleteError {
            key: key.to_string(),
            code: code.to_string(),
            message: message.to_string(),
            version_id: None,
        };
        let result = DeleteObjectsResult {
            deleted: vec![
                deleted_object("plain", false),
                deleted_object("marked", true),
            ],
            errors: vec![
                delete_error("gone", "NoSuchKey", "absent"),
                delete_error("gone-version", "NoSuchVersion", "absent"),
                delete_error("denied", "AccessDenied", "nope"),
            ],
        };
        let outcome = fold_bulk_delete_result(result);
        assert_eq!(outcome.deleted, 1);
        assert_eq!(outcome.already_missing, 2);
        assert_eq!(outcome.versioned_keys, vec!["marked".to_string()]);
        assert_eq!(
            outcome.failed,
            vec![(
                "denied".to_string(),
                "AccessDenied".to_string(),
                "nope".to_string()
            )]
        );
    }

    fn tenant(n: u128) -> TenantContext {
        TenantContext::resolved(
            CommunityId::from_uuid(uuid::Uuid::from_u128(n)),
            "media.example",
        )
    }

    fn storage_config(access: &str, secret: &str) -> crate::config::MediaConfig {
        crate::config::MediaConfig {
            s3_endpoint: "http://localhost:9000".to_string(),
            s3_access_key: access.to_string(),
            s3_secret_key: secret.to_string(),
            s3_bucket: "buzz-media".to_string(),
            s3_region: "us-west-2".to_string(),
            s3_addressing_style: S3AddressingStyle::Path,
            max_image_bytes: 50 * 1024 * 1024,
            max_gif_bytes: 10 * 1024 * 1024,
            max_video_bytes: 524_288_000,
            max_file_bytes: 104_857_600,
            public_base_url: "http://localhost:3000/media".to_string(),
            upload_records_enabled: false,
            upload_ip_header: None,
            upload_port_header: None,
        }
    }

    /// Static keys present: builds a client without touching the AWS
    /// credential chain (no env/metadata access), and the signing region
    /// comes from config rather than a hardcoded "us-east-1".
    #[test]
    fn static_keys_build_client_with_configured_region() {
        let storage = MediaStorage::new(&storage_config("buzz_dev", "buzz_dev_secret"))
            .expect("static creds should build a client");
        match storage.bucket.region {
            Region::Custom { ref region, .. } => assert_eq!(region, "us-west-2"),
            other => panic!("expected Custom region, got {other:?}"),
        }
    }

    #[test]
    fn client_constructor_applies_both_addressing_styles() {
        let path = MediaStorage::new(&storage_config("buzz_dev", "buzz_dev_secret"))
            .expect("path-style client");
        assert!(path.bucket.is_path_style());
        assert_eq!(path.bucket.url(), "http://localhost:9000/buzz-media");

        let mut virtual_config = storage_config("buzz_dev", "buzz_dev_secret");
        virtual_config.s3_addressing_style = S3AddressingStyle::Virtual;
        let virtual_hosted = MediaStorage::new(&virtual_config).expect("virtual-hosted client");
        assert!(virtual_hosted.bucket.is_subdomain_style());
        assert_eq!(
            virtual_hosted.bucket.url(),
            "http://buzz-media.localhost:9000"
        );
    }

    #[test]
    fn partial_static_keys_are_rejected() {
        let err = match MediaStorage::new(&storage_config("buzz_dev", "")) {
            Ok(_) => panic!("partial static creds must not silently use credential chain"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("must be configured together"),
            "unexpected error: {err}"
        );

        let err = match MediaStorage::new(&storage_config("", "buzz_dev_secret")) {
            Ok(_) => panic!("partial static creds must not silently use credential chain"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("must be configured together"),
            "unexpected error: {err}"
        );
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
