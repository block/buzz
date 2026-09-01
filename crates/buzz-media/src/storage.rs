//! S3/MinIO storage client.

use std::collections::HashMap;
use std::path::Path;
use std::pin::Pin;

use buzz_core::tenant::{CommunityId, TenantContext};

use crate::config::{MediaConfig, MediaMigrationPhase, S3AddressingStyle};
use crate::error::MediaError;
use bytes::Bytes;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use s3::creds::Credentials;
use s3::request::Request as _;
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

/// The kind of versioned S3 object-store entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectVersionKind {
    /// A concrete object version with bytes.
    Object,
    /// A delete-marker version hiding older bytes from live-object listing.
    DeleteMarker,
}

/// One S3 object version or delete marker under a tenant prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectVersionEntry {
    /// Object key.
    pub key: String,
    /// Concrete S3 version id.
    pub version_id: String,
    /// Whether this entry is a byte-bearing object or delete marker.
    pub kind: ObjectVersionKind,
    /// Byte size for object versions; zero for delete markers.
    pub size: u64,
}

/// Exact version identifier used for permanent deletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectVersionRef {
    /// Object key.
    pub key: String,
    /// Concrete S3 version id.
    pub version_id: String,
}

/// One `ListObjectVersions` page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectVersionsPage {
    /// Object versions and delete markers returned by this page.
    pub entries: Vec<ObjectVersionEntry>,
    /// Next key marker for truncated listings.
    pub next_key_marker: Option<String>,
    /// Next version-id marker for truncated listings.
    pub next_version_id_marker: Option<String>,
    /// Whether more pages remain.
    pub is_truncated: bool,
}

#[derive(Debug, Default)]
struct ListVersionFields {
    key: Option<String>,
    version_id: Option<String>,
    size: Option<u64>,
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn xml_error(error: impl std::fmt::Display) -> MediaError {
    MediaError::StorageError(error.to_string())
}

fn read_element_text(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart<'_>,
) -> Result<String, MediaError> {
    reader
        .read_text(start.to_end().name())
        .map(|text| text.into_owned())
        .map_err(xml_error)
}

fn skip_element(reader: &mut Reader<&[u8]>, start: &BytesStart<'_>) -> Result<(), MediaError> {
    reader
        .read_to_end(start.to_end().name())
        .map_err(xml_error)?;
    Ok(())
}

fn parse_list_version_entry(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart<'_>,
    kind: ObjectVersionKind,
) -> Result<ObjectVersionEntry, MediaError> {
    let mut fields = ListVersionFields::default();
    loop {
        match reader.read_event().map_err(xml_error)? {
            Event::Start(child) => match local_name(child.local_name().as_ref()) {
                b"Key" => fields.key = Some(read_element_text(reader, &child)?),
                b"VersionId" => fields.version_id = Some(read_element_text(reader, &child)?),
                b"Size" => {
                    let size = read_element_text(reader, &child)?;
                    fields.size = Some(size.parse::<u64>().map_err(xml_error)?);
                }
                _ => skip_element(reader, &child)?,
            },
            Event::Empty(child) => match local_name(child.local_name().as_ref()) {
                b"Key" => fields.key = Some(String::new()),
                b"VersionId" => fields.version_id = Some(String::new()),
                b"Size" => fields.size = Some(0),
                _ => {}
            },
            Event::End(end) if end.name().as_ref() == start.to_end().name().as_ref() => {
                let key = fields.key.ok_or_else(|| {
                    MediaError::StorageError("ListObjectVersions entry missing Key".to_string())
                })?;
                let version_id = fields.version_id.ok_or_else(|| {
                    MediaError::StorageError(
                        "ListObjectVersions entry missing VersionId".to_string(),
                    )
                })?;
                return Ok(ObjectVersionEntry {
                    key,
                    version_id,
                    kind,
                    size: if kind == ObjectVersionKind::Object {
                        fields.size.unwrap_or(0)
                    } else {
                        0
                    },
                });
            }
            Event::Eof => {
                return Err(MediaError::StorageError(
                    "unexpected EOF inside ListObjectVersions entry".to_string(),
                ));
            }
            _ => {}
        }
    }
}

fn parse_object_versions_page(xml: &[u8]) -> Result<ObjectVersionsPage, MediaError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut entries = Vec::new();
    let mut next_key_marker = None;
    let mut next_version_id_marker = None;
    let mut is_truncated = false;

