//! imeta tag validation helpers — shared between ingest pipeline and bridge.

use buzz_media::validation::mime_to_ext;

/// Validate imeta tags for correctness and safety.
///
/// Shared between REST (send_message) and WebSocket (handle_event) paths.
/// Returns Ok(()) if all tags are valid, or a human-readable error string.
pub fn validate_imeta_tags(tags: &[Vec<String>], media_base_url: &str) -> Result<(), String> {
    const ALLOWED_IMETA_KEYS: &[&str] = &[
        "url", "m", "x", "size", "dim", "blurhash", "alt", "thumb", "fallback", "duration",
        "bitrate", "image", "filename",
    ];
    const SINGLETON_KEYS: &[&str] = &[
        "url", "m", "x", "size", "dim", "blurhash", "thumb", "alt", "duration", "bitrate", "image",
        "filename",
    ];
    // Previewable media MIME types — these get the strict url-extension
    // consistency check below (their ext is derived from the MIME). Generic
    // files carry arbitrary MIME types whose ext can't be derived from the MIME
    // alone, so their consistency is enforced against the sidecar in
    // `verify_imeta_blobs` rather than here.
    const MEDIA_MIME: &[&str] = &[
        "image/jpeg",
        "image/png",
        "image/gif",
        "image/webp",
        "video/mp4",
    ];

    for tag in tags {
        if tag.first().map(|s| s.as_str()) != Some("imeta") {
            return Err("only imeta tags allowed in media_tags".into());
        }

        let mut has_url = false;
        let mut has_m = false;
        let mut has_x = false;
        let mut has_size = false;
        let mut seen_keys = std::collections::HashSet::new();
        let mut url_value = String::new();
        let mut x_value = String::new();
        let mut m_value = String::new();
        let mut thumb_value = String::new();

        for part in tag.iter().skip(1) {
            let mut parts = part.splitn(2, ' ');
            let key = parts.next().unwrap_or("");
            let value = parts.next().unwrap_or("");

            if !ALLOWED_IMETA_KEYS.contains(&key) {
                return Err(format!("disallowed imeta key: {key}"));
            }
            if SINGLETON_KEYS.contains(&key) && !seen_keys.insert(key.to_string()) {
                return Err(format!("duplicate imeta key: {key}"));
            }

            match key {
                "url" => {
                    if !is_local_media_url(value, media_base_url) {
                        return Err("imeta url must be a local /media/ path".into());
                    }
                    if value.contains(".thumb.") {
                        return Err(
                            "imeta url must not be a thumbnail path; use thumb field".into()
                        );
                    }
                    url_value = value.to_string();
                    has_url = true;
                }
                "m" => {
                    // Accept any well-formed `type/subtype` MIME token. The
                    // authoritative gate is `verify_imeta_blobs`, which requires
                    // `m` to equal the stored sidecar MIME — and a sidecar only
                    // exists for content that passed the upload validator's
                    // deny-list. So a blocked type can never reach a valid imeta.
                    if !is_well_formed_mime(value) {
                        return Err("imeta m must be a valid MIME type".into());
                    }
                    m_value = value.to_string();
                    has_m = true;
                }
                "x" => {
                    if value.len() != 64
                        || !value.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
                    {
                        return Err("imeta x must be a 64-char lowercase hex SHA-256".into());
                    }
                    x_value = value.to_string();
                    has_x = true;
                }
                "size" => {
                    match value.parse::<u64>() {
                        Ok(0) | Err(_) => {
                            return Err("imeta size must be a positive integer".into())
                        }
                        Ok(_) => {}
                    }
                    has_size = true;
                }
                "thumb" => {
                    if !is_local_media_url(value, media_base_url) || !value.ends_with(".thumb.jpg")
                    {
                        return Err("imeta thumb must be a local .thumb.jpg path".into());
                    }
                    thumb_value = value.to_string();
                }
                "duration" => {
                    if let Ok(d) = value.parse::<f64>() {
                        if d <= 0.0 || d.is_nan() || d.is_infinite() {
                            return Err("imeta duration must be a positive finite number".into());
                        }
                    } else {
                        return Err("imeta duration must be a valid float".into());
                    }
                }
                "bitrate" if value.parse::<u64>().map_or(true, |b| b == 0) => {
                    return Err("imeta bitrate must be a positive integer".into());
                }
                "image" => {
                    const IMAGE_EXTS: &[&str] = &["jpg", "png", "gif", "webp"];
                    if !is_local_media_url(value, media_base_url) {
                        return Err("imeta image must be a local /media/ path".into());
                    }
                    if value.contains(".thumb.") {
                        return Err(
                            "imeta image must reference a standalone poster frame, not a thumbnail"
                                .into(),
                        );
                    }
                    let ext = value.rsplit('.').next().unwrap_or("");
                    if !IMAGE_EXTS.contains(&ext) {
                        return Err(
                            "imeta image must reference an image file (jpg, png, gif, webp), not video"
                                .into(),
                        );
                    }
                }
                "filename" => {
                    // Original filename for the file-card label. Bounded length;
                    // no path separators or control chars (it's display-only and
                    // must never influence storage keys, which are content-addressed).
                    if value.is_empty() || value.len() > 255 {
                        return Err("imeta filename must be 1–255 chars".into());
                    }
                    if value.contains('/')
                        || value.contains('\\')
                        || value.chars().any(|c| c.is_control())
                    {
                        return Err(
                            "imeta filename must not contain path separators or control characters"
                                .into(),
                        );
                    }
                }
                _ => {}
            }
        }

        if !has_url || !has_m || !has_x || !has_size {
            return Err("imeta tag must include url, m, x, and size".into());
        }

        // Video-only NIP-71 fields must not appear on image blobs.
        let is_video = m_value == "video/mp4";
        if !is_video {
            for key in &["duration", "bitrate", "image"] {
                if seen_keys.contains(*key) {
                    return Err(format!(
                        "imeta {key} is only valid for video/mp4, not {m_value}"
                    ));
                }
            }
        }

        // Cross-check internal consistency: url hash must match x, url ext must match m.
        if let Some(hash_in_url) = extract_hash_from_media_url(&url_value) {
            if hash_in_url != x_value {
                return Err("imeta url hash does not match x".into());
            }
        }
        if let Some(ext_in_url) = extract_ext_from_media_url(&url_value) {
            if MEDIA_MIME.contains(&m_value.as_str()) {
                // Previewable media: the ext is derivable from the MIME, so
                // enforce exact equality.
                let expected_ext = mime_to_ext(&m_value);
                if ext_in_url != expected_ext {
                    return Err("imeta url extension does not match m".into());
                }
            }
            // Generic files: ext can't be derived from the MIME. The sidecar
            // cross-check in `verify_imeta_blobs` enforces that the URL's ext
            // (and hash, size, MIME) match the stored blob.
        }
        if !thumb_value.is_empty() {
            if let Some(thumb_hash) = extract_hash_from_media_url(&thumb_value) {
                if thumb_hash != x_value {
                    return Err("imeta thumb hash does not match x".into());
                }
            }
        }
    }
    Ok(())
}

