//! Upload pipeline — validate, store, thumbnail, sidecar.

use buzz_core::tenant::TenantContext;
use bytes::Bytes;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::auth::verify_blossom_upload_auth;
use crate::config::MediaConfig;
use crate::error::MediaError;
use crate::storage::{BlobMeta, MediaStorage};
use crate::thumbnail::generate_image_metadata_sync;
use crate::types::BlobDescriptor;
use crate::upload_record::{record_upload_event, UploadAttribution, UploadEventFacts};
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
///   as a thumbnail, but deliberately does not write the sidecar. The sidecar
///   is the media serve gate and is published only after the moderation record
///   succeeds. It receives the already-computed
///   `(sha256, ext, mime, uploaded_at)` so no work is repeated.
///
/// Everything else — hash, Blossom auth (10-minute window), content-addressed
/// key, the both-exist idempotency short-circuit, blob store, orphan-blob
/// handling, and descriptor build — is common. The streaming video path stays
/// separate (see [`process_video_upload`]) because it never buffers in RAM.
///
/// `attribution` is `Some` when per-event upload records are enabled
/// (`BUZZ_MEDIA_UPLOAD_RECORDS`): a record is then written for **every**
/// accepted upload — including the idempotent short-circuit, which does no
/// blob PUT and would otherwise be invisible to the moderation pipeline.
/// For fresh uploads, the record is written after the blob and derived
/// artifacts but before the sidecar. This preserves both contracts: record
/// existence implies referenced objects are readable, while a record failure
/// cannot publish media without triggering moderation.
struct BufferedUploadInput<'a> {
    storage: &'a MediaStorage,
    config: &'a MediaConfig,
    ctx: &'a TenantContext,
    auth_event: &'a nostr::Event,
    body: Bytes,
    attribution: Option<UploadAttribution>,
}

async fn process_buffered_upload<V, M, Fut>(
    input: BufferedUploadInput<'_>,
    validate: V,
    prepare_metadata: M,
) -> Result<BlobDescriptor, MediaError>
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
    let meta_key = MediaStorage::ctx_sidecar_key(ctx, &sha256);

    // Idempotent: short-circuit only if BOTH sidecar and blob exist. If the
    // sidecar exists but the blob is missing, fall through to re-upload.
    let sidecar_exists = storage.head(&meta_key).await?;
    let blob_exists = storage.head(&key).await?;
    if sidecar_exists && blob_exists {
        let meta = storage.get_sidecar(ctx, &sha256).await?;
        // A re-upload of known bytes is still a distinct upload *event*: no
        // blob PUT happens, so without this record the uploader would be
        // invisible to the moderation pipeline (and takedown re-uploads
        // would go unscanned).
        if let Some(attribution) = &attribution {
            record_upload_event(
                storage,
                ctx,
                &auth_event.pubkey,
                attribution,
                UploadEventFacts {
                    sha256: &sha256,
                    ext: &ext,
                    mime: &mime,
                    size: body.len() as u64,
                    uploaded_at: chrono::Utc::now().timestamp(),
                },
            )
            .await?;
        }
        return Ok(build_descriptor(
            config,
            &sha256,
            &ext,
            &mime,
            body.len() as u64,
            Some(&meta),
            meta.uploaded_at,
        ));
    }

    // Compute uploaded_at once — single source of truth for sidecar and response.
    let uploaded_at = chrono::Utc::now().timestamp();

    // Store blob first, then metadata.
    // On failure we intentionally do NOT delete the orphan blob — concurrent
    // uploads of the same hash could race and delete a blob that another
    // request is about to reference via its sidecar. Orphan blobs are
    // content-addressed and bounded by the upload size limit, so the storage
    // cost is negligible. A V2 background GC job can sweep blobs with no
    // matching sidecar after a grace period.
    storage.put(&key, &body, &mime).await?;

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

    // The moderation record precedes the sidecar publish gate. If this write
    // fails, the blob and any thumbnail remain orphaned but the media cannot be
    // served. Conversely, record existence still implies those objects exist.
    if let Some(attribution) = &attribution {
        record_upload_event(
            storage,
            ctx,
            &auth_event.pubkey,
            attribution,
            UploadEventFacts {
                sha256: &sha256,
                ext: &ext,
                mime: &mime,
                size: body.len() as u64,
                uploaded_at,
            },
        )
        .await?;
    }
    storage.put_sidecar(ctx, &sha256, &meta).await?;

    Ok(build_descriptor(
        config,
        &sha256,
        &ext,
        &mime,
        body.len() as u64,
        Some(&meta),
        uploaded_at,
    ))
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

