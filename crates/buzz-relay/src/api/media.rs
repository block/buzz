//! Blossom-compatible media upload, retrieval, and existence check handlers.
//!
//! Routes:
//!   PUT  /media/upload          — BUD-02 upload (auth required)
//!   GET  /media/{sha256_ext}    — BUD-01 serve blob
//!   HEAD /media/{sha256_ext}    — BUD-01 existence check

use std::sync::Arc;

use axum::http::header;
use axum::{
    extract::{FromRequestParts, Path, State},
    http::{request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use sha2::{Digest, Sha256};
use sprout_audit::{AuditAction, NewAuditEntry};
use sprout_auth::Scope;
use sprout_media::{BlobDescriptor, MediaError};

use crate::state::AppState;

// ── Upload ────────────────────────────────────────────────────────────────────

/// Axum extractor that validates Blossom auth + API token scopes from headers
/// BEFORE the request body is read. This prevents unauthenticated clients from
/// forcing the server to buffer up to 50MB of body data.
///
/// Axum processes `FromRequestParts` extractors before `FromRequest` (body)
/// extractors, so auth rejection happens before any body buffering.
pub(crate) struct AuthenticatedUpload {
    auth_event: nostr::Event,
    #[allow(dead_code)] // scopes validated in extractor; stored for future per-scope handler logic
    scopes: Vec<Scope>,
}

impl FromRequestParts<Arc<AppState>> for AuthenticatedUpload {
    type Rejection = MediaError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let headers = &parts.headers;

        // 1. Extract and validate Blossom auth event
        let auth_event = extract_blossom_auth(headers)?;
        // Use the permissive window (3600s) here because we don't know the
        // content type yet.  The upload functions re-verify with the correct
        // per-type window (600s for images, 3600s for video) after the body
        // has been consumed and the SHA-256 computed.
        sprout_media::auth::verify_blossom_auth_event(
            &auth_event,
            state.config.media.server_domain.as_deref(),
            3600,
        )?;

        // 2. Require X-SHA-256 header (BUD-11: mandatory for PUT /upload)
        let claimed_hash = headers
            .get("x-sha-256")
            .and_then(|v| v.to_str().ok())
            .ok_or(MediaError::MissingTag("x-sha-256"))?;

        // Validate format: exactly 64 lowercase hex characters
        if claimed_hash.len() != 64
            || !claimed_hash
                .chars()
                .all(|c| matches!(c, '0'..='9' | 'a'..='f'))
        {
            return Err(MediaError::HashMismatch);
        }

        // 3. Validate X-SHA-256 matches at least one x tag in the auth event
        let has_matching_x = auth_event
            .tags
            .iter()
            .any(|tag| tag.kind().to_string() == "x" && (tag.content() == Some(claimed_hash)));
        if !has_matching_x {
            return Err(MediaError::HashMismatch);
        }

        // 4. Resolve scopes (API token or dev mode)
        let scopes = resolve_upload_scopes(headers, state, &auth_event.pubkey).await?;
        sprout_auth::require_scope(&scopes, Scope::FilesWrite)
            .map_err(|_| MediaError::InsufficientScope)?;

        // 5. Relay membership gate (NIP-43).
        let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
        crate::api::relay_members::enforce_relay_membership(
            state,
            auth_event.pubkey.as_bytes(),
            auth_tag,
        )
        .await
        .map_err(|_| MediaError::RelayMembershipRequired)?;

        Ok(AuthenticatedUpload { auth_event, scopes })
    }
}

/// PUT /media/upload — Blossom BUD-02 upload.
///
/// Auth is validated via the [`AuthenticatedUpload`] extractor BEFORE the body
/// is read, preventing unauthenticated clients from forcing body buffering.
// AuthenticatedUpload is pub(crate) — it's an internal extractor type, never
// exposed outside this crate. The warning is benign: axum resolves it at
// compile time via trait bounds, not by name.
#[allow(private_interfaces)]
///
/// Expects:
///   - `Authorization: Nostr <base64(kind:24242 event)>` — Blossom auth
///   - `X-SHA-256: <hex>` — Required per BUD-11
///   - `X-Auth-Token: sprout_*` — API token for scope resolution (optional in dev mode)
///   - `Content-Type: video/mp4` — routes to video validation path; all other types use image path
///   - Raw binary body (the file bytes)
///
/// Returns a [`BlobDescriptor`] JSON on success.
// TODO(v2): Add per-pubkey upload rate limiting and storage quotas to prevent
// bandwidth/storage exhaustion from authenticated callers. Currently mitigated by
// auth requirement (API token + Blossom signature) and body size limit.
pub async fn upload_blob(
    State(state): State<Arc<AppState>>,
    auth: AuthenticatedUpload,
    headers: HeaderMap,
    body: axum::body::Body,
) -> Result<Json<BlobDescriptor>, MediaError> {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let descriptor = if content_type.starts_with("video/") {
        // Video path: stream body directly to disk — never fully buffered in RAM.
        let content_length = headers
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        sprout_media::process_video_upload(
            &state.media_storage,
            &state.config.media,
            &auth.auth_event,
            body.into_data_stream(),
            content_length,
        )
        .await?
    } else {
        // Non-video path: buffer the body (bounded by the larger of the image
        // and generic-file caps), then decide image-vs-generic by sniffed MIME.
        // Images go through the thumbnailing pipeline; everything else (docs,
        // archives, audio, text, data) takes the generic file path and is
        // served as a download.
        let max = state
            .config
            .media
            .max_image_bytes
            .max(state.config.media.max_file_bytes);
        let bytes = axum::body::to_bytes(body, max as usize)
            .await
            .map_err(|_| MediaError::FileTooLarge { size: 0, max })?;

        let is_image = matches!(
            infer::get(&bytes).map(|t| t.mime_type()),
            Some("image/jpeg" | "image/png" | "image/gif" | "image/webp")
        );

        if is_image {
            sprout_media::process_upload(
                &state.media_storage,
                &state.config.media,
                &auth.auth_event,
                bytes,
            )
            .await?
        } else {
            sprout_media::process_file_upload(
                &state.media_storage,
                &state.config.media,
                &auth.auth_event,
                bytes,
            )
            .await?
        }
    };

    // Normalize MIME to a known set to bound label cardinality.
    let mime_label = match descriptor.mime_type.as_str() {
        "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "video/mp4" => {
            &descriptor.mime_type
        }
        _ => "other",
    };
    metrics::counter!("sprout_media_uploads_total", "mime" => mime_label.to_owned()).increment(1);

    // Audit via bounded channel — same pattern as event audit.
    let desc = descriptor.clone();
    let uploader = auth.auth_event.pubkey.to_hex();
    if let Err(e) = state
        .audit_tx
        .send(NewAuditEntry {
            event_id: desc.sha256.clone(),
            event_kind: sprout_core::kind::KIND_MEDIA_UPLOAD,
            actor_pubkey: uploader,
            action: AuditAction::MediaUploaded,
            channel_id: None,
            metadata: serde_json::json!({
                "sha256": desc.sha256,
                "size": desc.size,
                "mime": desc.mime_type,
            }),
        })
        .await
    {
        tracing::error!("Media audit channel closed — entry lost: {e}");
        metrics::counter!("sprout_audit_send_errors_total").increment(1);
    }

    Ok(Json(descriptor))
}