    loop {
        match reader.read_event().map_err(xml_error)? {
            Event::Start(start) => match local_name(start.local_name().as_ref()) {
                b"Version" => entries.push(parse_list_version_entry(
                    &mut reader,
                    &start,
                    ObjectVersionKind::Object,
                )?),
                b"DeleteMarker" => entries.push(parse_list_version_entry(
                    &mut reader,
                    &start,
                    ObjectVersionKind::DeleteMarker,
                )?),
                b"IsTruncated" => {
                    let value = read_element_text(&mut reader, &start)?;
                    is_truncated = value.eq_ignore_ascii_case("true");
                }
                b"NextKeyMarker" => {
                    next_key_marker = Some(read_element_text(&mut reader, &start)?);
                }
                b"NextVersionIdMarker" => {
                    next_version_id_marker = Some(read_element_text(&mut reader, &start)?);
                }
                b"ListVersionsResult" => {}
                _ => skip_element(&mut reader, &start)?,
            },
            Event::Empty(start) => match local_name(start.local_name().as_ref()) {
                b"NextKeyMarker" => next_key_marker = Some(String::new()),
                b"NextVersionIdMarker" => next_version_id_marker = Some(String::new()),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(ObjectVersionsPage {
        entries,
        next_key_marker,
        next_version_id_marker,
        is_truncated,
    })
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
        _ctx: &TenantContext,
        sha256: &str,
        ext: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<String, MediaError> {
        let legacy = crate::keys::legacy_blob_key(sha256, ext)
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        let sharded = crate::keys::sharded_blob_key(sha256, ext)
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
        _ctx: &TenantContext,
        sha256: &str,
        ext: &str,
        path: &Path,
        content_type: &str,
    ) -> Result<String, MediaError> {
        let legacy = crate::keys::legacy_blob_key(sha256, ext)
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        let sharded = crate::keys::sharded_blob_key(sha256, ext)
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
        _ctx: &TenantContext,
        sha256: &str,
        bytes: &[u8],
    ) -> Result<String, MediaError> {
        let legacy = crate::keys::legacy_thumb_key(sha256)
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        let sharded = crate::keys::sharded_thumb_key(sha256)
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
        self.delete_object_identifiers(identifiers).await
    }

    /// Non-destructively verify that versioned bucket APIs are reachable.
    ///
    /// `ListObjectVersions` can be proven without mutation. S3 has no equivalent
    /// dry-run for `DeleteObjectVersion`: `DeleteObjects` is always destructive,
    /// even for exact versions, and deleting a fabricated version id does not
    /// prove permission when policies can be prefix- or tag-constrained.
    /// Operators must still provision `s3:DeleteObjectVersion`; the first exact
    /// version deletion remains the destructive proof.
    pub async fn preflight_version_listing(&self, prefix: &str) -> Result<(), MediaError> {
        self.list_prefix_versions_page(prefix, None, None, 1)
            .await
            .map(|_| ())
    }

    /// Bulk-delete exact object versions via S3 `DeleteObjects`.
    ///
    /// Every identifier includes a version id, so this removes historical
    /// versions and delete markers permanently instead of adding another
    /// delete marker to a versioned bucket.
    pub async fn delete_object_versions(
        &self,
        versions: &[ObjectVersionRef],
    ) -> Result<BulkDeleteOutcome, MediaError> {
        self.delete_object_versions_with_folding(versions, fold_version_delete_result)
            .await
    }

    async fn delete_object_versions_with_folding(
        &self,
        versions: &[ObjectVersionRef],
        fold: fn(s3::serde_types::DeleteObjectsResult) -> BulkDeleteOutcome,
    ) -> Result<BulkDeleteOutcome, MediaError> {
        if versions.is_empty() {
            return Ok(BulkDeleteOutcome::default());
        }
        let identifiers = object_version_identifiers(versions);
        let result = self
            .bucket
            .delete_objects(identifiers)
            .await
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        Ok(fold(result))
    }

    async fn delete_object_identifiers(
        &self,
        identifiers: Vec<s3::serde_types::ObjectIdentifier>,
    ) -> Result<BulkDeleteOutcome, MediaError> {
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
        self.list_prefix_page("", continuation_token, None, max_keys)
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

    /// One page of object versions and delete markers under a prefix.
    ///
    /// This uses S3 `ListObjectVersions` (`?versions`) instead of
    /// `ListObjectsV2`: versioned buckets can be logically empty while still
    /// retaining historical versions or delete markers, and permanent deletion
    /// must enumerate both. Pagination must carry both `KeyMarker` and
    /// `VersionIdMarker`; carrying only the key marker can skip siblings when a
    /// key has multiple versions on a page boundary.
    pub async fn list_prefix_versions_page(
        &self,
        prefix: &str,
        key_marker: Option<String>,
        version_id_marker: Option<String>,
        max_keys: usize,
    ) -> Result<ObjectVersionsPage, MediaError> {
        let mut query = HashMap::from([
            ("versions".to_string(), String::new()),
            ("prefix".to_string(), prefix.to_string()),
            ("max-keys".to_string(), max_keys.to_string()),
        ]);
        if let Some(marker) = key_marker {
            query.insert("key-marker".to_string(), marker);
        }
        if let Some(marker) = version_id_marker {
            query.insert("version-id-marker".to_string(), marker);
        }
        let bucket = self
            .bucket
            .with_extra_query(query)
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        let request = s3::request::tokio_backend::ReqwestRequest::new(
            &bucket,
            "/",
            s3::command::Command::GetObject,
        )
        .await
        .map_err(|e| MediaError::StorageError(e.to_string()))?;
        let response = request
            .response_data(false)
            .await
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        if response.status_code() >= 300 {
            return Err(MediaError::StorageError(format!(
                "list object versions failed with status {}: {}",
                response.status_code(),
                response.as_str().unwrap_or("")
            )));
        }
        parse_object_versions_page(response.as_slice())
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

fn object_version_identifiers(
    versions: &[ObjectVersionRef],
) -> Vec<s3::serde_types::ObjectIdentifier> {
    versions
        .iter()
        .map(|version| {
            s3::serde_types::ObjectIdentifier::with_version(
                version.key.clone(),
                version.version_id.clone(),
            )
        })
        .collect()
}

fn fold_bulk_delete_result(result: s3::serde_types::DeleteObjectsResult) -> BulkDeleteOutcome {
    fold_delete_result(result, DeleteMode::Unversioned)
}

fn fold_version_delete_result(result: s3::serde_types::DeleteObjectsResult) -> BulkDeleteOutcome {
    fold_delete_result(result, DeleteMode::ExplicitVersion)
}

enum DeleteMode {
    Unversioned,
    ExplicitVersion,
}

fn fold_delete_result(
    result: s3::serde_types::DeleteObjectsResult,
    mode: DeleteMode,
) -> BulkDeleteOutcome {
    let mut outcome = BulkDeleteOutcome::default();
    for deleted in result.deleted {
        let has_version_artifact = deleted.delete_marker == Some(true)
            || deleted.delete_marker_version_id.is_some()
            || deleted.version_id.is_some();
        match mode {
            DeleteMode::Unversioned if has_version_artifact => {
                outcome.versioned_keys.push(deleted.key);
            }
            DeleteMode::Unversioned | DeleteMode::ExplicitVersion => {
                outcome.deleted += 1;
            }
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

    #[test]
    fn version_delete_fold_counts_explicit_version_artifacts_as_deleted() {
        use s3::serde_types::{DeleteError, DeleteObjectsResult, DeletedObject};
        let result = DeleteObjectsResult {
            deleted: vec![DeletedObject {
                key: "versioned".to_string(),
                version_id: Some("v1".to_string()),
                delete_marker: Some(true),
                delete_marker_version_id: Some("v1".to_string()),
            }],
            errors: vec![
                DeleteError {
                    key: "retried-version".to_string(),
                    code: "NoSuchVersion".to_string(),
                    message: "already absent".to_string(),
                    version_id: Some("v-gone".to_string()),
                },
                DeleteError {
                    key: "denied-version".to_string(),
                    code: "AccessDenied".to_string(),
                    message: "denied".to_string(),
                    version_id: Some("v-denied".to_string()),
                },
            ],
        };

        let outcome = fold_version_delete_result(result);
        assert_eq!(outcome.deleted, 1);
        assert_eq!(outcome.already_missing, 1);
        assert!(outcome.versioned_keys.is_empty());
        assert_eq!(
            outcome.failed,
            vec![(
                "denied-version".to_string(),
                "AccessDenied".to_string(),
                "denied".to_string()
            )]
        );
    }

    #[test]
    fn object_version_identifiers_include_explicit_version_ids() {
        let identifiers = object_version_identifiers(&[
            ObjectVersionRef {
                key: "_meta/tenant/a.json".to_string(),
                version_id: "v-object".to_string(),
            },
            ObjectVersionRef {
                key: "uploads/tenant/event/blob".to_string(),
                version_id: "v-delete-marker".to_string(),
            },
        ]);

        assert_eq!(identifiers.len(), 2);
        assert_eq!(identifiers[0].key, "_meta/tenant/a.json");
        assert_eq!(identifiers[0].version_id.as_deref(), Some("v-object"));
        assert_eq!(identifiers[1].key, "uploads/tenant/event/blob");
        assert_eq!(
            identifiers[1].version_id.as_deref(),
            Some("v-delete-marker")
        );
    }

    #[tokio::test]
    async fn delete_object_versions_empty_input_short_circuits_before_folding() {
        let storage = MediaStorage::new(&storage_config("buzz_dev", "buzz_dev_secret"))
            .expect("static client");
        let outcome = storage
            .delete_object_versions_with_folding(&[], |_| BulkDeleteOutcome {
                deleted: 0,
                already_missing: 0,
                versioned_keys: vec!["wrong-fold".to_string()],
                failed: Vec::new(),
            })
            .await
            .expect("empty delete short-circuits before fold");
        assert_eq!(outcome, BulkDeleteOutcome::default());
    }

    #[test]
    fn parse_object_versions_page_includes_objects_delete_markers_and_dual_markers() {
        let page = parse_object_versions_page(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<ListVersionsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>buzz-media</Name>
  <Prefix>_meta/tenant/</Prefix>
  <KeyMarker>_meta/tenant/a.json</KeyMarker>
  <VersionIdMarker>v-old</VersionIdMarker>
  <MaxKeys>2</MaxKeys>
  <IsTruncated>true</IsTruncated>
  <NextKeyMarker>_meta/tenant/a.json</NextKeyMarker>
  <NextVersionIdMarker>v-new</NextVersionIdMarker>
  <DeleteMarker>
    <Key>_meta/tenant/a.json</Key>
    <VersionId>v-delete</VersionId>
    <IsLatest>true</IsLatest>
  </DeleteMarker>
  <Version>
    <Key>_meta/tenant/a.json</Key>
    <VersionId>v-new</VersionId>
    <IsLatest>false</IsLatest>
    <Size>42</Size>
  </Version>
</ListVersionsResult>"#,
        )
        .expect("parse versions page");

        assert!(page.is_truncated);
        assert_eq!(page.next_key_marker.as_deref(), Some("_meta/tenant/a.json"));
        assert_eq!(page.next_version_id_marker.as_deref(), Some("v-new"));
        assert_eq!(
            page.entries,
            vec![
                ObjectVersionEntry {
                    key: "_meta/tenant/a.json".to_string(),
                    version_id: "v-delete".to_string(),
                    kind: ObjectVersionKind::DeleteMarker,
                    size: 0,
                },
                ObjectVersionEntry {
                    key: "_meta/tenant/a.json".to_string(),
                    version_id: "v-new".to_string(),
                    kind: ObjectVersionKind::Object,
                    size: 42,
                },
            ]
        );
    }

    #[test]
    fn parse_object_versions_page_preserves_repeated_interleaved_aws_ordering() {
        let page = parse_object_versions_page(
            br#"<ListVersionsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Version><Key>k-a</Key><VersionId>v3</VersionId><Size>3</Size></Version>
  <DeleteMarker><Key>k-a</Key><VersionId>v2</VersionId></DeleteMarker>
  <Version><Key>k-a</Key><VersionId>v1</VersionId><Size>1</Size></Version>
  <DeleteMarker><Key>k-b</Key><VersionId>m2</VersionId></DeleteMarker>
  <Version><Key>k-b</Key><VersionId>m1</VersionId><Size>10</Size></Version>
</ListVersionsResult>"#,
        )
        .expect("parse interleaved versions page");

        assert_eq!(
            page.entries,
            vec![
                ObjectVersionEntry {
                    key: "k-a".to_string(),
                    version_id: "v3".to_string(),
                    kind: ObjectVersionKind::Object,
                    size: 3,
                },
                ObjectVersionEntry {
                    key: "k-a".to_string(),
                    version_id: "v2".to_string(),
                    kind: ObjectVersionKind::DeleteMarker,
                    size: 0,
                },
                ObjectVersionEntry {
                    key: "k-a".to_string(),
                    version_id: "v1".to_string(),
                    kind: ObjectVersionKind::Object,
                    size: 1,
                },
                ObjectVersionEntry {
                    key: "k-b".to_string(),
                    version_id: "m2".to_string(),
                    kind: ObjectVersionKind::DeleteMarker,
                    size: 0,
                },
                ObjectVersionEntry {
                    key: "k-b".to_string(),
                    version_id: "m1".to_string(),
                    kind: ObjectVersionKind::Object,
                    size: 10,
                },
            ]
        );
    }

    #[test]
    fn parse_object_versions_page_handles_marker_only_key_before_versioned_key() {
        let page = parse_object_versions_page(
            br#"<ListVersionsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <DeleteMarker><Key>k-marker-only</Key><VersionId>d-only</VersionId></DeleteMarker>
  <Version><Key>k-versioned</Key><VersionId>v2</VersionId><Size>20</Size></Version>
  <DeleteMarker><Key>k-versioned</Key><VersionId>d1</VersionId></DeleteMarker>
  <Version><Key>k-versioned</Key><VersionId>v1</VersionId><Size>10</Size></Version>
</ListVersionsResult>"#,
        )
        .expect("parse marker-only and versioned keys");

        assert_eq!(
            page.entries,
            vec![
                ObjectVersionEntry {
                    key: "k-marker-only".to_string(),
                    version_id: "d-only".to_string(),
                    kind: ObjectVersionKind::DeleteMarker,
                    size: 0,
                },
                ObjectVersionEntry {
                    key: "k-versioned".to_string(),
                    version_id: "v2".to_string(),
                    kind: ObjectVersionKind::Object,
                    size: 20,
                },
                ObjectVersionEntry {
                    key: "k-versioned".to_string(),
                    version_id: "d1".to_string(),
                    kind: ObjectVersionKind::DeleteMarker,
                    size: 0,
                },
                ObjectVersionEntry {
                    key: "k-versioned".to_string(),
                    version_id: "v1".to_string(),
                    kind: ObjectVersionKind::Object,
                    size: 10,
                },
            ]
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
