//! Immutable media publication manifests and cache-only sidecar projection.
//!
//! Raw blobs, thumbnails, and this manifest are staged before PostgreSQL
//! authorization commits. Only a canonical operation receipt whose result
//! digest equals [`StagedMediaPublication::result_digest`] makes the media
//! visible. The legacy community sidecar is refreshed afterward as a cache;
//! it is never an authority witness.

use buzz_core::{CommunityId, TenantContext};
use bytes::Bytes;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::OwnedSemaphorePermit;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{BlobDescriptor, BlobMeta, MediaError, MediaStorage};

const MEDIA_PUBLICATION_VERSION: u8 = 1;
const PUBLICATION_PREFIX: &str = "_publications/media";
const MAX_PUBLICATION_BYTES: u64 = 16 * 1024;
const MAX_THUMBNAIL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PRIMARY_BYTES: u64 = 500 * 1024 * 1024;
const MAX_DURATION_SECONDS: f64 = 600.0;
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaObjectKind {
    Primary,
    Thumbnail,
}

/// A DB-witnessed media object whose complete bytes match the manifest digest.
///
/// Construction downloads the full immutable object into a private temporary
/// file and verifies its size and SHA-256 before this value is returned. HTTP
/// handlers can therefore form headers, full streams, or range responses only
/// from already-verified bytes; no object-store bytes escape while verification
/// is still in progress.
#[derive(Debug)]
pub struct VerifiedMediaObject {
    file: NamedTempFile,
    content_type: String,
    size: u64,
    _verification_slot: OwnedSemaphorePermit,
}

impl VerifiedMediaObject {
    /// Exact content type authenticated by the DB-witnessed manifest.
    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    /// Exact verified byte length.
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Read an inclusive byte range from the already-verified local copy.
    pub async fn read_range(&self, start: u64, end: u64) -> Result<Vec<u8>, MediaError> {
        if start > end || end >= self.size {
            return Err(invalid_publication());
        }
        let length = end
            .checked_sub(start)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(invalid_publication)?;
        let length = usize::try_from(length).map_err(|_| invalid_publication())?;
        let file = self
            .file
            .reopen()
            .map_err(|error| MediaError::Io(error.to_string()))?;
        let mut file = tokio::fs::File::from_std(file);
        file.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(|error| MediaError::Io(error.to_string()))?;
        let mut bytes = vec![0; length];
        file.read_exact(&mut bytes)
            .await
            .map_err(|error| MediaError::Io(error.to_string()))?;
        Ok(bytes)
    }

    /// Consume this object into a stream over the already-verified local copy.
    pub fn into_stream(self) -> Result<crate::ByteStream, MediaError> {
        let file = self
            .file
            .reopen()
            .map_err(|error| MediaError::Io(error.to_string()))?;
        Ok(Box::pin(VerifiedMediaStream {
            inner: ReaderStream::new(tokio::fs::File::from_std(file)),
            _file: self.file,
            _verification_slot: self._verification_slot,
        }))
    }
}

struct VerifiedMediaStream {
    inner: ReaderStream<tokio::fs::File>,
    _file: NamedTempFile,
    _verification_slot: OwnedSemaphorePermit,
}

impl Stream for VerifiedMediaStream {
    type Item = Result<Bytes, MediaError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(context).map(|item| {
            item.map(|result| result.map_err(|error| MediaError::Io(error.to_string())))
        })
    }
}

/// Canonical immutable description of one media publication.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaPublicationManifest {
    version: u8,
    community_id: Uuid,
    blob_sha256: String,
    extension: String,
    mime_type: String,
    size: u64,
    uploaded_at: i64,
    dimensions: Option<String>,
    blurhash: Option<String>,
    thumbnail_sha256: Option<String>,
    moderation_projection_sha256: Option<String>,
    duration_seconds_bits: Option<u64>,
}