/// Process an upload end-to-end: validate, store, thumbnail, return descriptor.
///
/// This is the image path — body is already fully buffered in RAM. Do NOT use
/// this for video uploads; use [`process_video_upload`] instead.
pub async fn process_upload(
    storage: &MediaStorage,
    config: &MediaConfig,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    body: Bytes,
    attribution: Option<UploadAttribution>,
) -> Result<BlobDescriptor, MediaError> {
    process_buffered_upload(
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
pub async fn process_file_upload(
    storage: &MediaStorage,
    config: &MediaConfig,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    body: Bytes,
    attribution: Option<UploadAttribution>,
) -> Result<BlobDescriptor, MediaError> {
    process_buffered_upload(
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

/// Process a video upload end-to-end using a streaming pipeline.
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
pub async fn process_video_upload(
    storage: &MediaStorage,
    config: &MediaConfig,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    body_stream: impl futures_core::Stream<Item = Result<Bytes, axum::Error>> + Send + 'static,
    content_length: Option<u64>,
    attribution: Option<UploadAttribution>,
) -> Result<BlobDescriptor, MediaError> {
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

        // Convert axum::Error stream to std::io::Error stream for StreamReader,
        // preserving the error class in the io::ErrorKind so the read loop
        // below can answer with the right status:
        //   - idle-deadline trip (typed tower-http TimeoutError in the source
        //     chain) -> TimedOut -> RequestBodyTimeout / 408
        //   - body-limit breach -> WriteZero -> FileTooLarge / 413
        //   - anything else -> Other -> Io / 500
        // Limit detection is belt-and-suspenders by Display string because
        // axum wraps LengthLimitError without exposing the inner type for
        // downcasting (see classify_body_error + test_body_limit_error_detection).
        let mapped = futures_util::StreamExt::map(body_stream, |r| {
            r.map_err(|e| match crate::error::classify_body_error(&e) {
                crate::error::BodyErrorKind::IdleTimeout => {
                    std::io::Error::new(std::io::ErrorKind::TimedOut, e)
                }
                crate::error::BodyErrorKind::LengthLimit => {
                    std::io::Error::new(std::io::ErrorKind::WriteZero, e)
                }
                crate::error::BodyErrorKind::Other => std::io::Error::other(e),
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
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    // Idle deadline fired — the client stopped sending bytes.
                    // 408, never 500: this is not a storage failure.
                    return Err(MediaError::RequestBodyTimeout);
                }
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
    let meta_key = MediaStorage::ctx_sidecar_key(ctx, &sha256_hex);

    // --- 5. Idempotency check ---
    let sidecar_exists = storage.head(&meta_key).await?;
    let blob_exists = storage.head(&key).await?;
    if sidecar_exists && blob_exists {
        let meta = storage.get_sidecar(ctx, &sha256_hex).await?;
        // Re-upload of known bytes: still a distinct upload event — see the
        // buffered path's short-circuit for the rationale.
        if let Some(attribution) = &attribution {
            record_upload_event(
                storage,
                ctx,
                &auth_event.pubkey,
                attribution,
                UploadEventFacts {
                    sha256: &sha256_hex,
                    ext,
                    mime: &mime,
                    size: file_size,
                    uploaded_at: chrono::Utc::now().timestamp(),
                },
            )
            .await?;
        }
        return Ok(build_descriptor(
            config,
            &sha256_hex,
            ext,
            &mime,
            file_size,
            Some(&meta),
            meta.uploaded_at,
        ));
    }

    let uploaded_at = chrono::Utc::now().timestamp();

    // --- 6. Stream blob from temp file to S3 ---
    storage.put_file(&key, &tmp_path, &mime).await?;
    drop(tmp); // Free temp file disk space immediately after S3 upload.

    // --- 7. Build metadata (no thumbnail for video — desktop handles that) ---
    let meta = BlobMeta {
        dim: format!("{}x{}", video_meta.width, video_meta.height),
        blurhash: String::new(),
        thumb_url: String::new(),
        ext: ext.to_string(),
        mime_type: mime.clone(),
        size: file_size,
        uploaded_at,
        duration_secs: Some(video_meta.duration_secs),
    };

    // Record before publishing the sidecar serve gate. See the buffered path.
    if let Some(attribution) = &attribution {
        record_upload_event(
            storage,
            ctx,
            &auth_event.pubkey,
            attribution,
            UploadEventFacts {
                sha256: &sha256_hex,
                ext,
                mime: &mime,
                size: file_size,
                uploaded_at,
            },
        )
        .await?;
    }
    storage.put_sidecar(ctx, &sha256_hex, &meta).await?;

    Ok(build_descriptor(
        config,
        &sha256_hex,
        ext,
        &mime,
        file_size,
        Some(&meta),
        uploaded_at,
    ))
}

/// Generate thumbnail and metadata without publishing the sidecar serve gate.
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
        let thumb_key = format!("{}.thumb.jpg", input.sha256);
        storage.put(&thumb_key, tb, "image/jpeg").await?;
    }

    Ok(meta)
}

fn build_descriptor(
    config: &MediaConfig,
    sha256: &str,
    ext: &str,
    mime: &str,
    size: u64,
    meta: Option<&BlobMeta>,
    uploaded_at: i64,
) -> BlobDescriptor {
    let duration = meta.and_then(|m| m.duration_secs);
    BlobDescriptor {
        url: format!("{}/{sha256}.{ext}", config.public_base_url),
        sha256: sha256.to_string(),
        size,
        mime_type: mime.to_string(),
        uploaded: uploaded_at,
        dim: meta.and_then(|m| (!m.dim.is_empty()).then(|| m.dim.clone())),
        blurhash: meta.and_then(|m| (!m.blurhash.is_empty()).then(|| m.blurhash.clone())),
        thumb: meta.and_then(|m| (!m.thumb_url.is_empty()).then(|| m.thumb_url.clone())),
        duration,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> MediaConfig {
        MediaConfig {
            s3_endpoint: String::new(),
            s3_access_key: String::new(),
            s3_secret_key: String::new(),
            s3_bucket: String::new(),
            s3_region: "us-east-1".to_string(),
            s3_addressing_style: crate::config::S3AddressingStyle::Path,
            max_image_bytes: 50 * 1024 * 1024,
            max_gif_bytes: 10 * 1024 * 1024,
            max_video_bytes: 524_288_000,
            max_file_bytes: 104_857_600,
            public_base_url: "https://media.example.com".to_string(),
            upload_records_enabled: false,
            upload_ip_header: None,
            upload_port_header: None,
        }
    }

    #[test]
    fn test_build_descriptor_video_omits_empty_thumb_and_blurhash() {
        // Video uploads produce a BlobMeta with empty thumb_url and blurhash.
        // build_descriptor must convert these to None so they're omitted from JSON.
        let config = test_config();
        let meta = BlobMeta {
            dim: "320x240".to_string(),
            blurhash: String::new(),  // empty — video has no blurhash
            thumb_url: String::new(), // empty — video has no thumbnail
            ext: "mp4".to_string(),
            mime_type: "video/mp4".to_string(),
            size: 5_000_000,
            uploaded_at: 1700000000,
            duration_secs: Some(29.5),
        };

        let desc = build_descriptor(
            &config,
            "abc123",
            "mp4",
            "video/mp4",
            5_000_000,
            Some(&meta),
            1700000000,
        );

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
    fn test_build_descriptor_image_includes_thumb_and_blurhash() {
        // Image uploads produce a BlobMeta with populated thumb_url and blurhash.
        let config = test_config();
        let hash = "a".repeat(64);
        let meta = BlobMeta {
            dim: "800x600".to_string(),
            blurhash: "LEHV6nWB2yk8pyo0adR*.7kCMdnj".to_string(),
            thumb_url: format!("https://media.example.com/{hash}.thumb.jpg"),
            ext: "jpg".to_string(),
            mime_type: "image/jpeg".to_string(),
            size: 100_000,
            uploaded_at: 1700000000,
            duration_secs: None,
        };

        let desc = build_descriptor(
            &config,
            &hash,
            "jpg",
            "image/jpeg",
            100_000,
            Some(&meta),
            1700000000,
        );

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
        // Body-limit errors must classify as LengthLimit (which the upload
        // paths convert to FileTooLarge / 413). Belt-and-suspenders by
        // Display string because axum wraps LengthLimitError without
        // exposing the type; if ALL patterns break this test catches it.
        use crate::error::{classify_body_error, BodyErrorKind};

        let error = |msg: &str| std::io::Error::other(msg.to_string());

        // All known patterns should classify as LengthLimit.
        assert_eq!(
            classify_body_error(&error("length limit exceeded")),
            BodyErrorKind::LengthLimit
        );
        assert_eq!(
            classify_body_error(&error("body limit exceeded")),
            BodyErrorKind::LengthLimit
        );
        assert_eq!(
            classify_body_error(&error("LengthLimitError")),
            BodyErrorKind::LengthLimit
        );

        // Non-limit errors should remain Other.
        assert_eq!(
            classify_body_error(&error("connection reset")),
            BodyErrorKind::Other
        );
    }

    #[tokio::test]
    async fn classify_body_error_detects_real_tower_http_timeout_by_type() {
        // Drive a genuine tower_http::timeout::TimeoutBody so the classified
        // error is the real typed TimeoutError — not a hand-rolled stand-in —
        // wrapped the way axum wraps body errors in production.
        use crate::error::{classify_body_error, BodyErrorKind};

        let wrapped = axum::Error::new(real_tower_http_timeout_error().await);
        assert_eq!(
            classify_body_error(&wrapped),
            BodyErrorKind::IdleTimeout,
            "typed TimeoutError must classify as IdleTimeout through the axum::Error wrapping"
        );

        // Control: an unrelated wrapped error must NOT classify as a timeout.
        let other = axum::Error::new(std::io::Error::other("connection reset"));
        assert_eq!(classify_body_error(&other), BodyErrorKind::Other);
    }

    /// Produce a genuine `tower_http::timeout::TimeoutError` by driving a
    /// real `TimeoutBody` over a body that never yields a frame — the exact
    /// boxed error shape the media router's `RequestBodyTimeoutLayer`
    /// produces when a client withholds body bytes past the idle deadline.
    async fn real_tower_http_timeout_error() -> Box<dyn std::error::Error + Send + Sync> {
        use http_body_util::BodyExt;

        struct PendingBody;
        impl http_body::Body for PendingBody {
            type Data = Bytes;
            type Error = std::convert::Infallible;
            fn poll_frame(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
                std::task::Poll::Pending
            }
        }

        // TimeoutBody pins a tokio Sleep and is never Unpin; Box::pin gives
        // an Unpin handle that still implements Body, so BodyExt::frame works.
        let mut body = Box::pin(tower_http::timeout::TimeoutBody::new(
            std::time::Duration::from_millis(5),
            PendingBody,
        ));
        body.frame()
            .await
            .expect("timeout should produce a frame result")
            .expect_err("withheld body must error, not yield data")
    }

    #[tokio::test]
    async fn video_stream_idle_timeout_maps_to_request_body_timeout_not_500() {
        // Pins the timeout *conversion* chain at the video-stream boundary:
        // a real tower_http TimeoutError, wrapped the way axum wraps body
        // errors in production, must survive `axum::Error →
        // classify_body_error → io::ErrorKind::TimedOut → the StreamReader
        // read loop` and surface as RequestBodyTimeout / 408 — never as
        // Io / 500, which would page operators for a storage failure that
        // never happened.
        //
        // Honest scope (measured by Sami's mutation pass, M3): this test
        // proves the conversion, NOT the video routing. The read loop
        // returns on the error before `looks_like_mp4_iso_bmff` ever runs,
        // so the chunk content is causally irrelevant here — the sniff
        // decision (`should_stream_as_video`) lives in the relay's
        // `upload_blob`, upstream of this entry point, and is not covered
        // by this test. The MP4-shaped first chunk only makes the stream
        // realistic: bytes flowed, then the client stalled mid-body.
        use axum::response::IntoResponse;

        let mp4_prefix: &[u8] = b"\x00\x00\x00\x18ftypisom\x00\x00\x00\x00isommp42";
        let timeout_error = real_tower_http_timeout_error().await;
        let body_stream = futures_util::stream::iter(vec![
            Ok(Bytes::from_static(mp4_prefix)),
            Err(axum::Error::new(timeout_error)),
        ]);

        // Storage/auth are constructed but never reached: the body error
        // fires inside step 1 (stream-to-temp-file), before the MP4 check,
        // auth verification, or any storage call. Static dummy creds keep
        // `MediaStorage::new` off the AWS credential chain (which fails on
        // hosts without AWS creds), and the unroutable endpoint guarantees
        // any accidental network use fails loudly instead of hanging.
        let config = MediaConfig {
            s3_endpoint: "http://127.0.0.1:1".to_string(),
            s3_access_key: "test".to_string(),
            s3_secret_key: "test".to_string(),
            s3_bucket: "unused".to_string(),
            ..test_config()
        };
        let storage = MediaStorage::new(&config).expect("static-cred storage client");
        let ctx = buzz_core::tenant::TenantContext::resolved(
            buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::nil()),
            "media.example.com",
        );
        let keys = nostr::Keys::generate();
        let auth_event = nostr::EventBuilder::new(nostr::Kind::from(24242), "Upload buzz-media")
            .sign_with_keys(&keys)
            .expect("signable auth event");

        let result = process_video_upload(
            &storage,
            &config,
            &ctx,
            &auth_event,
            body_stream,
            None,
            None,
        )
        .await;

        let error = result.expect_err("a timed-out video stream must fail the upload");
        assert!(
            matches!(error, MediaError::RequestBodyTimeout),
            "post-sniff video-stream timeout must map to RequestBodyTimeout, got {error:?}"
        );
        assert_eq!(
            error.into_response().status(),
            axum::http::StatusCode::REQUEST_TIMEOUT,
            "and it must answer the client with 408"
        );
    }

    #[test]
    fn test_build_descriptor_no_meta() {
        // When meta is None, all optional fields should be None.
        let config = test_config();
        let desc = build_descriptor(
            &config,
            "abc123",
            "jpg",
            "image/jpeg",
            100,
            None,
            1700000000,
        );

        assert!(desc.dim.is_none());
        assert!(desc.blurhash.is_none());
        assert!(desc.thumb.is_none());
        assert!(desc.duration.is_none());
    }
}