// ── Serve ─────────────────────────────────────────────────────────────────────

/// Whether a path-segment extension is a safe token.
///
/// The sidecar's `ext` field is the *authoritative* extension — the serve and
/// resolve paths always compare the requested ext against it. This check is a
/// cheap structural gate to reject obviously hostile path segments (traversal,
/// overlong, non-alphanumeric) before any storage lookup. Accepts 1–8 lowercase
/// alphanumeric chars, which covers every extension the generic file path emits
/// (jpg, png, mp4, pdf, docx, xlsx, tar, 7z, mp3, flac, json, bin, …).
pub(crate) fn is_safe_ext(ext: &str) -> bool {
    !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9'))
}

/// Validate that `sha256_ext` is a safe path segment.
///
/// Accepted forms (max 3 segments):
///   - `{sha256}`                   — bare 64-char lowercase hex
///   - `{sha256}.{ext}`             — hash + extension
///   - `{sha256}.thumb.jpg`          — hash + thumb variant (always JPEG)
///
/// `{ext}` must be a safe token (see [`is_safe_ext`]); the sidecar comparison
/// downstream enforces the actual canonical extension.
/// Rejects path traversal, leading underscores, and any non-hex first segment.
fn validate_media_path(sha256_ext: &str) -> Result<(), MediaError> {
    let segments: Vec<&str> = sha256_ext.split('.').collect();

    // 1–3 segments only (hash, optional thumb, optional ext)
    if segments.is_empty() || segments.len() > 3 {
        return Err(MediaError::NotFound);
    }

    // First segment must be exactly 64 lowercase hex chars (SHA-256)
    let hash = segments[0];
    if hash.len() != 64 || !hash.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
        return Err(MediaError::NotFound);
    }

    // Validate remaining segments
    match segments.len() {
        1 => {} // bare hash — ok
        2 => {
            // {hash}.{ext}
            if !is_safe_ext(segments[1]) {
                return Err(MediaError::NotFound);
            }
        }
        3 => {
            // {hash}.thumb.jpg — thumbnails are always JPEG
            if segments[1] != "thumb" || segments[2] != "jpg" {
                return Err(MediaError::NotFound);
            }
        }
        _ => return Err(MediaError::NotFound),
    }

    Ok(())
}

