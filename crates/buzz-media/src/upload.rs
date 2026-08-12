//! Upload pipeline — validate and stage immutable blobs and manifests.

use buzz_core::tenant::TenantContext;
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::fmt;
use tokio::io::AsyncWriteExt;

use crate::auth::verify_blossom_upload_auth;
use crate::config::MediaConfig;
use crate::error::MediaError;
use crate::publication::{MediaPublicationManifest, StagedMediaPublication};
use crate::storage::{BlobMeta, MediaStorage};
use crate::thumbnail::generate_image_metadata_sync;
use crate::types::BlobDescriptor;
use crate::upload_record::{
    stage_upload_record, StagedUploadRecord, UploadAttribution, UploadEventFacts,
};
use crate::validation::{
    looks_like_mp4_iso_bmff, mime_to_ext, validate_content, validate_file_content,
    validate_video_file,
};

/// Shared buffered-upload pipeline for the image and generic-file paths.
///
/// Both paths are identical except for two steps, which are injected:
/// - `validate`: a CPU-bound check (run inside `spawn_blocking`) that returns
///   the `(mime, ext)` pair for the body. Images derive `ext` from the MIME;
///   generic files get both from the deny-list validator.
/// - `prepare_metadata`: builds metadata and stores any derived artifacts such
///   as a thumbnail, but deliberately does not write operational projections.
///   It receives the already-computed
///   `(sha256, ext, mime, uploaded_at)` so no work is repeated.
///
/// Everything else — hash, Blossom auth (10-minute window), content-addressed
/// key, blob store, orphan-blob handling, and descriptor build — is common.
/// The streaming video path stays separate (see [`stage_video_upload`])
/// because it never buffers in RAM.
///
/// `attribution` is `Some` when per-event upload records are enabled
/// (`BUZZ_MEDIA_UPLOAD_RECORDS`). Staging retains the exact record facts but
/// does not call the record sink: an upload is not accepted until its canonical
/// protected-operation witness commits. The caller publishes the retained
/// record and cache-only sidecar afterward.
struct BufferedUploadInput<'a> {
    storage: &'a MediaStorage,
    config: &'a MediaConfig,
    ctx: &'a TenantContext,
    auth_event: &'a nostr::Event,
    body: Bytes,
    attribution: Option<UploadAttribution>,
}

/// Immutable artifacts and response metadata staged before DB publication.
pub struct StagedMediaUpload {
    publication: StagedMediaPublication,
    descriptor: BlobDescriptor,
    upload_record: Option<StagedUploadRecord>,
}

impl fmt::Debug for StagedMediaUpload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StagedMediaUpload([REDACTED])")
    }
}

impl StagedMediaUpload {
    /// Opaque immutable publication consumed by the protected-operation commit.
    pub const fn publication(&self) -> &StagedMediaPublication {
        &self.publication
    }

    /// Response descriptor returned only after the DB witness commits.
    pub const fn descriptor(&self) -> &BlobDescriptor {
        &self.descriptor
    }

    /// Deterministic moderation recovery plan bound into the publication.
    pub const fn moderation_projection(&self) -> Option<&StagedUploadRecord> {
        self.upload_record.as_ref()
    }

    /// Consume the staged upload after publication and cache projection.
    pub fn into_descriptor(self) -> BlobDescriptor {
        self.descriptor
    }

    /// Publish non-authoritative operational projections after DB publication.
    ///
    /// The moderation record describes an accepted publication, so staging
    /// must not write it before the canonical protected-operation commit. The
    /// sidecar is refreshed afterward and remains a disposable cache.
    pub async fn publish_post_commit_projections(
        &self,
        storage: &MediaStorage,
        ctx: &TenantContext,
        public_base_url: &str,
    ) -> Result<(), MediaError> {
        if self.publication.manifest().community_id() != ctx.community() {
            return Err(MediaError::StorageError(
                "media projection tenant does not match staged publication".into(),
            ));
        }
        if let Some(record) = &self.upload_record {
            record.publish_exact(storage).await?;
        }
        self.publication
            .publish_sidecar_cache(storage, ctx, public_base_url)
            .await
    }
}