impl MediaPublicationManifest {
    /// Test-only convenience for manifests without a moderation projection.
    /// Production staging always calls the explicit projection-aware builder.
    #[cfg(test)]
    pub(crate) fn from_staged_blob(
        tenant: &TenantContext,
        blob_sha256: impl Into<String>,
        metadata: &BlobMeta,
    ) -> Result<Self, MediaError> {
        Self::from_staged_blob_with_projection(tenant, blob_sha256, metadata, None)
    }

    /// Build a canonical manifest that binds a deterministic moderation plan.
    pub(crate) fn from_staged_blob_with_projection(
        tenant: &TenantContext,
        blob_sha256: impl Into<String>,
        metadata: &BlobMeta,
        moderation_projection_digest: Option<[u8; 32]>,
    ) -> Result<Self, MediaError> {
        let manifest = Self {
            version: MEDIA_PUBLICATION_VERSION,
            community_id: *tenant.community().as_uuid(),
            blob_sha256: blob_sha256.into(),
            extension: metadata.ext.clone(),
            mime_type: metadata.mime_type.clone(),
            size: metadata.size,
            uploaded_at: metadata.uploaded_at,
            dimensions: nonempty(metadata.dim.clone()),
            blurhash: nonempty(metadata.blurhash.clone()),
            thumbnail_sha256: metadata.thumbnail_sha256.clone(),
            moderation_projection_sha256: moderation_projection_digest.map(hex::encode),
            duration_seconds_bits: metadata.duration_secs.map(f64::to_bits),
        };
        manifest.validate()?;
        if manifest.thumbnail_sha256.is_some() == metadata.thumb_url.is_empty() {
            return Err(invalid_publication());
        }
        Ok(manifest)
    }

    /// Server-resolved community bound into the publication digest.
    pub const fn community_id(&self) -> CommunityId {
        CommunityId::from_uuid(self.community_id)
    }

    /// Exact content hash of the primary blob.
    pub fn blob_sha256(&self) -> &str {
        &self.blob_sha256
    }

    /// Exact content-addressed storage key of the primary blob.
    pub fn blob_key(&self) -> String {
        format!("{}.{}", self.blob_sha256, self.extension)
    }

    /// Canonical extension authenticated by this manifest.
    pub fn extension(&self) -> &str {
        &self.extension
    }