/// Maximum bytes returned in a single 206 range response (16 MiB).
///
/// Caps memory per request and prevents clients from using range requests to
/// bypass the intent of chunked delivery. Clients that need more simply issue
/// additional range requests.
const MAX_RANGE_CHUNK: u64 = 16 * 1024 * 1024;

/// GET /media/{sha256_ext} — Blossom BUD-01 serve blob, with HTTP 206 range support.
///
/// `sha256_ext` is either:
///   - `<sha256>.<ext>` — direct key (e.g. `abc123.jpg`)
///   - `<sha256>` — bare hash; extension resolved from sidecar
///   - `<sha256>.thumb.jpg` — thumbnail variant
///
/// Range request behaviour (RFC 9110 §14.2):
///   - No `Range` header → 200 with full body
///   - `Range: bytes=START-END` → 206 with slice; `Content-Range: bytes START-END/TOTAL`
///   - Unsatisfiable range (start ≥ total) → 416 with `Content-Range: bytes */TOTAL`
///   - Suffix ranges (`bytes=-N`) → 206 with last N bytes (RFC 9110 §14.1.2)
///   - Chunk capped at 16 MiB; clients request additional ranges for the rest
///
/// All responses include `Accept-Ranges: bytes` so video players know seeking is supported.
pub async fn get_blob(
    State(state): State<Arc<AppState>>,
    Path(sha256_ext): Path<String>,
    req_headers: HeaderMap,
) -> Result<Response, MediaError> {
    validate_media_path(&sha256_ext)?;

    // Sidecar gate FIRST — reject before any blob I/O. Storage is not authoritative.
    let content_type = if sha256_ext.ends_with(".thumb.jpg") {
        let parent_hash = sha256_ext.strip_suffix(".thumb.jpg").unwrap_or(&sha256_ext);
        let _ = state
            .media_storage
            .read_sidecar_mime(parent_hash)
            .await
            .ok_or(MediaError::NotFound)?;
        "image/jpeg".to_string()
    } else {
        // For explicit paths (hash.ext), verify the requested extension matches
        // the sidecar's canonical extension — sidecar is authoritative.
        let sidecar_mime = state
            .media_storage
            .read_sidecar_mime(&sha256_ext)
            .await
            .ok_or(MediaError::NotFound)?;
        if sha256_ext.contains('.') {
            let requested_ext = sha256_ext.rsplit('.').next().unwrap_or("");
            let sidecar = state
                .media_storage
                .get_sidecar(sha256_ext.split('.').next().unwrap_or(&sha256_ext))
                .await
                .map_err(|_| MediaError::NotFound)?;
            if requested_ext != sidecar.ext {
                return Err(MediaError::NotFound);
            }
        }
        sidecar_mime
    };

    // Images and video render inline; generic files force download. This is the
    // primary defence for non-previewable types — combined with `nosniff` and
    // `CSP: default-src 'none'`, an attachment disposition prevents an uploaded
    // file from ever executing or rendering as active content in the client.
    let disposition = if sprout_media::serve_inline(&content_type) {
        "inline"
    } else {
        "attachment"
    };

    let key = resolve_s3_key(&state.media_storage, &sha256_ext).await?;

    // Parse optional Range header.
    let range_header = req_headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    // Extract single-range value, if present. Multi-range (comma-separated) is
    // unsupported — we ignore it and serve the full body per RFC 9110 §14.2:
    // "A server MAY ignore the Range header field."
    let single_range = range_header.filter(|r| !r.contains(','));

    match single_range {
        None => {
            // Full response — 200 OK. Stream from S3 — never loads full blob into RAM.
            let total = state
                .media_storage
                .head_with_metadata(&key)
                .await?
                .ok_or(MediaError::NotFound)?
                .size;
            let stream = state.media_storage.get_stream(&key).await?;
            let resp = axum::response::Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, &content_type)
                .header(header::CONTENT_LENGTH, total.to_string())
                .header(header::CONTENT_DISPOSITION, disposition)
                .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
                .header(header::CONTENT_SECURITY_POLICY, "default-src 'none'")
                .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                .header(header::ACCEPT_RANGES, "bytes")
                .body(axum::body::Body::from_stream(stream))
                .map_err(|_| MediaError::Internal)?;
            Ok(resp)
        }
        Some(range_str) => {
            // Single-range request — HEAD first to get total size without loading the blob.
            let total = state
                .media_storage
                .head_with_metadata(&key)
                .await?
                .ok_or(MediaError::NotFound)?
                .size;

            // Parse range: "bytes=START-END", "bytes=START-", or "bytes=-N" (suffix).
            let parsed = parse_byte_range(&range_str, total);
            match parsed {
                Some((start, end)) => {
                    if start >= total {
                        return axum::response::Response::builder()
                            .status(StatusCode::RANGE_NOT_SATISFIABLE)
                            .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                            .body(axum::body::Body::empty())
                            .map_err(|_| MediaError::Internal);
                    }

                    // Clamp end to total-1, then cap chunk size.
                    let end = end.min(total.saturating_sub(1));
                    let end = end
                        .min(start.saturating_add(MAX_RANGE_CHUNK - 1))
                        .min(total.saturating_sub(1));

                    // S3-native range GET — never loads the full blob into RAM.
                    let chunk = state.media_storage.get_range(&key, start, end).await?;
                    let content_range = format!("bytes {start}-{end}/{total}");

                    Ok(axum::response::Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::CONTENT_TYPE, &content_type)
                        .header(header::CONTENT_RANGE, content_range)
                        .header(header::CONTENT_LENGTH, chunk.len().to_string())
                        .header(header::CONTENT_DISPOSITION, disposition)
                        .header(header::ACCEPT_RANGES, "bytes")
                        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
                        .header(header::CONTENT_SECURITY_POLICY, "default-src 'none'")
                        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                        .body(axum::body::Body::from(chunk))
                        .map_err(|_| MediaError::Internal)?)
                }
                None => Ok(axum::response::Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                    .body(axum::body::Body::empty())
                    .map_err(|_| MediaError::Internal)?),
            }
        }
    }
}