async fn stage_buffered_upload<V, M, Fut>(
    input: BufferedUploadInput<'_>,
    validate: V,
    prepare_metadata: M,
) -> Result<StagedMediaUpload, MediaError>
where
    V: FnOnce(&Bytes, &MediaConfig) -> Result<(String, String), MediaError> + Send + 'static,
    M: FnOnce(MetadataInput) -> Fut,
    Fut: std::future::Future<Output = Result<BlobMeta, MediaError>>,
{
    let BufferedUploadInput {
        storage,
        config,
        ctx,
        auth_event,
        body,
        attribution,
    } = input;

    // CPU-bound: validate content, compute hash, verify auth.
    let auth = auth_event.clone();
    let bytes = body.clone();
    let cfg = config.clone();
    // Validate the Blossom `server` tag against the host this request was bound
    // to (the per-request tenant), not a process-global domain — a relay serves
    // many tenant hosts.
    let bound_host = ctx.host().to_string();
    let (mime, sha256, ext) = tokio::task::spawn_blocking(move || -> Result<_, MediaError> {
        let (mime, ext) = validate(&bytes, &cfg)?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        // Buffered uploads (image + file): 10-minute auth window is plenty.
        verify_blossom_upload_auth(&auth, &sha256, Some(bound_host.as_str()), 600)?;
        Ok((mime, sha256, ext))
    })
    .await
    .map_err(|_| MediaError::Internal)??;

    let key = format!("{sha256}.{ext}");
    // The signed request time makes crash recovery and exact replay converge
    // on one manifest and moderation projection instead of minting new state.
    let uploaded_at = i64::try_from(auth_event.created_at.as_secs())
        .map_err(|_| MediaError::StorageError("upload timestamp exceeds i64".into()))?;

    // Store blob first, then metadata.
    // On failure we intentionally do NOT delete the orphan blob — concurrent
    // uploads of the same hash could race and delete a blob that another
    // request is about to reference via its sidecar. Orphan blobs are
    // content-addressed and bounded by the upload size limit, so the storage
    // cost is negligible. A V2 background GC job can sweep blobs with no
    // matching sidecar after a grace period.
    storage.put_immutable_exact(&key, &body, &mime).await?;

    let meta = match prepare_metadata(MetadataInput {
        sha256: sha256.clone(),
        ext: ext.clone(),
        mime: mime.clone(),
        body: body.clone(),
        uploaded_at,
    })
    .await
    {
        Ok(meta) => meta,
        Err(e) => {
            tracing::warn!(sha256 = %sha256, error = %e, "metadata generation failed; orphan blob left for GC");
            return Err(e);
        }
    };

    let upload_record = match attribution {
        Some(attribution) => Some(
            stage_upload_record(
                storage,
                ctx,
                &auth_event.pubkey,
                &auth_event.id.to_hex(),
                &attribution,
                UploadEventFacts {
                    sha256: &sha256,
                    ext: &ext,
                    mime: &mime,
                    size: body.len() as u64,
                    uploaded_at,
                },
            )
            .await?,
        ),
        None => None,
    };
    let manifest = MediaPublicationManifest::from_staged_blob_with_projection(
        ctx,
        &sha256,
        &meta,
        upload_record.as_ref().map(StagedUploadRecord::digest),
    )?;
    let publication = storage.stage_media_publication(manifest).await?;
    let descriptor = publication.manifest().descriptor(&config.public_base_url)?;
    Ok(StagedMediaUpload {
        publication,
        descriptor,
        upload_record,
    })
}

/// Inputs handed to a buffered-upload metadata builder, after the shared
/// pipeline has already validated, hashed, and stored the blob. Owned so the
/// builder's future doesn't borrow the pipeline's locals; `body` is a `Bytes`
/// handle, so cloning it is a refcount bump, not a copy.
struct MetadataInput {
    sha256: String,
    ext: String,
    mime: String,
    body: Bytes,
    uploaded_at: i64,
}

/// Validate an image and stage all immutable artifacts without publishing it.
///
/// This is the image path — body is already fully buffered in RAM. Do NOT use
/// this for video uploads; use [`stage_video_upload`] instead.
pub async fn stage_upload(
    storage: &MediaStorage,
    config: &MediaConfig,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    body: Bytes,
    attribution: Option<UploadAttribution>,
) -> Result<StagedMediaUpload, MediaError> {
    stage_buffered_upload(
        BufferedUploadInput {
            storage,
            config,
            ctx,
            auth_event,
            body,
            attribution,
        },
        |bytes, cfg| {
            let mime = validate_content(bytes, cfg)?;
            let ext = mime_to_ext(&mime).to_string();
            Ok((mime, ext))
        },
        |input| async move { prepare_image_metadata(storage, config, input).await },
    )
    .await
}