/// Verify that every imeta tag references a blob that actually exists in storage
/// and that the claimed metadata (size, MIME) matches the sidecar.
pub async fn verify_imeta_blobs(
    tags: &[Vec<String>],
    storage: &buzz_media::MediaStorage,
) -> Result<(), String> {
    for tag in tags {
        let mut x_value = String::new();
        let mut m_value = String::new();
        let mut size_value: u64 = 0;
        let mut thumb_value = String::new();
        let mut image_value = String::new();
        let mut duration_value: f64 = 0.0;

        for part in tag.iter().skip(1) {
            let mut parts = part.splitn(2, ' ');
            let key = parts.next().unwrap_or("");
            let value = parts.next().unwrap_or("");
            match key {
                "x" => x_value = value.to_string(),
                "m" => m_value = value.to_string(),
                "size" => size_value = value.parse().unwrap_or(0),
                "thumb" => thumb_value = value.to_string(),
                "image" => image_value = value.to_string(),
                "duration" => duration_value = value.parse().unwrap_or(0.0),
                _ => {}
            }
        }

        if x_value.is_empty() {
            continue;
        }

        // 1. Sidecar must exist
        let sidecar = storage
            .get_sidecar(&x_value)
            .await
            .map_err(|_| format!("imeta references nonexistent blob: {x_value}"))?;

        // 2. HEAD the actual blob object
        let blob_key = format!("{x_value}.{}", sidecar.ext);
        let blob_exists = storage
            .head(&blob_key)
            .await
            .map_err(|e| format!("storage error checking blob {x_value}: {e}"))?;
        if !blob_exists {
            return Err(format!("imeta blob object missing in storage: {x_value}"));
        }

        // 3. Cross-check claimed metadata against sidecar.
        if !m_value.is_empty() && sidecar.mime_type != m_value {
            return Err(format!(
                "imeta m ({m_value}) does not match stored MIME ({})",
                sidecar.mime_type
            ));
        }
        if size_value > 0 && sidecar.size != size_value {
            return Err(format!(
                "imeta size ({size_value}) does not match stored size ({})",
                sidecar.size
            ));
        }
        if let Some(stored_dur) = sidecar.duration_secs {
            if duration_value > 0.0 && (duration_value - stored_dur).abs() > 0.1 {
                return Err(format!(
                    "imeta duration ({duration_value}) does not match stored duration ({stored_dur})"
                ));
            }
        }

        // 4. If thumb is claimed, HEAD the thumbnail object too.
        if !thumb_value.is_empty() {
            let thumb_key = format!("{x_value}.thumb.jpg");
            let thumb_exists = storage
                .head(&thumb_key)
                .await
                .map_err(|e| format!("storage error checking thumbnail: {e}"))?;
            if !thumb_exists {
                return Err(format!(
                    "imeta thumb references missing thumbnail: {x_value}"
                ));
            }
        }

        // 5. If image (poster frame) is claimed, verify sidecar + blob.
        if !image_value.is_empty() {
            let img_hash = extract_hash_from_media_url(&image_value)
                .ok_or_else(|| format!("imeta image URL has no extractable hash: {image_value}"))?;

            let img_sidecar = storage
                .get_sidecar(img_hash)
                .await
                .map_err(|_| format!("imeta image references nonexistent poster: {img_hash}"))?;

            const IMAGE_MIMES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];
            if !IMAGE_MIMES.contains(&img_sidecar.mime_type.as_str()) {
                return Err(format!(
                    "imeta image poster MIME must be image type, got {}",
                    img_sidecar.mime_type
                ));
            }

            if let Some(url_ext) = extract_ext_from_media_url(&image_value) {
                if url_ext != img_sidecar.ext {
                    return Err(format!(
                        "imeta image extension ({url_ext}) does not match stored extension ({})",
                        img_sidecar.ext
                    ));
                }
            }

            let img_key = format!("{img_hash}.{}", img_sidecar.ext);
            let img_exists = storage
                .head(&img_key)
                .await
                .map_err(|e| format!("storage error checking poster image: {e}"))?;
            if !img_exists {
                return Err(format!(
                    "imeta image references missing poster frame: {img_hash}"
                ));
            }
        }
    }
    Ok(())
}