/// Parse a `Range: bytes=START-END` header value.
///
/// Returns `Some((start, end))` for a valid absolute or suffix range.
/// Supported forms:
///   - `bytes=START-END` → absolute range
///   - `bytes=START-`    → from START to end of file
///   - `bytes=-N`        → last N bytes (suffix range, per RFC 9110 §14.1.2)
///
/// Returns `None` for malformed values or non-bytes units — callers respond with 416.
fn parse_byte_range(range: &str, total: u64) -> Option<(u64, u64)> {
    let range = range.strip_prefix("bytes=")?;

    // Suffix range: "bytes=-N" → last N bytes of the file.
    if let Some(suffix) = range.strip_prefix('-') {
        let n: u64 = suffix.parse().ok()?;
        if n == 0 || total == 0 {
            return None;
        }
        let start = total.saturating_sub(n);
        return Some((start, total - 1));
    }

    let (start_str, end_str) = range.split_once('-')?;
    let start: u64 = start_str.parse().ok()?;

    // Open-ended range: "bytes=START-" → from start to end of file.
    let end: u64 = if end_str.is_empty() {
        u64::MAX
    } else {
        end_str.parse().ok()?
    };

    if start > end {
        return None;
    }

    Some((start, end))
}

/// HEAD /media/{sha256_ext} — Blossom BUD-01 existence check.
///
/// Content-type is derived from the validated sidecar only — never from raw S3
/// object metadata — to prevent MIME spoofing via tampered storage. If the sidecar
/// is missing, we return 404 rather than fall back to untrusted metadata.
pub async fn head_blob(
    State(state): State<Arc<AppState>>,
    Path(sha256_ext): Path<String>,
) -> Result<Response, MediaError> {
    validate_media_path(&sha256_ext)?;

    // Sidecar gate FIRST — reject before any blob I/O.
    let content_type = if sha256_ext.ends_with(".thumb.jpg") {
        let parent_hash = sha256_ext.strip_suffix(".thumb.jpg").unwrap_or(&sha256_ext);
        let _ = state
            .media_storage
            .read_sidecar_mime(parent_hash)
            .await
            .ok_or(MediaError::NotFound)?;
        "image/jpeg".to_string()
    } else {
        let sidecar_mime = state
            .media_storage
            .read_sidecar_mime(&sha256_ext)
            .await
            .ok_or(MediaError::NotFound)?;
        if sha256_ext.contains('.') {
            let requested_ext = sha256_ext.rsplit('.').next().unwrap_or("");
            let sidecar = state
                .media_storage
                .get_sidecar(sha256_ext.split('.').next().unwrap_or(&sha256_ext))
                .await
                .map_err(|_| MediaError::NotFound)?;
            if requested_ext != sidecar.ext {
                return Err(MediaError::NotFound);
            }
        }
        sidecar_mime
    };

    let key = resolve_s3_key(&state.media_storage, &sha256_ext).await?;
    match state.media_storage.head_with_metadata(&key).await? {
        Some(meta) => {
            let size_str = meta.size.to_string();
            Ok((
                StatusCode::OK,
                [
                    ("content-type", content_type.as_str()),
                    ("content-length", size_str.as_str()),
                    ("accept-ranges", "bytes"),
                    ("cache-control", "public, max-age=31536000, immutable"),
                ],
            )
                .into_response())
        }
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Resolve the S3 key from a URL path segment.
///
/// - `sha256.ext`       → used as-is (already validated by `validate_media_path`)
/// - `sha256` (no dot)  → read sidecar to get extension, return `sha256.ext`
///
/// Sidecar-derived extensions are validated as safe tokens to prevent
/// object-key confusion if sidecar data is ever tampered with.
async fn resolve_s3_key(
    storage: &sprout_media::MediaStorage,
    sha256_ext: &str,
) -> Result<String, MediaError> {
    if sha256_ext.contains('.') {
        Ok(sha256_ext.to_string())
    } else {
        let sidecar = storage
            .get_sidecar(sha256_ext)
            .await
            .map_err(|_| MediaError::NotFound)?;
        // Validate sidecar ext — never trust storage as authoritative for path construction
        if !is_safe_ext(&sidecar.ext) {
            return Err(MediaError::NotFound);
        }
        Ok(format!("{}.{}", sha256_ext, sidecar.ext))
    }
}

/// Extract and verify a kind:24242 Blossom auth event from the `Authorization` header.
///
/// Accepts both base64url (BUD-11 spec) and standard base64 (nostr-tools compat).
fn extract_blossom_auth(headers: &HeaderMap) -> Result<nostr::Event, MediaError> {
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};

    let header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(MediaError::MissingAuth)?;

    let token = header
        .strip_prefix("Nostr ")
        .ok_or(MediaError::InvalidAuthScheme)?;

    let json_bytes = URL_SAFE_NO_PAD
        .decode(token)
        .or_else(|_| STANDARD.decode(token))
        .map_err(|_| MediaError::InvalidBase64)?;

    let event: nostr::Event =
        serde_json::from_slice(&json_bytes).map_err(|_| MediaError::InvalidAuthEvent)?;

    Ok(event)
}

/// Resolve permission scopes for an upload caller.
///
/// Resolution order:
/// 1. `X-Auth-Token: sprout_*` header — API token path (validates owner matches Blossom signer)
/// 2. If `require_auth_token` is false (dev mode) — check pubkey allowlist, then grant file scopes
async fn resolve_upload_scopes(
    headers: &HeaderMap,
    state: &AppState,
    blossom_pubkey: &nostr::PublicKey,
) -> Result<Vec<Scope>, MediaError> {
    // 1. API token path — desktop sends Blossom auth in Authorization + token in X-Auth-Token.
    if let Some(token) = headers
        .get("x-auth-token")
        .and_then(|v| v.to_str().ok())
        .filter(|t| t.starts_with("sprout_"))
    {
        let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let record = state
            .db
            .get_api_token_by_hash_including_revoked(&hash)
            .await
            .map_err(|_| MediaError::Unauthorized)?
            .ok_or(MediaError::Unauthorized)?;

        if record.revoked_at.is_some() {
            return Err(MediaError::TokenRevoked);
        }
        if let Some(expires_at) = record.expires_at {
            if expires_at < chrono::Utc::now() {
                return Err(MediaError::TokenExpired);
            }
        }

        // Token owner must match the Blossom signer — prevents token theft attacks.
        let blossom_bytes = blossom_pubkey.to_bytes().to_vec();
        if record.owner_pubkey != blossom_bytes {
            return Err(MediaError::PubkeyMismatch);
        }

        return Ok(record
            .scopes
            .iter()
            .filter_map(|s| s.parse::<Scope>().ok())
            .collect());
    }

    // 2. Dev mode: no API token required.
    if state.config.require_auth_token {
        return Err(MediaError::Unauthorized);
    }

    // Dev mode is active — any valid Blossom signer can upload.
    // This must never be enabled in production.
    tracing::warn!(
        "dev mode upload: no API token required — ensure require_auth_token=true in production"
    );

    // 3. Pubkey allowlist check (dev mode only).
    if state.config.pubkey_allowlist_enabled {
        let pubkey_bytes = blossom_pubkey.to_bytes().to_vec();
        if !state
            .db
            .is_pubkey_allowed(&pubkey_bytes)
            .await
            .unwrap_or(false)
        {
            return Err(MediaError::Unauthorized);
        }
    }

    // Dev mode: grant file scopes.
    Ok(vec![Scope::FilesRead, Scope::FilesWrite])
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_HASH: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    #[test]
    fn test_validate_media_path_bare_hash() {
        assert!(validate_media_path(VALID_HASH).is_ok());
    }

    #[test]
    fn test_validate_media_path_hash_ext() {
        for ext in &["jpg", "png", "gif", "webp", "mp4"] {
            assert!(validate_media_path(&format!("{VALID_HASH}.{ext}")).is_ok());
        }
    }

    #[test]
    fn test_validate_media_path_thumb_jpg_only() {
        assert!(validate_media_path(&format!("{VALID_HASH}.thumb.jpg")).is_ok());
        // Other thumb extensions rejected — thumbnails are always JPEG
        assert!(validate_media_path(&format!("{VALID_HASH}.thumb.png")).is_err());
        assert!(validate_media_path(&format!("{VALID_HASH}.thumb.webp")).is_err());
    }

    #[test]
    fn test_validate_media_path_accepts_generic_exts() {
        // Path validation now accepts any safe ext token — the deny-list for
        // dangerous *content* lives in the upload validator, not here. The
        // sidecar ext comparison is the authoritative check at serve time.
        assert!(validate_media_path(&format!("{VALID_HASH}.pdf")).is_ok());
        assert!(validate_media_path(&format!("{VALID_HASH}.docx")).is_ok());
        assert!(validate_media_path(&format!("{VALID_HASH}.zip")).is_ok());
        assert!(validate_media_path(&format!("{VALID_HASH}.mp3")).is_ok());
        assert!(validate_media_path(&format!("{VALID_HASH}.bin")).is_ok());
    }

    #[test]
    fn test_validate_media_path_rejects_malformed_ext() {
        // Reject ext tokens that aren't safe: uppercase, too long, special chars.
        assert!(validate_media_path(&format!("{VALID_HASH}.PDF")).is_err());
        assert!(validate_media_path(&format!("{VALID_HASH}.toolongext")).is_err());
        // 3-segment paths are only valid as the `.thumb.jpg` variant; a
        // hash.tar.gz form is rejected (compound extensions aren't a thing here —
        // the canonical ext is a single token like `gz`).
        assert!(validate_media_path(&format!("{VALID_HASH}.tar.gz")).is_err());
    }

    #[test]
    fn test_is_safe_ext() {
        assert!(is_safe_ext("jpg"));
        assert!(is_safe_ext("docx"));
        assert!(is_safe_ext("7z"));
        assert!(is_safe_ext("bin"));
        assert!(!is_safe_ext("")); // empty
        assert!(!is_safe_ext("PDF")); // uppercase
        assert!(!is_safe_ext("ta r")); // space
        assert!(!is_safe_ext("toolongext")); // > 8 chars
        assert!(!is_safe_ext("../etc")); // traversal chars
    }

    #[test]
    fn test_validate_media_path_rejects_short_hash() {
        assert!(validate_media_path("abc123.jpg").is_err());
    }

    #[test]
    fn test_validate_media_path_rejects_uppercase_hash() {
        let upper = "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789";
        assert!(validate_media_path(&format!("{upper}.jpg")).is_err());
    }

    #[test]
    fn test_validate_media_path_rejects_traversal() {
        assert!(validate_media_path("../etc/passwd").is_err());
        assert!(validate_media_path(&format!("../{VALID_HASH}.jpg")).is_err());
    }

    #[test]
    fn test_validate_media_path_rejects_too_many_segments() {
        assert!(validate_media_path(&format!("{VALID_HASH}.thumb.jpg.extra")).is_err());
    }

    #[test]
    fn test_validate_media_path_rejects_empty() {
        assert!(validate_media_path("").is_err());
    }

    // ── Range request parsing ─────────────────────────────────────────────────

    #[test]
    fn test_parse_byte_range_basic() {
        assert_eq!(parse_byte_range("bytes=0-499", 1000), Some((0, 499)));
        assert_eq!(parse_byte_range("bytes=500-999", 1000), Some((500, 999)));
    }

    #[test]
    fn test_parse_byte_range_open_ended() {
        // "bytes=500-" means from 500 to end of file
        assert_eq!(parse_byte_range("bytes=500-", 1000), Some((500, u64::MAX)));
    }

    #[test]
    fn test_parse_byte_range_suffix() {
        // "bytes=-500" on a 1000-byte file → last 500 bytes
        assert_eq!(parse_byte_range("bytes=-500", 1000), Some((500, 999)));
    }

    #[test]
    fn test_parse_byte_range_suffix_larger_than_file() {
        // Suffix larger than file → clamp to start of file
        assert_eq!(parse_byte_range("bytes=-5000", 1000), Some((0, 999)));
    }

    #[test]
    fn test_parse_byte_range_suffix_zero() {
        // "bytes=-0" is nonsensical → None
        assert_eq!(parse_byte_range("bytes=-0", 1000), None);
    }

    #[test]
    fn test_parse_byte_range_suffix_empty_file() {
        // Suffix on empty file → None
        assert_eq!(parse_byte_range("bytes=-500", 0), None);
    }

    #[test]
    fn test_parse_byte_range_rejects_inverted() {
        // start > end is invalid
        assert_eq!(parse_byte_range("bytes=999-0", 1000), None);
    }

    #[test]
    fn test_parse_byte_range_rejects_non_bytes_unit() {
        assert_eq!(parse_byte_range("items=0-10", 1000), None);
    }

    #[test]
    fn test_parse_byte_range_rejects_malformed() {
        assert_eq!(parse_byte_range("bytes=abc-def", 1000), None);
        assert_eq!(parse_byte_range("garbage", 1000), None);
        assert_eq!(parse_byte_range("bytes=", 1000), None);
    }

    #[test]
    fn test_parse_byte_range_zero_start() {
        assert_eq!(parse_byte_range("bytes=0-0", 1000), Some((0, 0)));
    }
}