    /// Canonical MIME type authenticated by this manifest.
    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }

    /// Exact byte size authenticated by this manifest. Zero is valid only for
    /// the canonical empty octet-stream attachment.
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Optional exact thumbnail digest authenticated by this manifest.
    pub fn thumbnail_sha256(&self) -> Option<&str> {
        self.thumbnail_sha256.as_deref()
    }

    /// Optional deterministic moderation plan bound into this publication.
    pub fn moderation_projection_sha256(&self) -> Option<&str> {
        self.moderation_projection_sha256.as_deref()
    }

    fn thumbnail_blob_key(&self) -> Option<String> {
        self.thumbnail_sha256()
            .map(|digest| format!("{digest}.thumb.jpg"))
    }

    fn resolve_request(&self, requested_path: &str) -> Result<MediaObjectKind, MediaError> {
        let bare = self.blob_sha256();
        if requested_path == bare || requested_path == self.blob_key() {
            return Ok(MediaObjectKind::Primary);
        }
        if self.thumbnail_sha256.is_some() && requested_path == format!("{bare}.thumb.jpg") {
            return Ok(MediaObjectKind::Thumbnail);
        }
        Err(MediaError::NotFound)
    }

    /// Content-addressed manifest bytes used to derive the receipt digest.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, MediaError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(MediaError::from)
    }

    /// Reconstruct the exact upload response for an applied or replayed witness.
    pub fn descriptor(&self, public_base_url: &str) -> Result<BlobDescriptor, MediaError> {
        let metadata = self.to_blob_meta(public_base_url)?;
        Ok(BlobDescriptor {
            url: format!("{public_base_url}/{}", self.blob_key()),
            sha256: self.blob_sha256.clone(),
            size: self.size,
            mime_type: self.mime_type.clone(),
            uploaded: self.uploaded_at,
            dim: self.dimensions.clone(),
            blurhash: self.blurhash.clone(),
            thumb: (!metadata.thumb_url.is_empty()).then_some(metadata.thumb_url),
            duration: self.duration_seconds_bits.map(f64::from_bits),
        })
    }

    fn validate(&self) -> Result<(), MediaError> {
        let duration = self.duration_seconds_bits.map(f64::from_bits);
        if self.version != MEDIA_PUBLICATION_VERSION
            || self.community_id.is_nil()
            || !is_lower_sha256(&self.blob_sha256)
            || !is_safe_extension(&self.extension)
            || !is_exact_mime(&self.mime_type)
            || (self.size == 0
                && (self.blob_sha256 != EMPTY_SHA256
                    || self.extension != "bin"
                    || self.mime_type != "application/octet-stream"
                    || self.dimensions.is_some()
                    || self.blurhash.is_some()
                    || self.thumbnail_sha256.is_some()
                    || self.duration_seconds_bits.is_some()))
            || self.size > MAX_PRIMARY_BYTES
            || self.uploaded_at <= 0
            || self
                .dimensions
                .as_deref()
                .is_some_and(|value| !is_bounded_text(value, 64))
            || self
                .blurhash
                .as_deref()
                .is_some_and(|value| !is_bounded_text(value, 256))
            || self
                .thumbnail_sha256
                .as_deref()
                .is_some_and(|value| !is_lower_sha256(value))
            || self
                .moderation_projection_sha256
                .as_deref()
                .is_some_and(|value| !is_lower_sha256(value))
            || duration.is_some_and(|value| {
                !value.is_finite() || !(0.0..=MAX_DURATION_SECONDS).contains(&value)
            })
        {
            return Err(invalid_publication());
        }
        Ok(())
    }

    fn to_blob_meta(&self, public_base_url: &str) -> Result<BlobMeta, MediaError> {
        self.validate()?;
        if public_base_url.is_empty() || public_base_url != public_base_url.trim_end_matches('/') {
            return Err(invalid_publication());
        }
        Ok(BlobMeta {
            dim: self.dimensions.clone().unwrap_or_default(),
            blurhash: self.blurhash.clone().unwrap_or_default(),
            thumb_url: self
                .thumbnail_sha256
                .as_ref()
                .map_or_else(String::new, |_| {
                    format!("{public_base_url}/{}.thumb.jpg", self.blob_sha256)
                }),
            thumbnail_sha256: self.thumbnail_sha256.clone(),
            ext: self.extension.clone(),
            mime_type: self.mime_type.clone(),
            size: self.size,
            uploaded_at: self.uploaded_at,
            duration_secs: self.duration_seconds_bits.map(f64::from_bits),
        })
    }
}

/// Opaque proof that immutable publication bytes were durably staged.
#[derive(Clone)]
pub struct StagedMediaPublication {
    manifest: MediaPublicationManifest,
    manifest_key: String,
    result_digest: [u8; 32],
}

impl fmt::Debug for StagedMediaPublication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StagedMediaPublication([REDACTED])")
    }
}

impl StagedMediaPublication {
    /// Canonical manifest staged under the exact result digest.
    pub const fn manifest(&self) -> &MediaPublicationManifest {
        &self.manifest
    }

    /// Immutable manifest storage key.
    pub fn manifest_key(&self) -> &str {
        &self.manifest_key
    }

    /// Exact digest to commit as the protected-operation result.
    pub const fn result_digest(&self) -> [u8; 32] {
        self.result_digest
    }