/// Whether a string is a well-formed `type/subtype` MIME token.
///
/// Structural check only — does not enforce a known type. The authoritative
/// content gate is the upload validator's deny-list plus the sidecar MIME
/// cross-check in `verify_imeta_blobs`. Rejects empties, missing slash,
/// whitespace, and control characters.
fn is_well_formed_mime(mime: &str) -> bool {
    let Some((ty, sub)) = mime.split_once('/') else {
        return false;
    };
    !ty.is_empty()
        && !sub.is_empty()
        && mime.len() <= 255
        && !mime.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Extract the 64-char hex hash from a `/media/{hash}.{ext}` URL.
fn extract_hash_from_media_url(url: &str) -> Option<&str> {
    let after = url.rsplit("/media/").next()?;
    let hash = after.split('.').next()?;
    if hash.len() == 64 && hash.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
        Some(hash)
    } else {
        None
    }
}

/// Extract the primary extension from a `/media/{hash}.{ext}` URL (not thumb).
fn extract_ext_from_media_url(url: &str) -> Option<&str> {
    let after = url.rsplit("/media/").next()?;
    let segments: Vec<&str> = after.split('.').collect();
    if segments.len() == 2 {
        Some(segments[1])
    } else {
        None
    }
}

/// Validate that a URL references a valid local media blob path.
fn is_local_media_url(url: &str, media_base_url: &str) -> bool {
    // A safe extension token: 1–8 lowercase alphanumeric chars. Covers media
    // (jpg, png, mp4) and every generic file ext (pdf, docx, zip, mp3, bin, …).
    // The blob's authoritative ext lives in the sidecar; this is a structural
    // gate. Shared with the serve/resolve paths so the predicate can't drift.
    use crate::api::media::is_safe_ext;

    let path_after_media = if let Some(rest) = url.strip_prefix("/media/") {
        rest
    } else {
        let base = media_base_url.trim_end_matches('/');
        let prefix = format!("{}/", base);
        if let Some(rest) = url.strip_prefix(&prefix) {
            rest
        } else {
            return false;
        }
    };

    if path_after_media.contains('?') || path_after_media.contains('#') {
        return false;
    }
    if path_after_media.contains('%') {
        return false;
    }

    let segments: Vec<&str> = path_after_media.split('.').collect();
    match segments.len() {
        2 => {
            let hash = segments[0];
            let ext = segments[1];
            hash.len() == 64
                && hash.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
                && is_safe_ext(ext)
        }
        3 => {
            let hash = segments[0];
            hash.len() == 64
                && hash.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
                && segments[1] == "thumb"
                && segments[2] == "jpg"
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    const BASE: &str = "https://relay.example.com/media";

    #[test]
    fn test_local_media_url_relative() {
        assert!(is_local_media_url(&format!("/media/{HASH}.jpg"), BASE));
        assert!(is_local_media_url(&format!("/media/{HASH}.png"), BASE));
    }

    #[test]
    fn test_local_media_url_absolute() {
        assert!(is_local_media_url(&format!("{BASE}/{HASH}.jpg"), BASE));
    }

    #[test]
    fn test_local_media_url_rejects_external() {
        assert!(!is_local_media_url(
            &format!("https://evil.com/media/{HASH}.jpg"),
            BASE
        ));
    }

    #[test]
    fn test_imeta_consistent_tags_pass() {
        let tag = vec![
            "imeta".into(),
            format!("url /media/{HASH}.jpg"),
            "m image/jpeg".into(),
            format!("x {HASH}"),
            "size 100".into(),
        ];
        assert!(validate_imeta_tags(&[tag], BASE).is_ok());
    }

    #[test]
    fn test_imeta_url_hash_must_match_x() {
        let other = "b".repeat(64);
        let tag = vec![
            "imeta".into(),
            format!("url /media/{HASH}.jpg"),
            "m image/jpeg".into(),
            format!("x {other}"),
            "size 100".into(),
        ];
        let err = validate_imeta_tags(&[tag], BASE).unwrap_err();
        assert!(err.contains("url hash does not match x"), "{err}");
    }

    #[test]
    fn test_imeta_generic_file_with_filename_passes() {
        // Generic file attachment: non-media MIME, arbitrary ext, filename label.
        // The url-ext-vs-MIME equality check is skipped for non-media MIMEs
        // (the sidecar cross-check in verify_imeta_blobs enforces correctness).
        let tag = vec![
            "imeta".into(),
            format!("url /media/{HASH}.pdf"),
            "m application/pdf".into(),
            format!("x {HASH}"),
            "size 2048".into(),
            "filename Q3-budget.pdf".into(),
        ];
        assert!(validate_imeta_tags(&[tag], BASE).is_ok());
    }

    #[test]
    fn test_imeta_octet_stream_passes() {
        // Un-sniffable text/data files upload as octet-stream with a .bin ext.
        let tag = vec![
            "imeta".into(),
            format!("url /media/{HASH}.bin"),
            "m application/octet-stream".into(),
            format!("x {HASH}"),
            "size 512".into(),
            "filename notes.txt".into(),
        ];
        assert!(validate_imeta_tags(&[tag], BASE).is_ok());
    }

    #[test]
    fn test_imeta_filename_rejects_path_separators() {
        let tag = vec![
            "imeta".into(),
            format!("url /media/{HASH}.pdf"),
            "m application/pdf".into(),
            format!("x {HASH}"),
            "size 2048".into(),
            "filename ../../etc/passwd".into(),
        ];
        let err = validate_imeta_tags(&[tag], BASE).unwrap_err();
        assert!(err.contains("filename"), "{err}");
    }

    #[test]
    fn test_imeta_rejects_malformed_mime() {
        let tag = vec![
            "imeta".into(),
            format!("url /media/{HASH}.bin"),
            "m not-a-mime".into(),
            format!("x {HASH}"),
            "size 512".into(),
        ];
        let err = validate_imeta_tags(&[tag], BASE).unwrap_err();
        assert!(err.contains("valid MIME"), "{err}");
    }

    #[test]
    fn test_is_well_formed_mime() {
        assert!(is_well_formed_mime("application/pdf"));
        assert!(is_well_formed_mime("application/octet-stream"));
        assert!(is_well_formed_mime("audio/mpeg"));
        assert!(!is_well_formed_mime("notamime"));
        assert!(!is_well_formed_mime("/pdf"));
        assert!(!is_well_formed_mime("application/"));
        assert!(!is_well_formed_mime("application/ pdf")); // whitespace
    }
}