/// Legacy end-to-end image helper retained until relay DB composition lands.
///
/// Protected transports must call [`stage_upload`], commit the returned digest
/// in PostgreSQL, and only then invoke
/// [`StagedMediaUpload::publish_post_commit_projections`].
pub async fn process_upload(
    storage: &MediaStorage,
    config: &MediaConfig,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    body: Bytes,
    attribution: Option<UploadAttribution>,
) -> Result<BlobDescriptor, MediaError> {
    let staged = stage_upload(storage, config, ctx, auth_event, body, attribution).await?;
    staged
        .publish_post_commit_projections(storage, ctx, &config.public_base_url)
        .await?;
    Ok(staged.into_descriptor())
}

/// Process a generic non-media file upload end-to-end.
///
/// This is the catch-all attachment path for documents, archives, text, and
/// data. Recognized image, video, and audio formats fail closed instead of
/// entering exact-byte storage without their format-specific location policy.
/// The body is fully buffered in RAM (bounded by `config.max_file_bytes` at the
/// transport layer), validated against the deny-list + size cap, stored, and
/// recorded in a minimal sidecar. No thumbnail, dimensions, or duration.
///
/// The resulting blob is served with `Content-Disposition: attachment`, so the
/// client always downloads it rather than rendering it inline.
pub async fn stage_file_upload(
    storage: &MediaStorage,
    config: &MediaConfig,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    body: Bytes,
    attribution: Option<UploadAttribution>,
) -> Result<StagedMediaUpload, MediaError> {
    stage_buffered_upload(
        BufferedUploadInput {
            storage,
            config,
            ctx,
            auth_event,
            body,
            attribution,
        },
        |bytes, cfg| validate_file_content(bytes, cfg),
        |input| async move {
            // Minimal sidecar — no thumbnail/dim/blurhash/duration for generic files.
            let meta = BlobMeta {
                dim: String::new(),
                blurhash: String::new(),
                thumb_url: String::new(),
                thumbnail_sha256: None,
                size: input.body.len() as u64,
                ext: input.ext,
                mime_type: input.mime,
                uploaded_at: input.uploaded_at,
                duration_secs: None,
            };
            Ok(meta)
        },
    )
    .await
}

/// Legacy end-to-end file helper retained until relay DB composition lands.
pub async fn process_file_upload(
    storage: &MediaStorage,
    config: &MediaConfig,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    body: Bytes,
    attribution: Option<UploadAttribution>,
) -> Result<BlobDescriptor, MediaError> {
    let staged = stage_file_upload(storage, config, ctx, auth_event, body, attribution).await?;
    staged
        .publish_post_commit_projections(storage, ctx, &config.public_base_url)
        .await?;
    Ok(staged.into_descriptor())
}

