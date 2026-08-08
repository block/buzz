//! S3/MinIO storage client.

use std::path::Path;
use std::pin::Pin;

use buzz_core::tenant::{CommunityId, TenantContext};

use crate::config::{MediaConfig, MediaMigrationPhase, S3AddressingStyle};
use crate::error::MediaError;
use bytes::Bytes;
use s3::creds::Credentials;
use s3::{Bucket, Region};
use serde::{Deserialize, Serialize};

/// A stream of byte chunks from S3, usable with `axum::body::Body::from_stream()`.
pub type ByteStream = Pin<Box<dyn futures_core::Stream<Item = Result<Bytes, MediaError>> + Send>>;

/// Full-object stream plus metadata resolved from one migration candidate.
pub struct BlobStream {
    pub size: u64,
    pub stream: ByteStream,
}

/// Parsed single-range request independent of object size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadByteRange {
    /// `bytes=START-END` or `bytes=START-`. `end` is inclusive when present.
    Absolute { start: u64, end: Option<u64> },
    /// `bytes=-N`, the final N bytes.
    Suffix(u64),
}

impl PayloadByteRange {
    /// Resolve this range against a known total object size.
    pub fn bounds(self, total: u64) -> Option<(u64, u64)> {
        match self {
            Self::Absolute { start, end } => {
                if let Some(end) = end {
                    if start > end {
                        return None;
                    }
                    Some((start, end))
                } else {
                    Some((start, u64::MAX))
                }
            }
            Self::Suffix(n) => {
                if n == 0 || total == 0 {
                    return None;
                }
                let start = total.saturating_sub(n);
                Some((start, total.saturating_sub(1)))
            }
        }
    }
}

/// Result of a tenant-scoped byte-range read.
pub enum PayloadRangeRead {
    Satisfiable {
        total: u64,
        start: u64,
        end: u64,
        bytes: Vec<u8>,
    },
    Unsatisfiable {
        total: u64,
    },
}

/// S3-compatible object storage client.
pub struct MediaStorage {
    bucket: Box<Bucket>,
    migration_phase: MediaMigrationPhase,
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
        Ok(Self {
            bucket,
            migration_phase: config.migration_phase,
        })
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

