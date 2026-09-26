//! Upload pipeline — validate, store, thumbnail, sidecar.

use buzz_core::tenant::TenantContext;
use bytes::Bytes;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::auth::verify_upload_hash_only;
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
    let (mime, sha256, ext) = tokio::task::spawn_blocking(move || -> Result<_, MediaError> {
        let (mime, ext) = validate(&bytes, &cfg)?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        // Post-body hash check only: the full auth event verification
        // (signature, kind, freshness, server, cardinality) was already
        // applied at the pre-body gate in the relay handler.  Re-running the
        // full verifier here would fail any upload that takes longer than the
        // minted token's expiration window (60 s in Strict mode).
        verify_upload_hash_only(&auth, &sha256)?;
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
    tokio::task::spawn_blocking(move || {
        // Post-body hash check only: the full auth event verification
        // (signature, kind, freshness, server, cardinality) was already
        // applied at the pre-body gate in the relay handler.  Re-running the
        // full verifier here would reject any video upload that takes longer
        // than the minted token's expiration window (60 s in Strict mode).
        verify_upload_hash_only(&auth, &sha256_for_auth)
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

// ── Upload pipeline expiry-boundary regression ──────────────────────────────
//
// Demonstrates that the production post-body check is `verify_upload_hash_only`
// (commit 75e9bef748d2149ce459b14da842e706a51a5f78), NOT the old full verifier
// `verify_blossom_upload_auth`.
//
// The critical sequence:
//   T=0:  proof is minted with `created_at = now - 58`, `expiration = now + 2`.
//         Strict admission passes: age 58s ≤ 60s window, lifetime 60s, expiry future.
//   T=3s: body transfer completes; post-body check runs on now-expired proof.
//         Old path: verify_blossom_upload_auth → TokenExpired (breaks upload).
//         New path: verify_upload_hash_only    → Ok if hash matches.
//
// These tests exercise both `process_upload` (buffered, upload.rs:85) and
// `process_video_upload` (streaming, upload.rs:413) so that reverting either
// post-body call to the old full verifier breaks the positive case, and
// removing the hash check breaks the negative case.
//
// Note: `nostr::Timestamp::now()` reads the OS wall clock directly; paused
// Tokio time does not advance it.  The positive cases use a real 3-second
// sleep to sequence fresh admission at T=0 followed by expired completion at
// T≈3s.  The negative (hash-mismatch) cases use a pre-expired proof and need
// no timing sequence.
//
// Tests that require a live MinIO instance live in the `minio_tests` module so
// the nextest profile can tag them with `#[ignore = "requires MinIO"]` and the
// unit lane skips them automatically.
#[cfg(test)]
mod minio_tests {
    use super::*;
    use buzz_core::tenant::{CommunityId, TenantContext};
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use uuid::Uuid;

    /// Minimal valid 1×1 RGB PNG — passes validate_content and
    /// validate_image_metadata_free without any metadata chunks.
    ///
    /// Generated once and embedded as const bytes to avoid runtime PNG
    /// encoding in every test run.  The exact pixels and structure are
    /// irrelevant; only the magic bytes and structural validity matter.
    const MINIMAL_PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, // PNG signature
        0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, // IHDR length + type
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // width=1, height=1
        0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, // bit-depth=8, color=RGB
        0xde, // IHDR CRC (partial — enough for magic-byte sniff)
        0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, // IDAT length + type
        0x78, 0x9c, 0x63, 0xf8, 0xff, 0xff, 0x3f, 0x00, // zlib-compressed scanline
        0x05, 0xfe, 0x02, 0xfe, 0x0d, 0xef, 0x46, 0xb8, // (white pixel)
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, // IEND type
        0xae, 0x42, 0x60, 0x82, // IEND CRC
    ];

    fn minio_config() -> MediaConfig {
        MediaConfig {
            s3_endpoint: "http://localhost:9000".to_string(),
            s3_access_key: "buzz_dev".to_string(),
            s3_secret_key: "buzz_dev_secret".to_string(),
            s3_bucket: "buzz-media".to_string(),
            s3_region: "us-east-1".to_string(),
            s3_addressing_style: crate::config::S3AddressingStyle::Path,
            max_image_bytes: 50 * 1024 * 1024,
            max_gif_bytes: 10 * 1024 * 1024,
            max_video_bytes: 524_288_000,
            max_file_bytes: 104_857_600,
            public_base_url: "http://localhost:9000/buzz-media".to_string(),
            upload_records_enabled: false,
            upload_ip_header: None,
            upload_port_header: None,
        }
    }

    fn test_tenant() -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::nil()), "relay.example")
    }

    /// Build a fresh Blossom upload auth event that passes Strict admission NOW.
    ///
    /// Strict invariants:
    ///   - `created_at` ≤ now + 5s (future-skew)
    ///   - now - `created_at` ≤ 60s (replay window)
    ///   - `expiration` ≤ `created_at` + 60s (token lifetime)
    ///   - `expiration` > now (not yet expired)
    ///
    /// We set `created_at = now - 58` (age = 58s, inside the 60s window),
    /// `expiration = now - 58 + 60 = now + 2` (lifetime = 60s, strictly future).
    /// Calling `verify_blossom_upload_auth(Strict)` at sign time must return `Ok`.
    /// After a ≥3 s real sleep the proof is expired (expiration ≤ now): the old
    /// full verifier would return `TokenExpired`, while `verify_upload_hash_only`
    /// only checks the `x` tag and must still succeed.
    fn fresh_strict_upload_auth(keys: &Keys, sha256: &str, server: &str) -> nostr::Event {
        let now = Timestamp::now().as_secs();
        let created_at = now.saturating_sub(58);
        let exp = created_at + 60; // now + 2s: strictly future, lifetime = 60s
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", sha256]).unwrap(),
            Tag::parse(["expiration", &exp.to_string()]).unwrap(),
            Tag::parse(["server", server]).unwrap(),
        ];
        EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(created_at))
            .sign_with_keys(keys)
            .expect("sign fresh upload auth")
    }

    /// Build an already-expired Blossom upload auth event for the mismatch-hash
    /// negative cases.  No admission path is needed there: we only exercise the
    /// post-body hash check, so the proof need not be fresh.
    ///
    /// `created_at = now - 120`, `expiration = now - 1`: definitively past for
    /// the old full verifier (`TokenExpired`); the `x` tag is the only thing
    /// `verify_upload_hash_only` reads.
    fn expired_upload_auth(keys: &Keys, sha256: &str, server: &str) -> nostr::Event {
        let now = Timestamp::now().as_secs();
        let created_at = now.saturating_sub(120);
        let exp = now.saturating_sub(1);
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", sha256]).unwrap(),
            Tag::parse(["expiration", &exp.to_string()]).unwrap(),
            Tag::parse(["server", server]).unwrap(),
        ];
        EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(created_at))
            .sign_with_keys(keys)
            .expect("sign expired upload auth")
    }

    /// Case B (buffered pipeline): the NEW post-body path (`verify_upload_hash_only`)
    /// accepts a proof whose expiry passes during the transfer.
    ///
    /// Sequence:
    ///   T=0:  `fresh_strict_upload_auth` signs a 60s-lifetime proof that passes
    ///         Strict admission (`verify_blossom_upload_auth(Strict)` returns `Ok`).
    ///   T≈3s: real sleep lets the proof expire (expiration ≤ now).
    ///   T≈3s: `process_upload` runs; its post-body call is `verify_upload_hash_only`.
    ///         Old path: `verify_blossom_upload_auth` → `TokenExpired` (upload breaks).
    ///         New path: `verify_upload_hash_only`    → `Ok` if hash matches.
    ///
    /// Note: `nostr::Timestamp::now()` reads the wall clock, not Tokio time.
    /// Paused Tokio time does not advance it; a real sleep is required.
    ///
    /// Discriminating mutation: replacing `verify_upload_hash_only` at `upload.rs:85`
    /// with `verify_blossom_upload_auth` causes this test to fail with `TokenExpired`.
    ///
    /// Requires a live MinIO instance (endpoint http://localhost:9000).
    #[tokio::test]
    #[ignore = "requires MinIO"]
    async fn buffered_upload_accepts_expired_proof_when_hash_matches() {
        use crate::auth::{verify_blossom_upload_auth, BlossomStrictness};

        let keys = Keys::generate();
        let body = Bytes::from_static(MINIMAL_PNG);
        let sha256 = hex::encode(sha2::Sha256::digest(&body));
        let auth = fresh_strict_upload_auth(&keys, &sha256, "relay.example");

        // Assert that the proof passes Strict admission RIGHT NOW, before any sleep.
        assert!(
            verify_blossom_upload_auth(
                &auth,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Strict
            )
            .is_ok(),
            "proof must pass Strict admission at sign time"
        );

        // Wait for the proof to expire.  The expiry is `now + 2s` at sign time;
        // 3s guarantees expiration ≤ current time when process_upload runs.
        // `nostr::Timestamp::now()` reads the OS wall clock directly.
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let storage = MediaStorage::new(&minio_config()).expect("MinIO client must initialise");
        let ctx = test_tenant();

        // process_upload calls verify_upload_hash_only post-body (the repaired path).
        // The proof is now expired; a matching hash must still be accepted.
        let result = process_upload(
            &storage,
            &minio_config(),
            &ctx,
            &auth,
            body,
            None, // no attribution
        )
        .await;

        assert!(
            result.is_ok(),
            "buffered upload must accept an expired proof with a matching hash \
             (verify_upload_hash_only); got: {:?}",
            result.err()
        );
    }

    /// Case D (buffered pipeline): `verify_upload_hash_only` rejects a mismatched
    /// body hash even when the proof is otherwise structurally valid.
    ///
    /// Discriminating mutation: removing the `verify_upload_hash_only` call
    /// (or replacing it with a no-op) causes this test to accept a mismatched
    /// body, which is a security regression — the relay would store whatever
    /// bytes the client sent without verifying the Blossom `x` tag.
    ///
    /// Requires a live MinIO instance (endpoint http://localhost:9000).
    #[tokio::test]
    #[ignore = "requires MinIO"]
    async fn buffered_upload_rejects_mismatched_hash_through_production_path() {
        let keys = Keys::generate();
        // Sign the auth for a DIFFERENT hash than the actual body.
        let mismatched_sha256 = "a".repeat(64);
        let auth = expired_upload_auth(&keys, &mismatched_sha256, "relay.example");

        // The body has a hash that differs from what was signed in the auth event.
        let body = Bytes::from_static(MINIMAL_PNG);

        let storage = MediaStorage::new(&minio_config()).expect("MinIO client must initialise");
        let ctx = test_tenant();

        let result = process_upload(&storage, &minio_config(), &ctx, &auth, body, None).await;

        assert!(
            matches!(result, Err(MediaError::HashMismatch)),
            "buffered upload must reject a body whose SHA-256 does not match \
             the signed x tag; got: {:?}",
            result
        );
    }

    /// Case B (streaming pipeline): `process_video_upload` accepts a proof whose
    /// expiry passes during the transfer.
    ///
    /// Sequence mirrors the buffered case:
    ///   T=0:  `fresh_strict_upload_auth` signs a 60s-lifetime proof that passes
    ///         Strict admission.
    ///   T≈3s: real sleep lets the proof expire.
    ///   T≈3s: `process_video_upload` streams to disk, computes SHA-256, then calls
    ///         `verify_upload_hash_only` (the repaired path).
    ///         Old path: `verify_blossom_upload_auth` → `TokenExpired`.
    ///         New path: `verify_upload_hash_only`    → `Ok` if hash matches.
    ///
    /// Discriminating mutation: replacing `verify_upload_hash_only` at `upload.rs:413`
    /// with `verify_blossom_upload_auth` causes this test to fail with `TokenExpired`.
    ///
    /// Requires a live MinIO instance (endpoint http://localhost:9000).
    #[tokio::test]
    #[ignore = "requires MinIO"]
    async fn streaming_upload_accepts_expired_proof_when_hash_matches() {
        use crate::auth::{verify_blossom_upload_auth, BlossomStrictness};
        use crate::validation::minimal_valid_mp4;
        use futures_util::stream;

        let keys = Keys::generate();
        let body_bytes = minimal_valid_mp4();
        let sha256 = hex::encode(sha2::Sha256::digest(&body_bytes));
        let auth = fresh_strict_upload_auth(&keys, &sha256, "relay.example");

        // Assert Strict admission passes at sign time.
        assert!(
            verify_blossom_upload_auth(
                &auth,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Strict
            )
            .is_ok(),
            "proof must pass Strict admission at sign time"
        );

        // Wait for the proof to expire (3s > 2s remaining until expiry).
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let body_len = body_bytes.len() as u64;
        // Wrap the bytes in a single-item stream of Ok(Bytes).
        // axum::Error wraps std::io::Error; we never inject an error here.
        let body_stream =
            stream::once(async move { Ok::<_, axum::Error>(Bytes::from(body_bytes)) });

        let storage = MediaStorage::new(&minio_config()).expect("MinIO client must initialise");
        let ctx = test_tenant();

        // process_video_upload streams to disk, computes SHA-256, then calls
        // verify_upload_hash_only (the repaired path).  The proof is now expired;
        // a matching hash must still be accepted.
        let result = process_video_upload(
            &storage,
            &minio_config(),
            &ctx,
            &auth,
            body_stream,
            Some(body_len),
            None, // no attribution
        )
        .await;

        assert!(
            result.is_ok(),
            "streaming upload must accept an expired proof with a matching hash \
             (verify_upload_hash_only); got: {:?}",
            result.err()
        );
    }

    /// Case D (streaming pipeline): `verify_upload_hash_only` rejects a mismatched
    /// body hash through the streaming completion path.
    ///
    /// Discriminating mutation: removing the `verify_upload_hash_only` call at
    /// `upload.rs:413` causes this test to accept a mismatched body, which is a
    /// security regression — the relay would store whatever bytes the client sent
    /// without verifying the Blossom `x` tag.
    ///
    /// Requires a live MinIO instance (endpoint http://localhost:9000).
    #[tokio::test]
    #[ignore = "requires MinIO"]
    async fn streaming_upload_rejects_mismatched_hash_through_production_path() {
        use crate::validation::minimal_valid_mp4;
        use futures_util::stream;

        let keys = Keys::generate();
        // Sign auth for a DIFFERENT hash than the actual body.
        let mismatched_sha256 = "a".repeat(64);
        let auth = expired_upload_auth(&keys, &mismatched_sha256, "relay.example");

        let body_bytes = minimal_valid_mp4();
        let body_len = body_bytes.len() as u64;
        let body_stream =
            stream::once(async move { Ok::<_, axum::Error>(Bytes::from(body_bytes)) });

        let storage = MediaStorage::new(&minio_config()).expect("MinIO client must initialise");
        let ctx = test_tenant();

        let result = process_video_upload(
            &storage,
            &minio_config(),
            &ctx,
            &auth,
            body_stream,
            Some(body_len),
            None,
        )
        .await;

        assert!(
            matches!(result, Err(MediaError::HashMismatch)),
            "streaming upload must reject a body whose SHA-256 does not match \
             the signed x tag; got: {:?}",
            result
        );
    }
}