    /// Refresh the legacy sidecar only after the DB witness is committed.
    ///
    /// Readers must still resolve and validate the PostgreSQL witness first;
    /// this sidecar is only a compatibility/cache projection.
    pub async fn publish_sidecar_cache(
        &self,
        storage: &MediaStorage,
        tenant: &TenantContext,
        public_base_url: &str,
    ) -> Result<(), MediaError> {
        if self.manifest.community_id() != tenant.community() {
            return Err(invalid_publication());
        }
        if let Some(thumbnail_digest) = self.manifest.thumbnail_sha256() {
            let immutable_key = self
                .manifest
                .thumbnail_blob_key()
                .ok_or_else(invalid_publication)?;
            let thumbnail = read_bounded_stream(
                storage.get_stream(&immutable_key).await?,
                MAX_THUMBNAIL_BYTES,
            )
            .await?;
            if hex::encode(Sha256::digest(&thumbnail)) != thumbnail_digest {
                return Err(invalid_publication());
            }
            let cache_key = format!("{}.thumb.jpg", self.manifest.blob_sha256());
            storage.put(&cache_key, &thumbnail, "image/jpeg").await?;
        }
        let metadata = self.manifest.to_blob_meta(public_base_url)?;
        storage
            .put_sidecar(tenant, self.manifest.blob_sha256(), &metadata)
            .await
    }
}

impl MediaStorage {
    /// Persist one immutable publication manifest without publishing visibility.
    pub(crate) async fn stage_media_publication(
        &self,
        manifest: MediaPublicationManifest,
    ) -> Result<StagedMediaPublication, MediaError> {
        let bytes = manifest.canonical_bytes()?;
        let result_digest: [u8; 32] = Sha256::digest(&bytes).into();
        if result_digest == [0; 32] {
            return Err(invalid_publication());
        }
        let manifest_key = media_publication_manifest_key(result_digest);
        put_immutable_publication(self, &manifest_key, &bytes).await?;
        Ok(StagedMediaPublication {
            manifest,
            manifest_key,
            result_digest,
        })
    }

    /// Fetch and digest-verify the immutable manifest named by a DB witness.
    pub async fn get_media_publication(
        &self,
        result_digest: [u8; 32],
    ) -> Result<MediaPublicationManifest, MediaError> {
        if result_digest == [0; 32] {
            return Err(invalid_publication());
        }
        let key = media_publication_manifest_key(result_digest);
        let size = self
            .head_with_metadata(&key)
            .await?
            .ok_or(MediaError::NotFound)?
            .size;
        if size == 0 || size > MAX_PUBLICATION_BYTES {
            return Err(invalid_publication());
        }
        let bytes =
            read_bounded_stream(self.get_stream(&key).await?, MAX_PUBLICATION_BYTES).await?;
        let actual: [u8; 32] = Sha256::digest(&bytes).into();
        if actual != result_digest {
            return Err(invalid_publication());
        }
        decode_media_publication(&bytes, result_digest)
    }

    /// Resolve a DB witness and fully verify the exact requested media bytes.
    ///
    /// `result_digest` must come from the canonical PostgreSQL publication
    /// witness. Sidecars are deliberately ignored. Tenant and request-path
    /// mismatches collapse to `NotFound`; storage absence or corruption never
    /// falls back to a raw object or sidecar.
    pub async fn verify_media_object(
        &self,
        tenant: &TenantContext,
        result_digest: [u8; 32],
        requested_path: &str,
    ) -> Result<VerifiedMediaObject, MediaError> {
        let manifest = self.get_media_publication(result_digest).await?;
        if manifest.community_id() != tenant.community() {
            return Err(MediaError::NotFound);
        }
        let kind = manifest.resolve_request(requested_path)?;
        let (key, expected_digest, expected_size, content_type) = match kind {
            MediaObjectKind::Primary => (
                manifest.blob_key(),
                manifest.blob_sha256().to_owned(),
                Some(manifest.size()),
                manifest.mime_type().to_owned(),
            ),
            MediaObjectKind::Thumbnail => {
                let thumbnail_digest = manifest
                    .thumbnail_sha256()
                    .ok_or_else(invalid_publication)?;
                (
                    manifest
                        .thumbnail_blob_key()
                        .ok_or_else(invalid_publication)?,
                    thumbnail_digest.to_owned(),
                    None,
                    "image/jpeg".to_owned(),
                )
            }
        };
        let verification_slot = self
            .verification_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                MediaError::StorageError("media verification capacity unavailable".to_owned())
            })?;
        let stream = self.get_stream(&key).await?;
        verify_media_stream(
            stream,
            &expected_digest,
            expected_size,
            content_type,
            verification_slot,
        )
        .await
    }
}