    /// Store a media payload according to the configured migration layout.
    ///
    /// Dual mode writes the sharded primary first and the legacy compatibility
    /// copy second. Callers publish sidecars only after this returns success.
    pub async fn put_payload(
        &self,
        ctx: &TenantContext,
        sha256: &str,
        ext: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<String, MediaError> {
        let legacy = crate::keys::legacy_blob_key(sha256, ext)
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        let sharded = crate::keys::sharded_blob_key(ctx.community(), sha256, ext)
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        if self.migration_phase.writes_sharded() {
            self.put(&sharded, bytes, content_type).await?;
        }
        if self.migration_phase.writes_legacy() {
            self.put(&legacy, bytes, content_type).await?;
        }
        Ok(if self.migration_phase.writes_sharded() {
            sharded
        } else {
            legacy
        })
    }

    /// Stream a media payload from disk according to the configured layout.
    pub async fn put_payload_file(
        &self,
        ctx: &TenantContext,
        sha256: &str,
        ext: &str,
        path: &Path,
        content_type: &str,
    ) -> Result<String, MediaError> {
        let legacy = crate::keys::legacy_blob_key(sha256, ext)
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        let sharded = crate::keys::sharded_blob_key(ctx.community(), sha256, ext)
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        if self.migration_phase.writes_sharded() {
            self.put_file(&sharded, path, content_type).await?;
        }
        if self.migration_phase.writes_legacy() {
            self.put_file(&legacy, path, content_type).await?;
        }
        Ok(if self.migration_phase.writes_sharded() {
            sharded
        } else {
            legacy
        })
    }

    /// Store a thumbnail according to the configured migration layout.
    pub async fn put_thumbnail(
        &self,
        ctx: &TenantContext,
        sha256: &str,
        bytes: &[u8],
    ) -> Result<String, MediaError> {
        let legacy = crate::keys::legacy_thumb_key(sha256)
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        let sharded = crate::keys::sharded_thumb_key(ctx.community(), sha256)
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        if self.migration_phase.writes_sharded() {
            self.put(&sharded, bytes, "image/jpeg").await?;
        }
        if self.migration_phase.writes_legacy() {
            self.put(&legacy, bytes, "image/jpeg").await?;
        }
        Ok(if self.migration_phase.writes_sharded() {
            sharded
        } else {
            legacy
        })
    }

    /// Return the authoritative key only when every layout required by the
    /// current write phase already exists. This prevents a re-upload from
    /// skipping the missing compatibility copy after a phase change.
    pub async fn existing_write_key(
        &self,
        ctx: &TenantContext,
        payload_name: &str,
    ) -> Result<Option<String>, MediaError> {
        let candidates =
            crate::keys::read_candidates(ctx, payload_name).map_err(|_| MediaError::NotFound)?;
        let sharded_exists = if self.migration_phase.writes_sharded() {
            self.head(&candidates.sharded).await?
        } else {
            true
        };
        let legacy_exists = if self.migration_phase.writes_legacy() {
            self.head(&candidates.legacy).await?
        } else {
            true
        };

        if !sharded_exists || !legacy_exists {
            return Ok(None);
        }
        Ok(Some(if self.migration_phase.writes_sharded() {
            candidates.sharded
        } else {
            candidates.legacy
        }))
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
            Ok(response) if response.status_code() == 404 => Err(MediaError::NotFound),
            Ok(response) if (200..300).contains(&response.status_code()) => Ok(response.to_vec()),
            Ok(response) => Err(MediaError::StorageError(format!(
                "S3 range GET returned status {}",
                response.status_code()
            ))),
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
        let response = match self.bucket.get_object_stream(key).await {
            Ok(response) => response,
            Err(s3::error::S3Error::HttpFailWithBody(404, _)) => {
                return Err(MediaError::NotFound);
            }
            Err(e) => return Err(MediaError::StorageError(e.to_string())),
        };

        if response.status_code == 404 {
            return Err(MediaError::NotFound);
        }
        if !(200..300).contains(&response.status_code) {
            return Err(MediaError::StorageError(format!(
                "S3 GET returned status {}",
                response.status_code
            )));
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

    /// Resolve a media payload according to the configured read layout.
    ///
    /// The dual-read-and-write phase prefers the sharded key and falls back to
    /// legacy only on an actual not-found result. Authorization, transport,
    /// throttling, and service errors are returned so fallback cannot mask an
    /// unhealthy store.
    pub async fn resolve_read_key(
        &self,
        ctx: &TenantContext,
        payload_name: &str,
    ) -> Result<String, MediaError> {
        let candidates = match crate::keys::read_candidates(ctx, payload_name) {
            Ok(candidates) => candidates,
            Err(_) => {
                metrics::counter!(
                    "buzz_media_s3_read_resolutions_total",
                    "result" => "missing"
                )
                .increment(1);
                return Err(MediaError::NotFound);
            }
        };

        if !self.migration_phase.reads_sharded() {
            return match self.head_with_metadata(&candidates.legacy).await {
                Ok(Some(_)) => {
                    metrics::counter!(
                        "buzz_media_s3_read_resolutions_total",
                        "result" => "legacy"
                    )
                    .increment(1);
                    Ok(candidates.legacy)
                }
                Ok(None) => {
                    metrics::counter!(
                        "buzz_media_s3_read_resolutions_total",
                        "result" => "missing"
                    )
                    .increment(1);
                    Err(MediaError::NotFound)
                }
                Err(error) => {
                    metrics::counter!(
                        "buzz_media_s3_read_resolutions_total",
                        "result" => "storage_error"
                    )
                    .increment(1);
                    Err(error)
                }
            };
        }

        match self.head_with_metadata(&candidates.sharded).await {
            Ok(Some(_)) => {
                metrics::counter!(
                    "buzz_media_s3_read_resolutions_total",
                    "result" => "sharded"
                )
                .increment(1);
                Ok(candidates.sharded)
            }
            Ok(None) if !self.migration_phase.reads_legacy() => {
                metrics::counter!(
                    "buzz_media_s3_read_resolutions_total",
                    "result" => "missing"
                )
                .increment(1);
                Err(MediaError::NotFound)
            }
            Ok(None) => {
                metrics::counter!("buzz_media_s3_read_fallbacks_total").increment(1);
                match self.head_with_metadata(&candidates.legacy).await {
                    Ok(Some(_)) => {
                        metrics::counter!(
                            "buzz_media_s3_read_resolutions_total",
                            "result" => "legacy"
                        )
                        .increment(1);
                        Ok(candidates.legacy)
                    }
                    Ok(None) => {
                        metrics::counter!(
                            "buzz_media_s3_read_resolutions_total",
                            "result" => "missing"
                        )
                        .increment(1);
                        Err(MediaError::NotFound)
                    }
                    Err(error) => {
                        metrics::counter!(
                            "buzz_media_s3_read_resolutions_total",
                            "result" => "storage_error"
                        )
                        .increment(1);
                        Err(error)
                    }
                }
            }
            Err(error) => {
                metrics::counter!(
                    "buzz_media_s3_read_resolutions_total",
                    "result" => "storage_error"
                )
                .increment(1);
                Err(error)
            }
        }
    }

    /// HEAD a media payload according to the configured read layout.
    pub async fn head_payload(
        &self,
        ctx: &TenantContext,
        payload_name: &str,
    ) -> Result<BlobHeadMeta, MediaError> {
        for candidate in self.ordered_read_candidates(ctx, payload_name)? {
            match self.head_with_metadata(&candidate.key).await {
                Ok(Some(meta)) => {
                    self.record_read_resolution(candidate.result);
                    return Ok(meta);
                }
                Ok(None) => {
                    self.record_candidate_missing(candidate);
                }
                Err(error) => {
                    self.record_read_resolution(ReadResolution::StorageError);
                    return Err(error);
                }
            }
        }

        self.record_read_resolution(ReadResolution::Missing);
        Err(MediaError::NotFound)
    }

    /// Stream a media payload by trying read-layout candidates with the real GET.
    ///
    /// A candidate that disappears between its metadata lookup and GET falls
    /// through to the next layout only on `NotFound`; storage/auth/transport
    /// errors are surfaced so fallback cannot mask an unhealthy store.
    pub async fn get_payload_stream(
        &self,
        ctx: &TenantContext,
        payload_name: &str,
    ) -> Result<BlobStream, MediaError> {
        for candidate in self.ordered_read_candidates(ctx, payload_name)? {
            let meta = match self.head_with_metadata(&candidate.key).await {
                Ok(Some(meta)) => meta,
                Ok(None) => {
                    self.record_candidate_missing(candidate);
                    continue;
                }
                Err(error) => {
                    self.record_read_resolution(ReadResolution::StorageError);
                    return Err(error);
                }
            };

            match self.get_stream(&candidate.key).await {
                Ok(stream) => {
                    self.record_read_resolution(candidate.result);
                    return Ok(BlobStream {
                        size: meta.size,
                        stream,
                    });
                }
                Err(MediaError::NotFound) => {
                    self.record_candidate_missing(candidate);
                }
                Err(error) => {
                    self.record_read_resolution(ReadResolution::StorageError);
                    return Err(error);
                }
            }
        }

        self.record_read_resolution(ReadResolution::Missing);
        Err(MediaError::NotFound)
    }

    /// Read a bounded byte range from a media payload by trying read-layout
    /// candidates with the real range GET.
    pub async fn get_payload_range(
        &self,
        ctx: &TenantContext,
        payload_name: &str,
        requested_range: PayloadByteRange,
        max_len: u64,
    ) -> Result<PayloadRangeRead, MediaError> {
        for candidate in self.ordered_read_candidates(ctx, payload_name)? {
            let total = match self.head_with_metadata(&candidate.key).await {
                Ok(Some(meta)) => meta.size,
                Ok(None) => {
                    self.record_candidate_missing(candidate);
                    continue;
                }
                Err(error) => {
                    self.record_read_resolution(ReadResolution::StorageError);
                    return Err(error);
                }
            };

            let Some((start, end)) = requested_range.bounds(total) else {
                self.record_read_resolution(candidate.result);
                return Ok(PayloadRangeRead::Unsatisfiable { total });
            };

            if start >= total {
                self.record_read_resolution(candidate.result);
                return Ok(PayloadRangeRead::Unsatisfiable { total });
            }

            let max_end = start.saturating_add(max_len.max(1).saturating_sub(1));
            let end = end.min(total.saturating_sub(1)).min(max_end);

            match self.get_range(&candidate.key, start, end).await {
                Ok(bytes) => {
                    self.record_read_resolution(candidate.result);
                    return Ok(PayloadRangeRead::Satisfiable {
                        total,
                        start,
                        end,
                        bytes,
                    });
                }
                Err(MediaError::NotFound) => {
                    self.record_candidate_missing(candidate);
                }
                Err(error) => {
                    self.record_read_resolution(ReadResolution::StorageError);
                    return Err(error);
                }
            }
        }

        self.record_read_resolution(ReadResolution::Missing);
        Err(MediaError::NotFound)
    }

    fn ordered_read_candidates(
        &self,
        ctx: &TenantContext,
        payload_name: &str,
    ) -> Result<Vec<ReadCandidate>, MediaError> {
        let candidates = crate::keys::read_candidates(ctx, payload_name).map_err(|_| {
            self.record_read_resolution(ReadResolution::Missing);
            MediaError::NotFound
        })?;

        let mut ordered = Vec::with_capacity(2);
        if self.migration_phase.reads_sharded() {
            ordered.push(ReadCandidate {
                key: candidates.sharded,
                result: ReadResolution::Sharded,
            });
        }
        if self.migration_phase.reads_legacy() {
            ordered.push(ReadCandidate {
                key: candidates.legacy,
                result: ReadResolution::Legacy,
            });
        }
        Ok(ordered)
    }

    fn record_candidate_missing(&self, candidate: ReadCandidate) {
        if matches!(candidate.result, ReadResolution::Sharded)
            && self.migration_phase.reads_legacy()
        {
            metrics::counter!("buzz_media_s3_read_fallbacks_total").increment(1);
        }
    }

    fn record_read_resolution(&self, resolution: ReadResolution) {
        let result = match resolution {
            ReadResolution::Sharded => "sharded",
            ReadResolution::Legacy => "legacy",
            ReadResolution::Missing => "missing",
            ReadResolution::StorageError => "storage_error",
        };
        metrics::counter!("buzz_media_s3_read_resolutions_total", "result" => result).increment(1);
    }

    /// Copy an object within the media bucket using an S3 server-side copy.
    pub async fn copy(&self, source: &str, destination: &str) -> Result<(), MediaError> {
        self.bucket
            .copy_object_internal(source, destination)
            .await
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        Ok(())
    }

    /// List one bounded page under a prefix for maintenance tools.
    pub async fn list_prefix_page(
        &self,
        prefix: &str,
        continuation_token: Option<String>,
        start_after: Option<String>,
        max_keys: usize,
    ) -> Result<crate::bucket_index::Page, MediaError> {
        let (result, _status) = self
            .bucket
            .list_page(
                prefix.to_string(),
                None,
                continuation_token,
                start_after,
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
        let (result, _status) = self
            .bucket
            .list_page(
                String::new(),
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

    fn storage_config(access: &str, secret: &str) -> crate::config::MediaConfig {
        crate::config::MediaConfig {
            s3_endpoint: "http://localhost:9000".to_string(),
            s3_access_key: access.to_string(),
            s3_secret_key: secret.to_string(),
            s3_bucket: "buzz-media".to_string(),
            s3_region: "us-west-2".to_string(),
            s3_addressing_style: S3AddressingStyle::Path,
            migration_phase: crate::config::MediaMigrationPhase::LegacyOnly,
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

#[derive(Debug, Clone)]
struct ReadCandidate {
    key: String,
    result: ReadResolution,
}

#[derive(Debug, Clone, Copy)]
enum ReadResolution {
    Sharded,
    Legacy,
    Missing,
    StorageError,
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
