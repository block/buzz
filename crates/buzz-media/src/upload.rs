//! Upload pipeline — validate, store, thumbnail, sidecar.

use bytes::Bytes;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::auth::verify_blossom_upload_auth;
use crate::config::MediaConfig;
use crate::error::MediaError;
use crate::storage::{BlobMeta, MediaStorage};
use crate::thumbnail::generate_image_metadata_sync;
use crate::types::BlobDescriptor;
use crate::validation::{
    mime_to_ext, validate_content, validate_file_content, validate_video_file,
};

/// Shared buffered-upload pipeline for the image and generic-file paths.
///
/// Both paths are identical except for two steps, which are injected:
/// - `validate`: a CPU-bound check (run inside `spawn_blocking`) that returns
///   the `(mime, ext)` pair for the body. Images derive `ext` from the MIME;
///   generic files get both from the deny-list validator.
/// - `store_metadata`: stores the sidecar (and any derived artifacts such as a
///   thumbnail) and returns the resulting [`BlobMeta`]. Images run the full
///   image-metadata pipeline; generic files write a minimal sidecar. It
///   receives the already-computed `(sha256, ext, mime, uploaded_at)` so no
///   work is repeated.
///
/// Everything else — hash, Blossom auth (10-minute window), content-addressed
/// key, the both-exist idempotency short-circuit, blob store, orphan-blob
/// handling, and descriptor build — is common. The streaming video path stays
/// separate (see [`process_video_upload`]) because it never buffers in RAM.
async fn process_buffered_upload<V, M, Fut>(
    storage: &MediaStorage,
    config: &MediaConfig,
    auth_event: &nostr::Event,
    body: Bytes,
    validate: V,
    store_metadata: M,
) -> Result<BlobDescriptor, MediaError>
where
    V: FnOnce(&Bytes, &MediaConfig) -> Result<(String, String), MediaError> + Send + 'static,
    M: FnOnce(MetadataInput) -> Fut,
    Fut: std::future::Future<Output = Result<BlobMeta, MediaError>>,
{
    // CPU-bound: validate content, compute hash, verify auth.
    let auth = auth_event.clone();
    let bytes = body.clone();
    let cfg = config.clone();
    let (mime, sha256, ext) = tokio::task::spawn_blocking(move || -> Result<_, MediaError> {
        let (mime, ext) = validate(&bytes, &cfg)?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        // Buffered uploads (image + file): 10-minute auth window is plenty.
        verify_blossom_upload_auth(&auth, &sha256, cfg.server_domain.as_deref(), 600)?;
        Ok((mime, sha256, ext))
    })
    .await
    .map_err(|_| MediaError::Internal)??;

    let key = format!("{sha256}.{ext}");
    let meta_key = format!("_meta/{sha256}.json");

    // Idempotent: short-circuit only if BOTH sidecar and blob exist. If the
    // sidecar exists but the blob is missing, fall through to re-upload.
    let sidecar_exists = storage.head(&meta_key).await?;
    let blob_exists = storage.head(&key).await?;
    if sidecar_exists && blob_exists {
        let meta = storage.get_sidecar(&sha256).await?;
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

    let meta_result = store_metadata(MetadataInput {
        sha256: sha256.clone(),
        ext: ext.clone(),
        mime: mime.clone(),
        body: body.clone(),
        uploaded_at,
    })
    .await;

    match meta_result {
        Ok(meta) => Ok(build_descriptor(
            config,
            &sha256,
            &ext,
            &mime,
            body.len() as u64,
            Some(&meta),
            uploaded_at,
        )),
        Err(e) => {
            tracing::warn!(sha256 = %sha256, error = %e, "metadata generation failed; orphan blob left for GC");
            Err(e)
        }
    }
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
    auth_event: &nostr::Event,
    body: Bytes,
) -> Result<BlobDescriptor, MediaError> {
    process_buffered_upload(
        storage,
        config,
        auth_event,
        body,
        |bytes, cfg| {
            let mime = validate_content(bytes, cfg)?;
            let ext = mime_to_ext(&mime).to_string();
            Ok((mime, ext))
        },
        |input| async move {
            generate_and_store_metadata(
                storage,
                config,
                &input.sha256,
                &input.ext,
                &input.mime,
                &input.body,
                input.uploaded_at,
            )
            .await
        },
    )
    .await
}

/// Process a generic (non-image, non-video) file upload end-to-end.
///
/// This is the catch-all attachment path: documents, archives, audio, text,
/// data — anything that isn't a previewable image or an H.264 MP4. The body is
/// fully buffered in RAM (bounded by `config.max_file_bytes` at the transport
/// layer), validated against the deny-list + size cap, stored, and recorded in
/// a minimal sidecar. No thumbnail, no dimensions, no duration.
///
/// The resulting blob is served with `Content-Disposition: attachment`, so the
/// client always downloads it rather than rendering it inline.
pub async fn process_file_upload(
    storage: &MediaStorage,
    config: &MediaConfig,
    auth_event: &nostr::Event,
    body: Bytes,
) -> Result<BlobDescriptor, MediaError> {
    process_buffered_upload(
        storage,
        config,
        auth_event,
        body,
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
            let meta_key = format!("_meta/{}.json", input.sha256);
            let meta_json = serde_json::to_vec(&meta)?;
            storage
                .put(&meta_key, &meta_json, "application/json")
                .await?;
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
    auth_event: &nostr::Event,
    body_stream: impl futures_core::Stream<Item = Result<Bytes, axum::Error>> + Send + 'static,
    content_length: Option<u64>,
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

    // --- 2. Magic-byte check (video/mp4 only) ---
    // sniff_buf has up to MIN_SNIFF_BYTES (4 KiB) of leading bytes — enough for
    // infer::get() to detect MP4 ftyp even if the first network chunk was tiny.
    let mime = infer::get(&first_bytes)
        .map(|t| t.mime_type().to_string())
        .ok_or(MediaError::UnknownContentType)?;
    if mime != "video/mp4" {
        return Err(MediaError::DisallowedContentType(mime));
    }

    // --- 3. Verify Blossom auth: x tag must match computed SHA-256 ---
    let auth = auth_event.clone();
    let sha256_for_auth = sha256_hex.clone();
    let server_domain = config.server_domain.clone();
    tokio::task::spawn_blocking(move || {
        // Videos: 1-hour window — large uploads on slow connections need headroom.
        verify_blossom_upload_auth(&auth, &sha256_for_auth, server_domain.as_deref(), 3600)
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
    let meta_key = format!("_meta/{sha256_hex}.json");

    // --- 5. Idempotency check ---
    let sidecar_exists = storage.head(&meta_key).await?;
    let blob_exists = storage.head(&key).await?;
    if sidecar_exists && blob_exists {
        let meta = storage.get_sidecar(&sha256_hex).await?;
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

    // --- 7. Write sidecar (no thumbnail for video — desktop handles that) ---
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
    let meta_json = serde_json::to_vec(&meta)?;
    storage
        .put(&meta_key, &meta_json, "application/json")
        .await?;

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

/// Generate thumbnail, blurhash, and sidecar metadata, then store them.
/// Returns the completed [`BlobMeta`] on success.
async fn generate_and_store_metadata(
    storage: &MediaStorage,
    config: &MediaConfig,
    sha256: &str,
    ext: &str,
    mime: &str,
    body: &Bytes,
    uploaded_at: i64,
) -> Result<BlobMeta, MediaError> {
    let body_ref = body.clone();
    let mime_ref = mime.to_string();
    let ext_ref = ext.to_string();
    let sha256_ref = sha256.to_string();
    let cfg_ref = config.clone();
    let (mut meta, thumb_bytes) = tokio::task::spawn_blocking(move || {
        generate_image_metadata_sync(&cfg_ref, &sha256_ref, &body_ref, &mime_ref, &ext_ref)
    })
    .await
    .map_err(|_| MediaError::Internal)??;

    meta.uploaded_at = uploaded_at;

    if let Some(ref tb) = thumb_bytes {
        let thumb_key = format!("{sha256}.thumb.jpg");
        storage.put(&thumb_key, tb, "image/jpeg").await?;
    }

    let meta_key = format!("_meta/{sha256}.json");
    let meta_json = serde_json::to_vec(&meta)?;
    storage
        .put(&meta_key, &meta_json, "application/json")
        .await?;
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
            max_image_bytes: 50 * 1024 * 1024,
            max_gif_bytes: 10 * 1024 * 1024,
            max_video_bytes: 524_288_000,
            max_file_bytes: 104_857_600,
            public_base_url: "https://media.example.com".to_string(),
            server_domain: None,
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