async fn put_immutable_publication(
    storage: &MediaStorage,
    key: &str,
    expected: &[u8],
) -> Result<(), MediaError> {
    storage
        .put_immutable_exact(key, expected, "application/json")
        .await?;
    Ok(())
}

fn decode_media_publication(
    bytes: &[u8],
    result_digest: [u8; 32],
) -> Result<MediaPublicationManifest, MediaError> {
    let manifest: MediaPublicationManifest = serde_json::from_slice(bytes)?;
    let canonical = manifest.canonical_bytes()?;
    let canonical_digest: [u8; 32] = Sha256::digest(&canonical).into();
    if canonical != bytes || canonical_digest != result_digest {
        return Err(invalid_publication());
    }
    Ok(manifest)
}

async fn verify_media_stream(
    mut stream: crate::ByteStream,
    expected_digest: &str,
    expected_size: Option<u64>,
    content_type: String,
    verification_slot: OwnedSemaphorePermit,
) -> Result<VerifiedMediaObject, MediaError> {
    let maximum_size = expected_size.unwrap_or(MAX_THUMBNAIL_BYTES);
    let file = NamedTempFile::new().map_err(|error| MediaError::Io(error.to_string()))?;
    let writer = file
        .reopen()
        .map_err(|error| MediaError::Io(error.to_string()))?;
    let mut writer = tokio::fs::File::from_std(writer);
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        size = size
            .checked_add(u64::try_from(chunk.len()).map_err(|_| invalid_publication())?)
            .ok_or_else(invalid_publication)?;
        if size > maximum_size {
            return Err(invalid_publication());
        }
        digest.update(&chunk);
        writer
            .write_all(&chunk)
            .await
            .map_err(|error| MediaError::Io(error.to_string()))?;
    }
    writer
        .flush()
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?;
    if (size == 0 && expected_size.is_none())
        || expected_size.is_some_and(|expected| expected != size)
        || hex::encode(digest.finalize()) != expected_digest
    {
        return Err(invalid_publication());
    }
    Ok(VerifiedMediaObject {
        file,
        content_type,
        size,
        _verification_slot: verification_slot,
    })
}

async fn read_bounded_stream(
    mut stream: crate::ByteStream,
    maximum_size: u64,
) -> Result<Vec<u8>, MediaError> {
    let mut bytes = Vec::new();
    let mut size = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        size = size
            .checked_add(u64::try_from(chunk.len()).map_err(|_| invalid_publication())?)
            .ok_or_else(invalid_publication)?;
        if size > maximum_size {
            return Err(invalid_publication());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err(invalid_publication());
    }
    Ok(bytes)
}

/// Exact immutable manifest key for a protected-operation result digest.
pub fn media_publication_manifest_key(result_digest: [u8; 32]) -> String {
    format!("{PUBLICATION_PREFIX}/{}.json", hex::encode(result_digest))
}

fn nonempty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_safe_extension(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

fn is_exact_mime(value: &str) -> bool {
    is_bounded_text(value, 255)
        && value.contains('/')
        && !value.contains(',')
        && value.bytes().all(|byte| byte.is_ascii_graphic())
}

fn is_bounded_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value == value.trim()
        && !value.chars().any(char::is_control)
}