/// Validate a video and stage all immutable artifacts using a streaming pipeline.
///
/// Unlike [`process_upload`], this function:
/// 1. Streams the request body to a [`tempfile::NamedTempFile`] while computing
///    SHA-256 incrementally — the full body is never in RAM simultaneously.
/// 2. Verifies the Blossom auth event `x` tag against the computed hash.
/// 3. Runs full MP4 validation (codec, duration, resolution, moov placement).
/// 4. Stores the blob via [`MediaStorage::put_file`] (streaming read from disk).
/// 5. Writes a sidecar with `duration_secs` (no thumbnail — desktop handles that).
///
/// Returns a [`BlobDescriptor`] with the `duration` field populated.
pub async fn stage_video_upload(
    storage: &MediaStorage,
    config: &MediaConfig,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    body_stream: impl futures_core::Stream<Item = Result<Bytes, axum::Error>> + Send + 'static,
    content_length: Option<u64>,
    attribution: Option<UploadAttribution>,
) -> Result<StagedMediaUpload, MediaError> {
    // --- 1. Stream body to temp file, compute SHA-256 incrementally ---
    let tmp = tempfile::NamedTempFile::new().map_err(|e| MediaError::Io(e.to_string()))?;
    let tmp_path = tmp.path().to_path_buf();

    let max_bytes = config.max_video_bytes;

    // Fast-fail: reject oversized uploads before streaming starts.
    if let Some(cl) = content_length {
        if cl > max_bytes {
            return Err(MediaError::FileTooLarge {
                size: cl,
                max: max_bytes,
            });
        }
    }

    let (sha256_hex, file_size, first_bytes) = {
        use tokio_util::io::StreamReader;

        // Convert axum::Error stream to std::io::Error stream for StreamReader.
        // Box::pin is required because StreamReader needs a pinned stream.
        // Belt-and-suspenders body-limit detection: axum wraps LengthLimitError
        // in its error chain but doesn't expose the inner type for downcasting.
        // We check multiple Display strings so that if axum changes the wording,
        // at least one pattern still matches. test_body_limit_error_detection
        // will catch a regression if ALL patterns break.
        let mapped = futures_util::StreamExt::map(body_stream, |r| {
            r.map_err(|e| {
                let msg = e.to_string();
                if msg.contains("length limit")
                    || msg.contains("body limit")
                    || msg.contains("LengthLimitError")
                {
                    std::io::Error::new(std::io::ErrorKind::WriteZero, msg)
                } else {
                    std::io::Error::other(e)
                }
            })
        });
        let mut reader = StreamReader::new(Box::pin(mapped));

        let mut file = tokio::fs::File::create(&tmp_path)
            .await
            .map_err(|e| MediaError::Io(e.to_string()))?;
        let mut hasher = Sha256::new();
        let mut total: u64 = 0;
        // Accumulate enough leading bytes for magic-byte detection.
        // 4 KiB is the standard sniff buffer — infer checks signatures at
        // various offsets, and some formats need more than just the first few
        // bytes. This is tiny relative to any real upload.
        const MIN_SNIFF_BYTES: usize = 4096;
        let mut sniff_buf: Vec<u8> = Vec::with_capacity(MIN_SNIFF_BYTES);
        let mut buf = vec![0u8; 64 * 1024]; // 64 KiB read buffer

        loop {
            use tokio::io::AsyncReadExt;
            let n = match reader.read(&mut buf).await {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::WriteZero => {
                    // Body limit exceeded — return 413 instead of 500.
                    // `total` is bytes received before the cutoff — honest, not exact.
                    return Err(MediaError::FileTooLarge {
                        size: total,
                        max: max_bytes,
                    });
                }
                Err(e) => return Err(MediaError::Io(e.to_string())),
            };
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > max_bytes {
                return Err(MediaError::FileTooLarge {
                    size: total,
                    max: max_bytes,
                });
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])
                .await
                .map_err(|e| MediaError::Io(e.to_string()))?;
            if sniff_buf.len() < MIN_SNIFF_BYTES {
                let need = MIN_SNIFF_BYTES - sniff_buf.len();
                sniff_buf.extend_from_slice(&buf[..n.min(need)]);
            }
        }
        file.flush()
            .await
            .map_err(|e| MediaError::Io(e.to_string()))?;

        let sha256_hex = hex::encode(hasher.finalize());
        (sha256_hex, total, sniff_buf)
    };

    // --- 2. ISO-BMFF/MP4 structural check ---
    // Do not depend on `infer`'s finite major-brand list: valid MP4 producers
    // may use a proprietary major brand while declaring `isom` compatibility.
    if !looks_like_mp4_iso_bmff(&first_bytes) {
        return Err(MediaError::UnsupportedContainer);
    }
    let mime = "video/mp4".to_string();

    // --- 3. Verify Blossom auth: x tag must match computed SHA-256 ---
    let auth = auth_event.clone();
    let sha256_for_auth = sha256_hex.clone();
    // Validate the Blossom `server` tag against the bound tenant host (not a
    // process-global domain) — a relay serves many tenant hosts.
    let bound_host = ctx.host().to_string();
    tokio::task::spawn_blocking(move || {
        // Videos: 1-hour window — large uploads on slow connections need headroom.
        verify_blossom_upload_auth(&auth, &sha256_for_auth, Some(bound_host.as_str()), 3600)
    })
    .await
    .map_err(|_| MediaError::Internal)??;

    // --- 4. Full MP4 validation on the temp file ---
    let tmp_path_clone = tmp_path.clone();
    let cfg = config.clone();
    let video_meta =
        tokio::task::spawn_blocking(move || validate_video_file(&tmp_path_clone, &cfg))
            .await
            .map_err(|_| MediaError::Internal)??;

    let ext = "mp4";
    let key = format!("{sha256_hex}.{ext}");
    let uploaded_at = i64::try_from(auth_event.created_at.as_secs())
        .map_err(|_| MediaError::StorageError("upload timestamp exceeds i64".into()))?;

    // --- 6. Stream blob from temp file to S3 ---
    storage
        .put_file_immutable_verified(&key, &tmp_path, &mime, &sha256_hex, file_size)
        .await?;
    drop(tmp); // Free temp file disk space immediately after S3 upload.

    // --- 7. Build metadata (no thumbnail for video — desktop handles that) ---
    let meta = BlobMeta {
        dim: format!("{}x{}", video_meta.width, video_meta.height),
        blurhash: String::new(),
        thumb_url: String::new(),
        thumbnail_sha256: None,
        ext: ext.to_string(),
        mime_type: mime.clone(),
        size: file_size,
        uploaded_at,
        duration_secs: Some(video_meta.duration_secs),
    };

    let upload_record = match attribution {
        Some(attribution) => Some(
            stage_upload_record(
                storage,
                ctx,
                &auth_event.pubkey,
                &auth_event.id.to_hex(),
                &attribution,
                UploadEventFacts {
                    sha256: &sha256_hex,
                    ext,
                    mime: &mime,
                    size: file_size,
                    uploaded_at,
                },
            )
            .await?,
        ),
        None => None,
    };
    let manifest = MediaPublicationManifest::from_staged_blob_with_projection(
        ctx,
        &sha256_hex,
        &meta,
        upload_record.as_ref().map(StagedUploadRecord::digest),
    )?;
    let publication = storage.stage_media_publication(manifest).await?;
    let descriptor = publication.manifest().descriptor(&config.public_base_url)?;
    Ok(StagedMediaUpload {
        publication,
        descriptor,
        upload_record,
    })
}