fn invalid_publication() -> MediaError {
    MediaError::StorageError("invalid immutable media publication".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant(id: u128) -> TenantContext {
        TenantContext::resolved(
            CommunityId::from_uuid(Uuid::from_u128(id)),
            format!("{id}.example"),
        )
    }

    fn metadata() -> BlobMeta {
        BlobMeta {
            dim: "800x600".into(),
            blurhash: "LEHV6nWB2yk8pyo0adR*.7kCMdnj".into(),
            thumb_url: "https://media.example/aaaaaaaa.thumb.jpg".into(),
            thumbnail_sha256: Some("b".repeat(64)),
            ext: "jpg".into(),
            mime_type: "image/jpeg".into(),
            size: 42,
            uploaded_at: 1_800_000_000,
            duration_secs: None,
        }
    }

    #[test]
    fn canonical_manifest_is_deterministic_and_domain_bound() {
        let sha = "a".repeat(64);
        let one = MediaPublicationManifest::from_staged_blob(&tenant(1), &sha, &metadata())
            .expect("manifest");
        let again = MediaPublicationManifest::from_staged_blob(&tenant(1), &sha, &metadata())
            .expect("manifest");
        let other = MediaPublicationManifest::from_staged_blob(&tenant(2), &sha, &metadata())
            .expect("manifest");
        let one_bytes = one.canonical_bytes().expect("canonical bytes");
        assert_eq!(one_bytes, again.canonical_bytes().expect("canonical bytes"));
        assert_ne!(one_bytes, other.canonical_bytes().expect("canonical bytes"));
        assert_eq!(one.blob_key(), format!("{sha}.jpg"));
        let expected_thumbnail_key = format!("{}.thumb.jpg", "b".repeat(64));
        assert_eq!(
            one.thumbnail_blob_key().as_deref(),
            Some(expected_thumbnail_key.as_str())
        );
    }

    #[test]
    fn exact_metadata_changes_the_publication_digest() {
        let sha = "a".repeat(64);
        let original = MediaPublicationManifest::from_staged_blob(&tenant(1), &sha, &metadata())
            .expect("manifest");
        let mut changed = metadata();
        changed.mime_type = "image/png".into();
        let changed = MediaPublicationManifest::from_staged_blob(&tenant(1), &sha, &changed)
            .expect("manifest");
        let digest = |manifest: &MediaPublicationManifest| -> [u8; 32] {
            Sha256::digest(manifest.canonical_bytes().expect("canonical bytes")).into()
        };
        assert_ne!(digest(&original), digest(&changed));
    }

    #[test]
    fn zero_byte_primary_has_a_canonical_manifest_and_descriptor() {
        let mut empty = metadata();
        empty.dim.clear();
        empty.blurhash.clear();
        empty.thumb_url.clear();
        empty.thumbnail_sha256 = None;
        empty.ext = "bin".into();
        empty.mime_type = "application/octet-stream".into();
        empty.size = 0;
        let digest = hex::encode(Sha256::digest([]));

        let manifest = MediaPublicationManifest::from_staged_blob(&tenant(1), &digest, &empty)
            .expect("empty primary manifest");
        let descriptor = manifest
            .descriptor("https://media.example")
            .expect("empty primary descriptor");

        assert_eq!(manifest.size(), 0);
        assert_eq!(descriptor.size, 0);
        assert_eq!(descriptor.sha256, digest);
        assert_eq!(descriptor.mime_type, "application/octet-stream");
    }

    #[test]
    fn moderation_projection_is_bound_into_publication_digest() {
        let sha = "a".repeat(64);
        let without = MediaPublicationManifest::from_staged_blob(&tenant(1), &sha, &metadata())
            .expect("manifest without projection");
        let with = MediaPublicationManifest::from_staged_blob_with_projection(
            &tenant(1),
            &sha,
            &metadata(),
            Some([0x42; 32]),
        )
        .expect("manifest with projection");
        let expected_projection = hex::encode([0x42; 32]);
        assert_eq!(
            with.moderation_projection_sha256(),
            Some(expected_projection.as_str())
        );
        assert_ne!(
            without.canonical_bytes().expect("canonical bytes"),
            with.canonical_bytes().expect("canonical bytes")
        );
    }

    #[test]
    fn manifest_rejects_ambiguous_or_unverifiable_metadata() {
        let sha = "a".repeat(64);
        let mut invalid = metadata();
        invalid.ext = "../jpg".into();
        assert!(MediaPublicationManifest::from_staged_blob(&tenant(1), &sha, &invalid).is_err());

        let mut missing_thumbnail_digest = metadata();
        missing_thumbnail_digest.thumbnail_sha256 = None;
        assert!(MediaPublicationManifest::from_staged_blob(
            &tenant(1),
            &sha,
            &missing_thumbnail_digest
        )
        .is_err());

        let mut non_finite_duration = metadata();
        non_finite_duration.thumb_url.clear();
        non_finite_duration.thumbnail_sha256 = None;
        non_finite_duration.duration_secs = Some(f64::NAN);
        assert!(
            MediaPublicationManifest::from_staged_blob(&tenant(1), &sha, &non_finite_duration)
                .is_err()
        );

        let mut oversized = metadata();
        oversized.size = MAX_PRIMARY_BYTES + 1;
        assert!(MediaPublicationManifest::from_staged_blob(&tenant(1), &sha, &oversized).is_err());

        let mut false_empty = metadata();
        false_empty.size = 0;
        false_empty.thumb_url.clear();
        false_empty.thumbnail_sha256 = None;
        assert!(
            MediaPublicationManifest::from_staged_blob(&tenant(1), &sha, &false_empty).is_err()
        );
    }

    #[test]
    fn manifest_round_trip_reconstructs_cache_metadata() {
        let manifest =
            MediaPublicationManifest::from_staged_blob(&tenant(1), "a".repeat(64), &metadata())
                .expect("manifest");
        let bytes = manifest.canonical_bytes().expect("bytes");
        let decoded: MediaPublicationManifest = serde_json::from_slice(&bytes).expect("decode");
        decoded.validate().expect("valid decoded manifest");
        let cached = decoded
            .to_blob_meta("https://media.example")
            .expect("cache metadata");
        assert_eq!(cached.thumbnail_sha256, Some("b".repeat(64)));
        assert_eq!(
            cached.thumb_url,
            format!("https://media.example/{}.thumb.jpg", "a".repeat(64))
        );
        let descriptor = decoded
            .descriptor("https://media.example")
            .expect("replay descriptor");
        assert_eq!(
            descriptor.url,
            format!("https://media.example/{}.jpg", "a".repeat(64))
        );
        assert_eq!(descriptor.thumb, Some(cached.thumb_url));
    }

    #[test]
    fn publication_decode_requires_exact_canonical_witness_bytes() {
        let manifest =
            MediaPublicationManifest::from_staged_blob(&tenant(1), "a".repeat(64), &metadata())
                .expect("manifest");
        let canonical = manifest.canonical_bytes().expect("canonical bytes");
        let digest: [u8; 32] = Sha256::digest(&canonical).into();
        assert_eq!(
            decode_media_publication(&canonical, digest).expect("canonical witness"),
            manifest
        );

        let noncanonical = serde_json::to_vec_pretty(&manifest).expect("pretty bytes");
        let noncanonical_digest: [u8; 32] = Sha256::digest(&noncanonical).into();
        assert!(decode_media_publication(&noncanonical, noncanonical_digest).is_err());
        assert!(decode_media_publication(&canonical, [0x55; 32]).is_err());
    }

    #[test]
    fn manifest_key_is_exact_digest_address() {
        assert_eq!(
            media_publication_manifest_key([0xab; 32]),
            format!("{PUBLICATION_PREFIX}/{}.json", "ab".repeat(32))
        );
    }

    #[test]
    fn request_resolution_is_exact_and_sidecar_independent() {
        let sha = "a".repeat(64);
        let manifest = MediaPublicationManifest::from_staged_blob(&tenant(1), &sha, &metadata())
            .expect("manifest");
        assert_eq!(
            manifest.resolve_request(&sha).expect("bare primary"),
            MediaObjectKind::Primary
        );
        assert_eq!(
            manifest
                .resolve_request(&format!("{sha}.jpg"))
                .expect("exact primary"),
            MediaObjectKind::Primary
        );
        assert_eq!(
            manifest
                .resolve_request(&format!("{sha}.thumb.jpg"))
                .expect("exact thumbnail"),
            MediaObjectKind::Thumbnail
        );
        for rejected in [
            format!("{sha}.png"),
            format!("{sha}.JPG"),
            format!("{sha}.thumb.jpeg"),
            format!("{sha}.jpg/extra"),
        ] {
            assert!(matches!(
                manifest.resolve_request(&rejected),
                Err(MediaError::NotFound)
            ));
        }
    }

    #[tokio::test]
    async fn complete_bytes_are_verified_before_a_stream_is_returned() {
        let bytes = Bytes::from_static(b"canonical media bytes");
        let digest = hex::encode(Sha256::digest(&bytes));
        let stream = futures_util::stream::iter([Ok(bytes.clone())]);
        let verified = verify_media_stream(
            Box::pin(stream),
            &digest,
            Some(bytes.len() as u64),
            "application/octet-stream".into(),
            std::sync::Arc::new(tokio::sync::Semaphore::new(1))
                .try_acquire_owned()
                .expect("verification slot"),
        )
        .await
        .expect("verified object");
        assert_eq!(verified.size(), bytes.len() as u64);
        assert_eq!(
            verified.read_range(1, 4).await.expect("verified range"),
            b"anon"
        );

        for (expected_digest, expected_size) in [
            ("00".repeat(32), Some(bytes.len() as u64)),
            (digest.clone(), Some(bytes.len() as u64 + 1)),
        ] {
            let stream = futures_util::stream::iter([Ok(bytes.clone())]);
            assert!(verify_media_stream(
                Box::pin(stream),
                &expected_digest,
                expected_size,
                "application/octet-stream".into(),
                std::sync::Arc::new(tokio::sync::Semaphore::new(1))
                    .try_acquire_owned()
                    .expect("verification slot"),
            )
            .await
            .is_err());
        }
    }

    #[tokio::test]
    async fn empty_primary_is_verified_but_empty_derived_objects_are_rejected() {
        let empty_digest = hex::encode(Sha256::digest([]));
        let empty_stream = futures_util::stream::empty();
        let verified = verify_media_stream(
            Box::pin(empty_stream),
            &empty_digest,
            Some(0),
            "application/octet-stream".into(),
            std::sync::Arc::new(tokio::sync::Semaphore::new(1))
                .try_acquire_owned()
                .expect("verification slot"),
        )
        .await
        .expect("empty primary");
        assert_eq!(verified.size(), 0);
        let mut stream = verified.into_stream().expect("verified stream");
        assert!(stream.next().await.is_none());

        let empty_thumbnail = futures_util::stream::empty();
        assert!(verify_media_stream(
            Box::pin(empty_thumbnail),
            &empty_digest,
            None,
            "image/jpeg".into(),
            std::sync::Arc::new(tokio::sync::Semaphore::new(1))
                .try_acquire_owned()
                .expect("verification slot"),
        )
        .await
        .is_err());

        let wrong_digest = futures_util::stream::empty();
        assert!(verify_media_stream(
            Box::pin(wrong_digest),
            &"00".repeat(32),
            Some(0),
            "application/octet-stream".into(),
            std::sync::Arc::new(tokio::sync::Semaphore::new(1))
                .try_acquire_owned()
                .expect("verification slot"),
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn manifest_stream_read_is_bounded_independently_of_head() {
        let chunks = [
            Ok(Bytes::from_static(b"1234")),
            Ok(Bytes::from_static(b"5678")),
        ];
        assert_eq!(
            read_bounded_stream(Box::pin(futures_util::stream::iter(chunks)), 8)
                .await
                .expect("exact bound"),
            b"12345678"
        );

        let oversized = [
            Ok(Bytes::from_static(b"1234")),
            Ok(Bytes::from_static(b"56789")),
        ];
        assert!(
            read_bounded_stream(Box::pin(futures_util::stream::iter(oversized)), 8)
                .await
                .is_err()
        );
    }
}