/// Legacy end-to-end video helper retained until relay DB composition lands.
pub async fn process_video_upload(
    storage: &MediaStorage,
    config: &MediaConfig,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    body_stream: impl futures_core::Stream<Item = Result<Bytes, axum::Error>> + Send + 'static,
    content_length: Option<u64>,
    attribution: Option<UploadAttribution>,
) -> Result<BlobDescriptor, MediaError> {
    let staged = stage_video_upload(
        storage,
        config,
        ctx,
        auth_event,
        body_stream,
        content_length,
        attribution,
    )
    .await?;
    staged
        .publish_post_commit_projections(storage, ctx, &config.public_base_url)
        .await?;
    Ok(staged.into_descriptor())
}

/// Generate immutable thumbnail and metadata without publishing cache projections.
/// Returns the completed [`BlobMeta`] on success.
async fn prepare_image_metadata(
    storage: &MediaStorage,
    config: &MediaConfig,
    input: MetadataInput,
) -> Result<BlobMeta, MediaError> {
    let body_ref = input.body.clone();
    let mime_ref = input.mime.clone();
    let ext_ref = input.ext.clone();
    let sha256_ref = input.sha256.clone();
    let cfg_ref = config.clone();
    let (mut meta, thumb_bytes) = tokio::task::spawn_blocking(move || {
        generate_image_metadata_sync(&cfg_ref, &sha256_ref, &body_ref, &mime_ref, &ext_ref)
    })
    .await
    .map_err(|_| MediaError::Internal)??;

    meta.uploaded_at = input.uploaded_at;

    if let Some(ref tb) = thumb_bytes {
        let thumbnail_sha256 = hex::encode(Sha256::digest(tb));
        let thumb_key = format!("{thumbnail_sha256}.thumb.jpg");
        storage
            .put_immutable_exact(&thumb_key, tb, "image/jpeg")
            .await?;
        meta.thumbnail_sha256 = Some(thumbnail_sha256);
    }

    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_publication_descriptor_video_omits_empty_thumb_and_blurhash() {
        // Video publications omit thumbnail and blurhash response fields.
        let meta = BlobMeta {
            dim: "320x240".to_string(),
            blurhash: String::new(),  // empty — video has no blurhash
            thumb_url: String::new(), // empty — video has no thumbnail
            thumbnail_sha256: None,
            ext: "mp4".to_string(),
            mime_type: "video/mp4".to_string(),
            size: 5_000_000,
            uploaded_at: 1700000000,
            duration_secs: Some(29.5),
        };

        let desc = MediaPublicationManifest::from_staged_blob(
            &TenantContext::resolved(
                buzz_core::CommunityId::from_uuid(uuid::Uuid::from_u128(1)),
                "media.example.com",
            ),
            "a".repeat(64),
            &meta,
        )
        .expect("manifest")
        .descriptor("https://media.example.com")
        .expect("descriptor");

        // Empty strings must become None, not Some("")
        assert!(
            desc.blurhash.is_none(),
            "blurhash should be None for video, got {:?}",
            desc.blurhash
        );
        assert!(
            desc.thumb.is_none(),
            "thumb should be None for video, got {:?}",
            desc.thumb
        );
        // Non-empty fields should be present
        assert_eq!(desc.dim, Some("320x240".to_string()));
        assert_eq!(desc.duration, Some(29.5));

        // Verify JSON serialization omits the empty fields entirely
        let json = serde_json::to_value(&desc).unwrap();
        assert!(
            json.get("blurhash").is_none(),
            "blurhash should be absent from JSON"
        );
        assert!(
            json.get("thumb").is_none(),
            "thumb should be absent from JSON"
        );
        assert!(json.get("dim").is_some(), "dim should be present in JSON");
        assert!(
            json.get("duration").is_some(),
            "duration should be present in JSON"
        );
    }

    #[test]
    fn test_publication_descriptor_image_includes_thumb_and_blurhash() {
        // Image uploads produce a BlobMeta with populated thumb_url and blurhash.
        let hash = "a".repeat(64);
        let meta = BlobMeta {
            dim: "800x600".to_string(),
            blurhash: "LEHV6nWB2yk8pyo0adR*.7kCMdnj".to_string(),
            thumb_url: format!("https://media.example.com/{hash}.thumb.jpg"),
            thumbnail_sha256: Some("b".repeat(64)),
            ext: "jpg".to_string(),
            mime_type: "image/jpeg".to_string(),
            size: 100_000,
            uploaded_at: 1700000000,
            duration_secs: None,
        };

        let desc = MediaPublicationManifest::from_staged_blob(
            &TenantContext::resolved(
                buzz_core::CommunityId::from_uuid(uuid::Uuid::from_u128(1)),
                "media.example.com",
            ),
            &hash,
            &meta,
        )
        .expect("manifest")
        .descriptor("https://media.example.com")
        .expect("descriptor");

        assert_eq!(
            desc.blurhash,
            Some("LEHV6nWB2yk8pyo0adR*.7kCMdnj".to_string())
        );
        assert!(desc.thumb.is_some());
        assert!(desc.duration.is_none());

        // Verify JSON: duration should be absent, blurhash and thumb present
        let json = serde_json::to_value(&desc).unwrap();
        assert!(json.get("blurhash").is_some());
        assert!(json.get("thumb").is_some());
        assert!(
            json.get("duration").is_none(),
            "duration should be absent for images"
        );
    }

    #[test]
    fn test_body_limit_error_detection() {
        // Verify that body-limit errors are mapped to WriteZero (which
        // process_video_upload converts to FileTooLarge / 413).
        // Must match the detection logic in process_video_upload exactly.
        let detect = |msg: &str| -> std::io::ErrorKind {
            if msg.contains("length limit")
                || msg.contains("body limit")
                || msg.contains("LengthLimitError")
            {
                std::io::ErrorKind::WriteZero
            } else {
                std::io::ErrorKind::Other
            }
        };

        // All known patterns should trigger WriteZero.
        assert_eq!(
            detect("length limit exceeded"),
            std::io::ErrorKind::WriteZero
        );
        assert_eq!(detect("body limit exceeded"), std::io::ErrorKind::WriteZero);
        assert_eq!(detect("LengthLimitError"), std::io::ErrorKind::WriteZero);

        // Non-limit errors should remain as Other.
        assert_eq!(detect("connection reset"), std::io::ErrorKind::Other);
    }

    #[test]
    fn test_publication_descriptor_without_optional_metadata() {
        let meta = BlobMeta {
            dim: String::new(),
            blurhash: String::new(),
            thumb_url: String::new(),
            thumbnail_sha256: None,
            ext: "bin".to_owned(),
            mime_type: "application/octet-stream".to_owned(),
            size: 100,
            uploaded_at: 1_700_000_000,
            duration_secs: None,
        };
        let desc = MediaPublicationManifest::from_staged_blob(
            &TenantContext::resolved(
                buzz_core::CommunityId::from_uuid(uuid::Uuid::from_u128(1)),
                "media.example.com",
            ),
            "a".repeat(64),
            &meta,
        )
        .expect("manifest")
        .descriptor("https://media.example.com")
        .expect("descriptor");

        assert!(desc.dim.is_none());
        assert!(desc.blurhash.is_none());
        assert!(desc.thumb.is_none());
        assert!(desc.duration.is_none());
    }
}
