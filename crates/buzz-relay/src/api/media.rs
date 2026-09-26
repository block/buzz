//! Blossom-compatible media upload, retrieval, and existence check handlers.
//!
//! Routes:
//!   PUT  /upload                — BUD-02 exact-byte upload (auth required)
//!   PUT  /media/upload          — temporary media-only legacy alias
//!   GET  /media/{sha256_ext}    — BUD-01 serve blob
//!   HEAD /media/{sha256_ext}    — BUD-01 existence check

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::header;
use axum::http::HeaderValue;
use axum::{
    extract::{FromRequestParts, Path, State},
    http::{request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use buzz_audit::{AuditAction, NewAuditEntry};
use buzz_core::tenant::TenantContext;
use buzz_media::{BlobDescriptor, MediaError, UploadAttribution, UploadNetworkInfo};

use crate::state::AppState;

/// Lightweight pre-auth upload context: tenant + route mode only.
///
/// Used as the first-phase extractor for `upload_blob`. Blossom auth
/// extraction is deliberately NOT done here so it can run inside the
/// NIP-FI admission closure, ensuring that in Enforce mode a missing or
/// malformed Authorization header is mapped to the correct NIP-FI denial
/// bytes (MissingEvidence/EvidenceRejected) rather than legacy
/// `MediaError` JSON 401/403.  [FI-TRACE-AUTHORITY-UNIFORM]
// pub(crate) so axum can resolve the extractor from the pub handler signature.
pub(crate) struct UploadContext {
    tenant: TenantContext,
    route_mode: UploadRouteMode,
}

impl FromRequestParts<Arc<AppState>> for UploadContext {
    type Rejection = MediaError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let headers = &parts.headers;

        // Row zero: bind tenant from the request host.  Fail-closed:
        // unmapped host → 404.
        let raw_host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let tenant = crate::tenant::bind_community(&state.db, raw_host)
            .await
            .map_err(|_| MediaError::NotFound)?;

        let route_mode = upload_route_mode(parts.uri.path())?;

        Ok(UploadContext { tenant, route_mode })
    }
}

pub(crate) struct AuthenticatedUpload {
    auth_event: nostr::Event,
    /// Community resolved from the request host at extraction time (row zero for
    /// this HTTP door), identical to the WS door in `router.rs` and the bridge
    /// door in `bridge.rs`. Server-resolved, never client-supplied.
    tenant: TenantContext,
    route_mode: UploadRouteMode,
    _upload_permit: UploadPermit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UploadRouteMode {
    Upload,
    LegacyMedia,
}

fn should_stream_as_video(sniff: &[u8]) -> bool {
    infer::get(sniff).is_some_and(|kind| kind.mime_type() == "video/mp4")
        || buzz_media::looks_like_iso_bmff(sniff)
}

fn upload_route_mode(path: &str) -> Result<UploadRouteMode, MediaError> {
    match path {
        "/upload" => Ok(UploadRouteMode::Upload),
        "/media/upload" => Ok(UploadRouteMode::LegacyMedia),
        _ => Err(MediaError::NotFound),
    }
}

const MEDIA_UPLOAD_RATE_WINDOW: Duration = Duration::from_secs(60);

struct UploadPermit {
    _global: tokio::sync::OwnedSemaphorePermit,
    in_flight: Arc<dashmap::DashMap<crate::state::ScopedPubkeyKey, u32>>,
    key: crate::state::ScopedPubkeyKey,
}

impl Drop for UploadPermit {
    fn drop(&mut self) {
        use dashmap::mapref::entry::Entry;

        if let Entry::Occupied(mut entry) = self.in_flight.entry(self.key) {
            if *entry.get() <= 1 {
                entry.remove();
            } else {
                *entry.get_mut() -= 1;
            }
        }
    }
}

fn upload_rate_limited(
    state: &AppState,
    community_id: buzz_core::CommunityId,
    pubkey: &nostr::PublicKey,
) -> bool {
    let key = (community_id, pubkey.to_bytes());
    let now = Instant::now();
    let limit = state.config.media_uploads_per_minute;
    let mut entry = state
        .media_upload_rate_limiter
        .entry(key)
        .or_insert((0, now));
    let (count, window_start) = entry.value_mut();
    if now.duration_since(*window_start) >= MEDIA_UPLOAD_RATE_WINDOW {
        *count = 1;
        *window_start = now;
        return false;
    }
    if *count >= limit {
        return true;
    }
    *count += 1;
    false
}

fn acquire_upload_permit(
    state: &AppState,
    community_id: buzz_core::CommunityId,
    pubkey: &nostr::PublicKey,
) -> Result<UploadPermit, MediaError> {
    let global = state
        .media_upload_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| MediaError::UploadConcurrencyLimitReached)?;

    let key = (community_id, pubkey.to_bytes());
    let mut in_flight = state.media_uploads_in_flight.entry(key).or_insert(0);
    if *in_flight >= state.config.media_max_concurrent_uploads_per_pubkey {
        return Err(MediaError::UploadConcurrencyLimitReached);
    }
    *in_flight += 1;
    drop(in_flight);

    Ok(UploadPermit {
        _global: global,
        in_flight: Arc::clone(&state.media_uploads_in_flight),
        key,
    })
}

/// Build per-event upload attribution when upload records are enabled
/// (`BUZZ_MEDIA_UPLOAD_RECORDS`). Returns `None` when the feature is off —
/// the upload pipeline then writes no `_uploads/` record at all.
///
/// - `uploader_name` is the uploader's current display name in the bound
///   community (best-effort label; lookup failure degrades to absent).
/// - `net.ip` is read from the operator-configured trusted edge header
///   (`BUZZ_MEDIA_UPLOAD_IP_HEADER`) and validated as a public IP —
///   fail-empty: missing/malformed/non-public values record nothing. The
///   socket address is never used; behind a sidecar it is meaningless, and a
///   wrong address is worse than none.
/// - `net.port` (optional companion header) is only kept alongside a valid IP.
async fn upload_attribution(
    state: &AppState,
    auth: &AuthenticatedUpload,
    headers: &HeaderMap,
) -> Option<UploadAttribution> {
    let cfg = &state.config.media;
    if !cfg.upload_records_enabled {
        return None;
    }

    let uploader_name = state
        .db
        .get_user(auth.tenant.community(), &auth.auth_event.pubkey.to_bytes())
        .await
        .ok()
        .flatten()
        .and_then(|profile| profile.display_name)
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());

    let header_value = |name: &Option<String>| {
        name.as_deref()
            .and_then(|h| headers.get(h))
            .and_then(|v| v.to_str().ok())
    };
    let ip = header_value(&cfg.upload_ip_header).and_then(buzz_media::parse_public_ip);
    let port = ip.and(header_value(&cfg.upload_port_header).and_then(buzz_media::parse_port));

    Some(UploadAttribution {
        uploader_name,
        net: UploadNetworkInfo { ip, port },
    })
}

fn serving_write_error(error: anyhow::Error) -> MediaError {
    if buzz_deletion::ServingWriteGuard::acquisition_is_fenced(&error) {
        MediaError::CommunityWriteFenced
    } else {
        MediaError::ServiceUnavailable
    }
}

fn serving_lease_lost(error: anyhow::Error) -> MediaError {
    tracing::warn!(%error, "media serving-write lease lost");
    MediaError::ServiceUnavailable
}

/// PUT `/upload` or the temporary media-only `/media/upload` alias.
///
/// Auth is extracted inside the NIP-FI admission closure so that in active
/// modes a missing or malformed Authorization header produces the contract's
/// NIP-FI denial bytes rather than legacy `MediaError` JSON. [FI-TRACE-AUTHORITY-UNIFORM]
///
/// Expects:
///   - `Authorization: Nostr <base64(kind:24242 event)>` — Blossom auth
///   - `X-SHA-256: <hex>` — Required per BUD-11
///   - `Content-Type` is advisory only; a bounded body prefix selects the
///     streaming video path from actual bytes
///   - Raw binary body (the file bytes)
///
/// Returns a [`BlobDescriptor`] JSON on success.
// TODO(v2): Add persistent per-pubkey storage quotas. Admission limits below
// bound active parser/storage work, but they do not cap durable bytes stored.
// UploadContext is pub(crate) — it's an internal extractor type, never exposed
// outside this crate. The warning is benign: axum resolves it at compile time
// via trait bounds, not by name.
#[allow(private_interfaces)]
#[allow(clippy::result_large_err)] // Response is the natural error type for axum closures
pub async fn upload_blob(
    State(state): State<Arc<AppState>>,
    ctx: UploadContext,
    headers: HeaderMap,
    body: axum::body::Body,
) -> axum::response::Response {
    use crate::nip_fi_http::{admit_nip_fi_http_on_state, Nip98Proof};
    use axum::response::IntoResponse as _;

    // NIP-FI admission with Blossom extraction as the NIP-98 closure.
    // In Enforce mode: extraction failure → NIP-FI denial bytes (MissingEvidence/
    // EvidenceRejected).  In Off mode: MediaError propagates unchanged [FI-INV-15].
    //
    // The closure must verify the auth event against the tenant host BEFORE
    // returning the proven pubkey to the admission gate — same ordering invariant
    // as the read path. [FI-TRACE-AUTHORITY-UNIFORM]
    let tenant_host = ctx.tenant.host().to_owned();
    let headers_clone = headers.clone();
    let admission = match admit_nip_fi_http_on_state(&state, &headers, move || {
        let auth_event = extract_blossom_auth(&headers_clone).map_err(|e| e.into_response())?;
        // Permissive window (3600s): content type unknown until body arrives.
        buzz_media::auth::verify_blossom_auth_event(&auth_event, Some(&tenant_host), 3600)
            .map_err(|e| e.into_response())?;
        let pubkey = auth_event.pubkey;
        Ok(Nip98Proof::new(pubkey, auth_event))
    }) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let auth_event = admission.into_extra();

    // Post-admission: validate X-SHA-256 header and hash binding.
    // These are Blossom-protocol checks, not NIP-FI — Off mode still enforces
    // them because they protect body integrity, not the assertion boundary.
    let claimed_hash = match headers.get("x-sha-256").and_then(|v| v.to_str().ok()) {
        Some(h) => h.to_owned(),
        None => return MediaError::MissingTag("x-sha-256").into_response(),
    };
    if claimed_hash.len() != 64
        || !claimed_hash
            .chars()
            .all(|c| matches!(c, '0'..='9' | 'a'..='f'))
    {
        return MediaError::HashMismatch.into_response();
    }
    let has_matching_x = auth_event
        .tags
        .iter()
        .any(|tag| tag.kind().to_string() == "x" && (tag.content() == Some(&claimed_hash)));
    if !has_matching_x {
        return MediaError::HashMismatch.into_response();
    }

    // Post-admission: relay membership gate (NIP-43).
    let auth_tag = crate::api::relay_members::extract_auth_tag_header(&headers);
    if let Err(e) = crate::api::relay_members::enforce_relay_membership(
        &state,
        ctx.tenant.community(),
        auth_event.pubkey.as_bytes(),
        auth_tag,
        Some(auth_event.created_at.as_secs()),
    )
    .await
    .map(|_| ())
    .map_err(|_| MediaError::RelayMembershipRequired)
    {
        return e.into_response();
    }

    // Post-admission: rate limit and concurrency permit.
    if upload_rate_limited(&state, ctx.tenant.community(), &auth_event.pubkey) {
        metrics::counter!("buzz_media_upload_rejections_total", "reason" => "rate_limit")
            .increment(1);
        return MediaError::UploadRateLimitExceeded.into_response();
    }
    let upload_permit = match acquire_upload_permit(
        &state,
        ctx.tenant.community(),
        &auth_event.pubkey,
    )
    .inspect_err(|_| {
        metrics::counter!("buzz_media_upload_rejections_total", "reason" => "concurrency")
            .increment(1);
    }) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };

    let auth = AuthenticatedUpload {
        auth_event,
        tenant: ctx.tenant,
        route_mode: ctx.route_mode,
        _upload_permit: upload_permit,
    };
    upload_blob_inner(state, auth, headers, body).await
}

async fn upload_blob_inner(
    state: Arc<AppState>,
    auth: AuthenticatedUpload,
    headers: HeaderMap,
    body: axum::body::Body,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    upload_blob_result(state, auth, headers, body)
        .await
        .into_response()
}

async fn upload_blob_result(
    state: Arc<AppState>,
    auth: AuthenticatedUpload,
    headers: HeaderMap,
    body: axum::body::Body,
) -> Result<Json<BlobDescriptor>, MediaError> {
    let attribution = upload_attribution(&state, &auth, &headers).await;

    let serving_write =
        buzz_deletion::acquire_serving_write(&state.db, auth.tenant.community(), "media_upload")
            .await
            .map_err(serving_write_error)?;

    if auth.route_mode == UploadRouteMode::LegacyMedia {
        metrics::counter!("buzz_media_legacy_upload_route_total").increment(1);
    }

    // Probe actual bytes without trusting Content-Type. Keep the chunks used
    // for the bounded probe and replay them into the selected pipeline so the
    // stored/hash-verified body remains byte-identical.
    use futures_util::StreamExt;
    const SNIFF_BYTES: usize = 4096;
    let mut source = body.into_data_stream();
    let mut replay_chunks = Vec::new();
    let mut sniff = Vec::with_capacity(SNIFF_BYTES);
    while sniff.len() < SNIFF_BYTES {
        match source.next().await {
            Some(Ok(chunk)) => {
                let needed = SNIFF_BYTES - sniff.len();
                sniff.extend_from_slice(&chunk[..chunk.len().min(needed)]);
                replay_chunks.push(chunk);
            }
            Some(Err(error)) => return Err(MediaError::Io(error.to_string())),
            None => break,
        }
    }
    let replay = futures_util::stream::iter(replay_chunks.into_iter().map(Ok)).chain(source);

    serving_write.verify().await.map_err(serving_lease_lost)?;

    let mut descriptor = serving_write
        .protect(async {
            Ok(if should_stream_as_video(&sniff) {
                // Video path: stream body directly to disk — never fully buffered in RAM.
                let content_length = headers
                    .get("content-length")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok());
                buzz_media::process_video_upload(
                    &state.media_storage,
                    &state.config.media,
                    &auth.tenant,
                    &auth.auth_event,
                    replay,
                    content_length,
                    attribution,
                )
                .await?
            } else {
                // Non-video path: buffer the body (bounded by the larger of the image
                // and generic-file caps), then decide image-vs-generic by sniffed MIME.
                // Images go through the thumbnailing pipeline; non-media attachments
                // (docs, archives, text, data) take the generic file path and are
                // served as downloads. Recognized audio/video cannot fall through it.
                let max = state
                    .config
                    .media
                    .max_image_bytes
                    .max(state.config.media.max_file_bytes);
                let bytes =
                    axum::body::to_bytes(axum::body::Body::from_stream(replay), max as usize)
                        .await
                        .map_err(|_| MediaError::FileTooLarge { size: 0, max })?;

                let is_image = matches!(
                    infer::get(&bytes).map(|t| t.mime_type()),
                    Some("image/jpeg" | "image/png" | "image/gif" | "image/webp")
                );

                if is_image {
                    buzz_media::process_upload(
                        &state.media_storage,
                        &state.config.media,
                        &auth.tenant,
                        &auth.auth_event,
                        bytes,
                        attribution,
                    )
                    .await?
                } else if auth.route_mode == UploadRouteMode::LegacyMedia {
                    let mime = infer::get(&bytes)
                        .map(|kind| kind.mime_type().to_string())
                        .unwrap_or_else(|| "application/octet-stream".to_string());
                    return Err(MediaError::DisallowedContentType(mime));
                } else {
                    buzz_media::process_file_upload(
                        &state.media_storage,
                        &state.config.media,
                        &auth.tenant,
                        &auth.auth_event,
                        bytes,
                        attribution,
                    )
                    .await?
                }
            })
        })
        .await
        .map_err(|error| {
            if buzz_deletion::ServingWriteGuard::is_lease_lost(&error) {
                serving_lease_lost(error)
            } else {
                match error.downcast::<MediaError>() {
                    Ok(error) => error,
                    Err(_) => MediaError::Internal,
                }
            }
        })??;

    rewrite_descriptor_urls_for_tenant(
        &mut descriptor,
        &state.config.relay_url,
        auth.tenant.host(),
    );

    // Normalize MIME to a known set to bound label cardinality.
    let mime_label = match descriptor.mime_type.as_str() {
        "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "video/mp4" => {
            &descriptor.mime_type
        }
        _ => "other",
    };
    metrics::counter!(
        "buzz_media_uploads_total",
        "mime" => mime_label.to_owned(),
        "community" => auth.tenant.host().to_owned()
    )
    .increment(1);

    // Audit via bounded channel — same pattern as event audit.
    if let Some(audit_tx) = &state.audit_tx {
        let desc = descriptor.clone();
        if let Err(e) = audit_tx
            .send(NewAuditEntry {
                community_id: auth.tenant.community(),
                action: AuditAction::MediaUploaded,
                actor_pubkey: Some(auth.auth_event.pubkey.to_bytes().to_vec()),
                object_id: Some(desc.sha256.clone()),
                detail: serde_json::json!({
                    "sha256": desc.sha256,
                    "size": desc.size,
                    "mime": desc.mime_type,
                }),
            })
            .await
        {
            tracing::error!("Media audit channel closed — entry lost: {e}");
            metrics::counter!("buzz_audit_send_errors_total").increment(1);
        }
    }

    serving_write.finish().await.map_err(serving_lease_lost)?;
    Ok(Json(descriptor))
}

pub(crate) fn media_base_url_for_tenant(config_relay_url: &str, tenant_host: &str) -> String {
    let scheme = if config_relay_url.trim_start().starts_with("wss://")
        || config_relay_url.trim_start().starts_with("https://")
    {
        "https"
    } else {
        "http"
    };
    format!("{scheme}://{tenant_host}/media")
}

fn rewrite_descriptor_urls_for_tenant(
    descriptor: &mut BlobDescriptor,
    config_relay_url: &str,
    tenant_host: &str,
) {
    let base = media_base_url_for_tenant(config_relay_url, tenant_host);
    let ext = descriptor
        .url
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| is_safe_ext(ext))
        .unwrap_or("bin");
    descriptor.url = format!("{base}/{}.{ext}", descriptor.sha256);
    if descriptor.thumb.is_some() {
        descriptor.thumb = Some(format!("{base}/{}.thumb.jpg", descriptor.sha256));
    }
}

async fn bind_media_read_tenant(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TenantContext, MediaError> {
    let raw_host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| MediaError::NotFound)
}

/// Extract and signature-verify the Blossom auth event for a GET/HEAD read.
///
/// This is the NIP-98 extraction step for media reads: it parses the
/// `Authorization: Nostr <base64>` header, decodes and verifies the NIP-98
/// event, and checks the Blossom GET auth binding (sha256 and server tags).
///
/// Used as the NIP-98 closure inside `admit_nip_fi_http_on_state` so that in
/// active NIP-FI modes a missing/malformed Authorization header is mapped to
/// the correct NIP-FI DenialClass instead of a legacy `MediaError` JSON 401.
/// Off mode propagates `MediaError` unchanged ([FI-INV-15]).
///
/// [FI-TRACE-AUTHORITY-UNIFORM]
fn extract_blossom_read_proof(
    headers: &HeaderMap,
    sha256: &str,
    tenant_host: &str,
) -> Result<crate::nip_fi_http::Nip98Proof<nostr::Event>, MediaError> {
    let auth_event = extract_blossom_auth(headers)?;
    buzz_media::auth::verify_blossom_get_auth(&auth_event, sha256, Some(tenant_host), 3600)?;
    let pubkey = auth_event.pubkey;
    Ok(crate::nip_fi_http::Nip98Proof::new(pubkey, auth_event))
}

/// Post-admission membership gate for media reads.
///
/// Called after `admit_nip_fi_http_on_state` succeeds so that membership
/// is checked against the NIP-FI-verified pubkey rather than a raw
/// header value.  Separated from extraction so it can run after admission
/// in both Off and active modes.
async fn enforce_blossom_read_membership(
    state: &AppState,
    tenant: &TenantContext,
    auth_event: &nostr::Event,
    headers: &HeaderMap,
) -> Result<(), MediaError> {
    let auth_tag = crate::api::relay_members::extract_auth_tag_header(headers);
    crate::api::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        auth_event.pubkey.as_bytes(),
        auth_tag,
        Some(auth_event.created_at.as_secs()),
    )
    .await
    .map(|_| ())
    .map_err(|_| MediaError::RelayMembershipRequired)
}

fn blob_cache_control() -> &'static str {
    "private, max-age=31536000, immutable"
}

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

/// CSP for a blob response, kept least-privilege while allowing browser-native
/// viewers to load the response as their generated `<img>` or `<video>` source.
///
/// Chromium applies the blob response's CSP when a user navigates directly to
/// an image or video URL. A bare `default-src 'none'` therefore blocks the
/// browser-generated media element from loading the very response being viewed.
fn blob_content_security_policy(content_type: &str) -> &'static str {
    if content_type.starts_with("image/") {
        "default-src 'none'; img-src 'self'"
    } else if content_type.starts_with("video/") {
        "default-src 'none'; media-src 'self'"
    } else {
        "default-src 'none'"
    }
}

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
#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
pub async fn get_blob(
    State(state): State<Arc<AppState>>,
    Path(sha256_ext): Path<String>,
    req_headers: HeaderMap,
) -> Result<Response, MediaError> {
    validate_media_path(&sha256_ext)?;
    // Row zero: bind tenant. Blossom auth extraction and NIP-FI admission follow
    // so that in Enforce mode a missing/malformed Authorization header produces
    // NIP-FI denial bytes (not legacy MediaError JSON). [FI-TRACE-AUTHORITY-UNIFORM]
    let tenant = bind_media_read_tenant(&state, &req_headers).await?;
    let sha256 = sha256_ext
        .split('.')
        .next()
        .unwrap_or(&sha256_ext)
        .to_owned();
    let tenant_host = tenant.host().to_owned();
    let headers_clone = req_headers.clone();
    // NIP-FI admission with Blossom extraction as the NIP-98 closure.
    // In Enforce mode: extraction failure → NIP-FI denial bytes (MissingEvidence/
    // EvidenceRejected).  In Off mode: MediaError propagates unchanged [FI-INV-15].
    use crate::nip_fi_http::admit_nip_fi_http_on_state;
    let admission = match admit_nip_fi_http_on_state(&state, &req_headers, move || {
        extract_blossom_read_proof(&headers_clone, &sha256, &tenant_host)
            .map_err(|e| e.into_response())
    }) {
        Ok(a) => a,
        Err(resp) => return Ok(resp),
    };
    let auth_event = admission.into_extra();
    // Post-admission: membership gate.
    enforce_blossom_read_membership(&state, &tenant, &auth_event, &req_headers).await?;
    serve_blob_for_tenant(&state, &tenant, &sha256_ext, &req_headers).await
}

/// Serve a validated blob from an already-authorized tenant context.
///
/// This is the common byte-serving mechanism for Blossom reads and narrowly
/// scoped internal readers. Callers must establish their own authorization
/// before entering this function; the tenant is never derived from client input.
pub(crate) async fn serve_blob_for_tenant(
    state: &AppState,
    tenant: &TenantContext,
    sha256_ext: &str,
    req_headers: &HeaderMap,
) -> Result<Response, MediaError> {
    validate_media_path(sha256_ext)?;
    let cache_control = blob_cache_control();

    // Sidecar gate FIRST — reject before any blob I/O. Storage is not authoritative.
    let content_type = if sha256_ext.ends_with(".thumb.jpg") {
        let parent_hash = sha256_ext.strip_suffix(".thumb.jpg").unwrap_or(sha256_ext);
        let _ = state
            .media_storage
            .read_sidecar_mime(tenant, parent_hash)
            .await
            .ok_or(MediaError::NotFound)?;
        "image/jpeg".to_string()
    } else {
        // For explicit paths (hash.ext), verify the requested extension matches
        // the sidecar's canonical extension — sidecar is authoritative.
        let sidecar_mime = state
            .media_storage
            .read_sidecar_mime(tenant, sha256_ext)
            .await
            .ok_or(MediaError::NotFound)?;
        if sha256_ext.contains('.') {
            let requested_ext = sha256_ext.rsplit('.').next().unwrap_or("");
            let sidecar = state
                .media_storage
                .get_sidecar(tenant, sha256_ext.split('.').next().unwrap_or(sha256_ext))
                .await
                .map_err(|_| MediaError::NotFound)?;
            if requested_ext != sidecar.ext {
                return Err(MediaError::NotFound);
            }
        }
        sidecar_mime
    };

    // Images and video render inline; generic files force download. This is the
    // primary defence for non-previewable types — combined with `nosniff` and a
    // restrictive CSP, an attachment disposition prevents an uploaded file from
    // ever executing or rendering as active content in the client. Inline media
    // gets only its own CSP source directive so direct browser navigation works.
    let disposition = if buzz_media::serve_inline(&content_type) {
        "inline"
    } else {
        "attachment"
    };
    let content_security_policy = blob_content_security_policy(&content_type);

    let key = resolve_s3_key(&state.media_storage, tenant, sha256_ext).await?;

    // Parse optional Range header.
    let range_header = req_headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    // Extract single-range value, if present. Multi-range (comma-separated) is
    // unsupported — we ignore it and serve the full body per RFC 9110 §14.2.
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
                .header(header::CACHE_CONTROL, cache_control)
                .header(header::CONTENT_SECURITY_POLICY, content_security_policy)
                .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                .header(header::ACCEPT_RANGES, "bytes")
                .body(axum::body::Body::from_stream(stream))
                .map_err(|_| MediaError::Internal)?;
            Ok(resp)
        }
        Some(range_str) => {
            // S3-native single-range response, capped to bound request memory.
            let total = state
                .media_storage
                .head_with_metadata(&key)
                .await?
                .ok_or(MediaError::NotFound)?
                .size;

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

                    let end = end.min(total.saturating_sub(1));
                    let end = end
                        .min(start.saturating_add(MAX_RANGE_CHUNK - 1))
                        .min(total.saturating_sub(1));
                    let chunk = state.media_storage.get_range(&key, start, end).await?;
                    let content_range = format!("bytes {start}-{end}/{total}");

                    Ok(axum::response::Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::CONTENT_TYPE, &content_type)
                        .header(header::CONTENT_RANGE, content_range)
                        .header(header::CONTENT_LENGTH, chunk.len().to_string())
                        .header(header::CONTENT_DISPOSITION, disposition)
                        .header(header::ACCEPT_RANGES, "bytes")
                        .header(header::CACHE_CONTROL, cache_control)
                        .header(header::CONTENT_SECURITY_POLICY, content_security_policy)
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

/// Passive raster image formats safe to render inline in a browser, keyed by
/// content sniff of the stored bytes. SVG is intentionally excluded: it is an
/// active document that can execute script.
fn verified_inline_image_type(bytes: &[u8]) -> Option<&'static str> {
    match infer::get(bytes).map(|kind| kind.mime_type()) {
        Some("image/png") => Some("image/png"),
        Some("image/jpeg") => Some("image/jpeg"),
        Some("image/gif") => Some("image/gif"),
        Some("image/webp") => Some("image/webp"),
        _ => None,
    }
}

/// The browser-facing response policy for a feedback attachment, derived solely
/// from a content sniff of the stored `prefix` bytes — never the reporter's
/// `imeta` MIME. Returns the served `Content-Type` and `Content-Disposition`:
/// verified passive raster renders `inline` with its sniffed type; every other
/// payload is forced to `application/octet-stream` + `attachment` so the browser
/// downloads it instead of running it. `X-Content-Type-Options: nosniff` is
/// always applied by the caller so a forced attachment can never be sniffed back
/// into an executable type. This is the load-bearing security seam.
fn feedback_attachment_response_policy(prefix: &[u8]) -> (&'static str, &'static str) {
    match verified_inline_image_type(prefix) {
        Some(mime) => (mime, "inline"),
        None => ("application/octet-stream", "attachment"),
    }
}

/// Serve a feedback attachment to an admin operator without ever letting an
/// attacker-controlled payload execute as a typed document.
///
/// Feedback attachment bytes, their `imeta` MIME, and filename are all supplied
/// by untrusted reporters. The normal media route trusts the stored sidecar
/// MIME to choose an inline disposition, so a hash-valid HTML or SVG payload
/// mislabelled `image/*` would open as an executable document on the admin
/// origin. This wrapper re-derives the served type from a content sniff of the
/// stored bytes: only verified passive raster images render inline; every other
/// payload is forced to `application/octet-stream` + `Content-Disposition:
/// attachment` so the browser downloads it instead of running it. The normal
/// `/media` route is unchanged.
pub(crate) async fn serve_feedback_attachment(
    state: &AppState,
    tenant: &TenantContext,
    sha256: &str,
    req_headers: &HeaderMap,
) -> Result<Response, MediaError> {
    // infer needs only the leading magic bytes (webp reads through byte 11).
    const SNIFF_PREFIX_LEN: u64 = 32;
    let key = resolve_s3_key(&state.media_storage, tenant, sha256).await?;
    let prefix = state
        .media_storage
        .get_range(&key, 0, SNIFF_PREFIX_LEN - 1)
        .await
        .unwrap_or_default();
    let (content_type, disposition) = feedback_attachment_response_policy(&prefix);

    let mut response = serve_blob_for_tenant(state, tenant, sha256, req_headers).await?;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static(disposition),
    );
    // A forced attachment must never be sniffed back into an executable type.
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
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
#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
pub async fn head_blob(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(sha256_ext): Path<String>,
) -> Result<Response, MediaError> {
    validate_media_path(&sha256_ext)?;
    // Row zero: bind tenant. Blossom auth extraction and NIP-FI admission follow
    // so that in Enforce mode a missing/malformed Authorization header produces
    // NIP-FI denial bytes (not legacy MediaError JSON). [FI-TRACE-AUTHORITY-UNIFORM]
    let tenant = bind_media_read_tenant(&state, &headers).await?;
    let sha256 = sha256_ext
        .split('.')
        .next()
        .unwrap_or(&sha256_ext)
        .to_owned();
    let tenant_host = tenant.host().to_owned();
    let headers_clone = headers.clone();
    use crate::nip_fi_http::admit_nip_fi_http_on_state;
    let admission = match admit_nip_fi_http_on_state(&state, &headers, move || {
        extract_blossom_read_proof(&headers_clone, &sha256, &tenant_host)
            .map_err(|e| e.into_response())
    }) {
        Ok(a) => a,
        Err(resp) => return Ok(resp),
    };
    let auth_event = admission.into_extra();
    enforce_blossom_read_membership(&state, &tenant, &auth_event, &headers).await?;
    let cache_control = blob_cache_control();

    // Sidecar gate FIRST — reject before any blob I/O.
    let content_type = if sha256_ext.ends_with(".thumb.jpg") {
        let parent_hash = sha256_ext.strip_suffix(".thumb.jpg").unwrap_or(&sha256_ext);
        let _ = state
            .media_storage
            .read_sidecar_mime(&tenant, parent_hash)
            .await
            .ok_or(MediaError::NotFound)?;
        "image/jpeg".to_string()
    } else {
        let sidecar_mime = state
            .media_storage
            .read_sidecar_mime(&tenant, &sha256_ext)
            .await
            .ok_or(MediaError::NotFound)?;
        if sha256_ext.contains('.') {
            let requested_ext = sha256_ext.rsplit('.').next().unwrap_or("");
            let sidecar = state
                .media_storage
                .get_sidecar(&tenant, sha256_ext.split('.').next().unwrap_or(&sha256_ext))
                .await
                .map_err(|_| MediaError::NotFound)?;
            if requested_ext != sidecar.ext {
                return Err(MediaError::NotFound);
            }
        }
        sidecar_mime
    };

    let key = resolve_s3_key(&state.media_storage, &tenant, &sha256_ext).await?;
    match state.media_storage.head_with_metadata(&key).await? {
        Some(meta) => {
            let size_str = meta.size.to_string();
            Ok((
                StatusCode::OK,
                [
                    ("content-type", content_type.as_str()),
                    ("content-length", size_str.as_str()),
                    ("accept-ranges", "bytes"),
                    ("cache-control", cache_control),
                ],
            )
                .into_response())
        }
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

/// Resolve the S3 key from a URL path segment.
///
/// - `sha256.ext`       → used as-is (already validated by `validate_media_path`)
/// - `sha256` (no dot)  → read sidecar to get extension, return `sha256.ext`
///
/// Sidecar-derived extensions are validated as safe tokens to prevent
/// object-key confusion if sidecar data is ever tampered with.
async fn resolve_s3_key(
    storage: &buzz_media::MediaStorage,
    tenant: &TenantContext,
    sha256_ext: &str,
) -> Result<String, MediaError> {
    if sha256_ext.contains('.') {
        Ok(sha256_ext.to_string())
    } else {
        let sidecar = storage
            .get_sidecar(tenant, sha256_ext)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
    };
    use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
    use tower::ServiceExt;
    use uuid::Uuid;

    const VALID_HASH: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    #[test]
    fn serving_write_error_taxonomy_separates_fence_from_backend_failure() {
        let fenced = anyhow::Error::from(buzz_db::DbError::AccessDenied("fenced".to_string()));
        assert!(matches!(
            serving_write_error(fenced),
            MediaError::CommunityWriteFenced
        ));
        let backend = anyhow::Error::from(buzz_db::DbError::Sqlx(sqlx::Error::PoolTimedOut));
        assert!(matches!(
            serving_write_error(backend),
            MediaError::ServiceUnavailable
        ));
    }

    #[test]
    fn feedback_inline_allows_only_sniffed_passive_raster_images() {
        // Real magic bytes for the four verified passive raster formats.
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];
        let gif = *b"GIF89a";
        let mut webp = Vec::from(*b"RIFF");
        webp.extend_from_slice(&[0, 0, 0, 0]);
        webp.extend_from_slice(b"WEBP");
        assert_eq!(verified_inline_image_type(&png), Some("image/png"));
        assert_eq!(verified_inline_image_type(&jpeg), Some("image/jpeg"));
        assert_eq!(verified_inline_image_type(&gif), Some("image/gif"));
        assert_eq!(verified_inline_image_type(&webp), Some("image/webp"));

        // Active documents and non-raster payloads never render inline — a
        // reporter cannot smuggle script past the sniff, regardless of the
        // imeta MIME they supplied.
        assert_eq!(
            verified_inline_image_type(b"<svg xmlns=\"...\"></svg>"),
            None
        );
        assert_eq!(
            verified_inline_image_type(b"<!DOCTYPE html><script>alert(1)</script>"),
            None
        );
        assert_eq!(verified_inline_image_type(b"%PDF-1.7"), None);
        assert_eq!(verified_inline_image_type(b""), None);
    }

    #[test]
    fn feedback_attachment_response_policy_pins_browser_facing_contract() {
        // Verified passive raster is the ONLY payload that serves inline, and it
        // serves as its sniffed type — never a reporter-controlled MIME.
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];
        let gif = *b"GIF89a";
        let mut webp = Vec::from(*b"RIFF");
        webp.extend_from_slice(&[0, 0, 0, 0]);
        webp.extend_from_slice(b"WEBP");
        for (bytes, mime) in [
            (&png[..], "image/png"),
            (&jpeg[..], "image/jpeg"),
            (&gif[..], "image/gif"),
            (&webp[..], "image/webp"),
        ] {
            assert_eq!(
                feedback_attachment_response_policy(bytes),
                (mime, "inline"),
                "verified raster must serve inline as its sniffed type"
            );
        }

        // Every hostile or unrecognized payload is forced to a non-navigable
        // download. This is the seam that keeps a hash-valid HTML/SVG feedback
        // attachment from opening as an executing document on the admin origin.
        for hostile in [
            &b"<!DOCTYPE html><script>alert(1)</script>"[..],
            &b"<svg xmlns=\"...\"><script>alert(1)</script></svg>"[..],
            &b"%PDF-1.7"[..],
            &b""[..],       // failed/empty sniff prefix — fail closed to download
            &b"\x89PN"[..], // short/truncated prefix — not enough to verify
        ] {
            assert_eq!(
                feedback_attachment_response_policy(hostile),
                ("application/octet-stream", "attachment"),
                "hostile/unrecognized bytes must force a download, never inline"
            );
        }
    }

    #[test]
    fn upload_routes_distinguish_standard_and_legacy_modes() {
        assert_eq!(
            upload_route_mode("/upload").expect("standard upload route"),
            UploadRouteMode::Upload
        );
        assert_eq!(
            upload_route_mode("/media/upload").expect("legacy upload route"),
            UploadRouteMode::LegacyMedia
        );
        assert!(matches!(
            upload_route_mode("/media"),
            Err(MediaError::NotFound)
        ));
    }

    #[test]
    fn proprietary_iso_bmff_brand_still_uses_video_pipeline() {
        let bytes = b"\x00\x00\x00\x18ftypPRIV\x00\x00\x00\x00isommp42";
        assert!(infer::get(bytes).is_none());
        assert!(should_stream_as_video(bytes));
    }

    #[test]
    fn inline_media_csp_allows_only_its_browser_native_viewer() {
        assert_eq!(
            blob_content_security_policy("video/mp4"),
            "default-src 'none'; media-src 'self'"
        );
        assert_eq!(
            blob_content_security_policy("image/png"),
            "default-src 'none'; img-src 'self'"
        );
    }

    #[test]
    fn attachment_csp_keeps_all_resource_loading_disabled() {
        assert_eq!(
            blob_content_security_policy("application/pdf"),
            "default-src 'none'"
        );
        assert_eq!(
            blob_content_security_policy("application/octet-stream"),
            "default-src 'none'"
        );
    }

    async fn test_state() -> Arc<AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.media_uploads_per_minute = 1;
        config.media_max_concurrent_uploads = 2;
        config.media_max_concurrent_uploads_per_pubkey = 1;

        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        db.ensure_configured_community("relay.example")
            .await
            .expect("seed relay.example community for host-bound media tests");
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        Arc::new(state)
    }

    async fn media_get_auth_router() -> axum::Router {
        let state = test_state().await;
        axum::Router::new()
            .route(
                "/media/{sha256_ext}",
                axum::routing::get(get_blob).head(head_blob),
            )
            .with_state(state)
    }

    fn media_get_auth_header(keys: &Keys, tags: Vec<Tag>) -> String {
        let event = EventBuilder::new(Kind::from(24242), "Get media")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign get auth");
        format!(
            "Nostr {}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(event.as_json().as_bytes())
        )
    }

    fn media_get_tags_for(host: &str, sha256: Option<&str>) -> Vec<Tag> {
        let now = Timestamp::now().as_secs();
        let expiration = (now + 300).to_string();
        let mut tags = vec![
            Tag::parse(["t", "get"]).expect("t tag"),
            Tag::parse(["expiration", &expiration]).expect("expiration tag"),
            Tag::parse(["server", host]).expect("server tag"),
        ];
        if let Some(sha256) = sha256 {
            tags.push(Tag::parse(["x", sha256]).expect("x tag"));
        }
        tags
    }

    fn media_request(method: &str, auth: Option<String>) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("/media/{VALID_HASH}.jpg"))
            .header(header::HOST, "relay.example");
        if let Some(auth) = auth {
            builder = builder.header(header::AUTHORIZATION, auth);
        }
        builder.body(Body::empty()).expect("request")
    }

    #[tokio::test]
    async fn media_reads_reject_unauthenticated_get_and_head_before_sidecar_gate() {
        for method in ["GET", "HEAD"] {
            let response = media_get_auth_router()
                .await
                .oneshot(media_request(method, None))
                .await
                .expect("response");

            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{method}");
        }
    }

    #[tokio::test]
    async fn media_read_with_valid_server_scoped_token_reaches_sidecar_gate() {
        let keys = Keys::generate();
        let auth = media_get_auth_header(&keys, media_get_tags_for("relay.example", None));
        let response = media_get_auth_router()
            .await
            .oneshot(media_request("GET", Some(auth)))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn media_read_rejects_upload_verb_wrong_server_and_wrong_x() {
        let keys = Keys::generate();
        let now = Timestamp::now().as_secs();
        let expiration = (now + 300).to_string();
        let cases = [
            vec![
                Tag::parse(["t", "upload"]).expect("t tag"),
                Tag::parse(["expiration", &expiration]).expect("expiration tag"),
                Tag::parse(["server", "relay.example"]).expect("server tag"),
            ],
            vec![
                Tag::parse(["t", "get"]).expect("t tag"),
                Tag::parse(["expiration", &expiration]).expect("expiration tag"),
                Tag::parse(["server", "evil.example"]).expect("server tag"),
            ],
            vec![
                Tag::parse(["t", "get"]).expect("t tag"),
                Tag::parse(["expiration", &expiration]).expect("expiration tag"),
                Tag::parse(["x", &"f".repeat(64)]).expect("x tag"),
            ],
        ];

        for tags in cases {
            let auth = media_get_auth_header(&keys, tags);
            let response = media_get_auth_router()
                .await
                .oneshot(media_request("GET", Some(auth)))
                .await
                .expect("response");

            assert_ne!(response.status(), StatusCode::NOT_FOUND);
            assert!(
                response.status() == StatusCode::UNAUTHORIZED
                    || response.status() == StatusCode::FORBIDDEN,
                "unexpected status {}",
                response.status()
            );
        }
    }

    #[tokio::test]
    async fn media_read_accepts_range_header_only_after_auth() {
        let keys = Keys::generate();
        let auth = media_get_auth_header(&keys, media_get_tags_for("relay.example", None));
        let mut request = media_request("GET", Some(auth));
        request
            .headers_mut()
            .insert(header::RANGE, "bytes=0-0".parse().expect("range header"));

        let response = media_get_auth_router()
            .await
            .oneshot(request)
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn upload_rate_limiter_is_scoped_by_community() {
        let state = test_state().await;
        let pubkey = nostr::Keys::generate().public_key();
        let community_a = buzz_core::CommunityId::from_uuid(Uuid::from_u128(0xAAAA));
        let community_b = buzz_core::CommunityId::from_uuid(Uuid::from_u128(0xBBBB));

        assert!(!upload_rate_limited(&state, community_a, &pubkey));
        assert!(upload_rate_limited(&state, community_a, &pubkey));
        assert!(
            !upload_rate_limited(&state, community_b, &pubkey),
            "A's exhausted upload budget must not rate-limit the same key in B"
        );
    }

    #[tokio::test]
    async fn upload_concurrency_limit_is_scoped_by_community() {
        let state = test_state().await;
        let pubkey = nostr::Keys::generate().public_key();
        let community_a = buzz_core::CommunityId::from_uuid(Uuid::from_u128(0xAAAA));
        let community_b = buzz_core::CommunityId::from_uuid(Uuid::from_u128(0xBBBB));

        let permit_a =
            acquire_upload_permit(&state, community_a, &pubkey).expect("first A upload allowed");
        assert!(matches!(
            acquire_upload_permit(&state, community_a, &pubkey),
            Err(MediaError::UploadConcurrencyLimitReached)
        ));
        let permit_b = acquire_upload_permit(&state, community_b, &pubkey)
            .expect("A's in-flight upload must not block B");

        drop(permit_b);
        drop(permit_a);
    }

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

    #[test]
    fn test_validate_media_path_rejects_upload_record_keys() {
        // The `_uploads/` per-event records (and `_meta/` sidecars) must be
        // unreachable through the serve path. Axum's single path segment
        // can't even contain `/`, but assert the validator rejects these
        // shapes outright so the property survives any routing change.
        assert!(validate_media_path("_uploads").is_err());
        assert!(validate_media_path(&format!("_uploads/c/{VALID_HASH}/01J.json")).is_err());
        assert!(validate_media_path("_meta").is_err());
        assert!(validate_media_path(&format!("_meta/c/{VALID_HASH}.json")).is_err());
        // Suffix-style metadata keys are also non-servable (>3 segments / bad ext).
        assert!(validate_media_path(&format!("{VALID_HASH}.png.metadata")).is_err());
    }

    #[test]
    fn media_base_url_for_tenant_uses_tenant_host_and_http_scheme() {
        assert_eq!(
            media_base_url_for_tenant("wss://config.example", "tenant-b.example"),
            "https://tenant-b.example/media"
        );
        assert_eq!(
            media_base_url_for_tenant("ws://config.example", "localhost:3100"),
            "http://localhost:3100/media"
        );
    }

    #[test]
    fn rewrite_descriptor_urls_for_tenant_replaces_global_media_host() {
        let hash = "a".repeat(64);
        let mut descriptor = BlobDescriptor {
            url: format!("https://primary.example/media/{hash}.jpg"),
            sha256: hash.clone(),
            size: 42,
            mime_type: "image/jpeg".to_string(),
            uploaded: 1700000000,
            dim: Some("1x1".to_string()),
            blurhash: None,
            thumb: Some(format!("https://primary.example/media/{hash}.thumb.jpg")),
            duration: None,
        };

        rewrite_descriptor_urls_for_tenant(
            &mut descriptor,
            "wss://primary.example",
            "tenant-b.example",
        );

        assert_eq!(
            descriptor.url,
            format!("https://tenant-b.example/media/{hash}.jpg")
        );
        assert_eq!(
            descriptor.thumb,
            Some(format!("https://tenant-b.example/media/{hash}.thumb.jpg"))
        );
    }

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

    // ── NIP-FI media regression tests ────────────────────────────────────────
    //
    // These postgres-backed tests prove the NIP-FI admission gate is wired into
    // the media upload and read routes at the router level. They all require a
    // live Postgres + Redis and are tagged #[ignore = "requires Postgres"].
    //
    // Verified contract rows (NIP-FI.md rejection table):
    //   Enforce mode, missing Blossom auth:  401 `authentication required\n`
    //                                         text/plain; charset=utf-8
    //                                         WWW-Authenticate: Nostr
    //   Enforce mode, malformed auth:         403 `evidence rejected\n`
    //                                         text/plain; charset=utf-8
    //                                         no challenge
    //   Enforce mode, duplicate auth header:  403 `evidence rejected\n` (cardinality)
    //   Off mode, missing Blossom auth:       401 `{"error":"authentication failed"}`
    //                                         application/json
    //                                         no WWW-Authenticate
    //
    // [FI-TRACE-AUTHORITY-UNIFORM, FI-INV-15]
    #[cfg(test)]
    mod postgres_tests {
        use super::*;
        use std::sync::Arc;

        use axum::body::{to_bytes, Body};
        use axum::http::{Request, StatusCode};
        use buzz_auth::NipFiMode;
        use nostr::{EventBuilder, Keys, Kind, Tag};
        use tower::ServiceExt;

        // ── Test infrastructure ────────────────────────────────────────────────

        /// Always-fresh replay guard (no Redis needed) — same pattern as bridge tests.
        struct AlwaysFreshReplayGuard;
        impl buzz_auth::Nip98ReplayGuard for AlwaysFreshReplayGuard {
            fn try_mark_in_scope<'a>(
                &'a self,
                _scope: &'a str,
                _event_id: &'a nostr::EventId,
                _ttl_secs: u64,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>>
                        + Send
                        + 'a,
                >,
            > {
                Box::pin(async { Ok(true) })
            }
        }

        /// Constants for the static P-256 test key, matching the cardinality test in bridge.rs.
        const TEST_ISSUER: &str = "https://issuer.example";
        const TEST_AUDIENCE: &str = "https://relay.example";
        const TEST_KID: &str = "test-key-1";
        const TEST_EC_PKCS8_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
            MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\n\
            WZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\n\
            zhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n\
            -----END PRIVATE KEY-----\n";

        /// Build an AppState with NIP-FI Enforce and a real injected P-256 verifier.
        async fn media_enforce_test_state() -> Option<Arc<AppState>> {
            use buzz_auth::{
                AssertionKeySet, FederatedAssertionVerifier, FreshnessClass, IssuerPolicy,
                IssuerRegistry, StaticIssuerKeySource, TokenClass, VerifyAssertion,
            };
            use jsonwebtoken::{jwk::JwkSet, Algorithm};

            let mut config = crate::config::Config::from_env().ok()?;
            config.database_url = crate::test_support::database_url();
            config.redis_url =
                std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
            config.relay_url = "wss://nip-fi-media-test.local".to_string();
            config.require_auth_token = false;
            config.require_relay_membership = false;
            config.nip_fi.mode = NipFiMode::Enforce;

            let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
                .await
                .ok()?;
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .ok()?;
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .ok()?,
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;
            let (mut state, _) = crate::state::AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                Keys::generate(),
                media_storage,
            );
            state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);

            // Inject a real P-256 verifier so the assertion guard can forward requests.
            let jwks: JwkSet = serde_json::from_value(serde_json::json!({
                "keys": [{
                    "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256",
                    "kid": TEST_KID,
                    "x": "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI",
                    "y": "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA"
                }]
            }))
            .expect("valid test JWKS");
            let hard_deadline = chrono::Utc::now() + chrono::Duration::seconds(3600);
            let key_set =
                AssertionKeySet::new_for_test(TEST_ISSUER.to_owned(), 1, jwks, hard_deadline)
                    .expect("valid test key set");
            let jwks_contract = buzz_auth::JwksSourceContract::new(
                format!("{TEST_ISSUER}/.well-known/jwks.json"),
                300,
                3600,
            )
            .expect("valid jwks contract");
            let policy = IssuerPolicy::new(
                TEST_ISSUER.to_owned(),
                vec![TEST_AUDIENCE.to_owned()],
                TokenClass::DedicatedNipFi,
                FreshnessClass::OfflineJwt,
                vec![Algorithm::ES256],
                60,
                3600,
                None,
                jwks_contract,
            )
            .expect("valid issuer policy");
            let mut registry = IssuerRegistry::new();
            registry.insert(policy);
            let verifier: Arc<dyn VerifyAssertion> = Arc::new(FederatedAssertionVerifier::new(
                registry,
                StaticIssuerKeySource::new([key_set]),
            ));
            state.nip_fi_verifier = Some(verifier);
            Some(Arc::new(state))
        }

        /// Build an AppState with NIP-FI Off.
        async fn media_off_test_state() -> Option<Arc<AppState>> {
            let mut config = crate::config::Config::from_env().ok()?;
            config.database_url = crate::test_support::database_url();
            config.redis_url =
                std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
            config.relay_url = "wss://nip-fi-media-off.local".to_string();
            config.require_auth_token = false;
            config.require_relay_membership = false;
            config.nip_fi.mode = NipFiMode::Off;

            let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
                .await
                .ok()?;
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .ok()?;
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .ok()?,
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;
            let (mut state, _) = crate::state::AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                Keys::generate(),
                media_storage,
            );
            state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
            Some(Arc::new(state))
        }

        /// Mint a valid Blossom upload auth header value.
        fn blossom_upload_auth_value(keys: &Keys, host: &str, sha256_hex: &str) -> String {
            use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
            use nostr::JsonUtil as _;
            let now = nostr::Timestamp::now().as_secs();
            let exp = now + 300;
            let event = EventBuilder::new(Kind::from(24242), "Upload blob")
                .tags(vec![
                    Tag::parse(["t", "upload"]).unwrap(),
                    Tag::parse(["expiration", &exp.to_string()]).unwrap(),
                    Tag::parse(["server", host]).unwrap(),
                    Tag::parse(["x", sha256_hex]).unwrap(),
                ])
                .sign_with_keys(keys)
                .expect("sign blossom upload auth");
            format!("Nostr {}", B64.encode(event.as_json().as_bytes()))
        }

        /// Mint a valid Blossom get auth header value.
        fn blossom_get_auth_value(keys: &Keys, host: &str, sha256_hex: &str) -> String {
            use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
            use nostr::JsonUtil as _;
            let now = nostr::Timestamp::now().as_secs();
            let exp = now + 300;
            let event = EventBuilder::new(Kind::from(24242), "Get blob")
                .tags(vec![
                    Tag::parse(["t", "get"]).unwrap(),
                    Tag::parse(["expiration", &exp.to_string()]).unwrap(),
                    Tag::parse(["server", host]).unwrap(),
                    Tag::parse(["x", sha256_hex]).unwrap(),
                ])
                .sign_with_keys(keys)
                .expect("sign blossom get auth");
            format!("Nostr {}", B64.encode(event.as_json().as_bytes()))
        }

        /// Mint a signed NIP-FI assertion whose `nostr_pubkey` matches `keys`.
        fn signed_assertion(nostr_pubkey_hex: &str) -> String {
            use jsonwebtoken::{Algorithm, EncodingKey, Header};
            let now = chrono::Utc::now().timestamp();
            let claims = serde_json::json!({
                "iss": TEST_ISSUER,
                "aud": TEST_AUDIENCE,
                "iat": now,
                "exp": now + 600,
                "sub": "test-subject",
                "nostr_pubkey": nostr_pubkey_hex,
            });
            let mut header = Header::new(Algorithm::ES256);
            header.kid = Some(TEST_KID.to_owned());
            header.typ = Some("nip-fi+jwt".to_owned());
            let key =
                EncodingKey::from_ec_pem(TEST_EC_PKCS8_PEM.as_bytes()).expect("valid test EC PEM");
            jsonwebtoken::encode(&header, &claims, &key).expect("sign assertion")
        }

        /// Drive a oneshot request through the full relay router; return
        /// `(status, resp_headers, body_bytes)`.
        async fn media_oneshot(
            state: Arc<AppState>,
            method: &str,
            uri: &str,
            host: &str,
            headers: axum::http::HeaderMap,
            body: &[u8],
        ) -> (StatusCode, axum::http::HeaderMap, bytes::Bytes) {
            let mut builder = Request::builder()
                .method(method)
                .uri(uri)
                .header("host", host);
            for (name, value) in &headers {
                builder = builder.header(name, value);
            }
            let resp = crate::router::build_router(state)
                .oneshot(
                    builder
                        .body(Body::from(body.to_vec()))
                        .expect("build request"),
                )
                .await
                .expect("router oneshot");
            let status = resp.status();
            let resp_headers = resp.headers().clone();
            let resp_body = to_bytes(resp.into_body(), 8192).await.unwrap_or_default();
            (status, resp_headers, resp_body)
        }

        /// Upload body that both upload routes reject deterministically before
        /// any storage call: the ID3 magic sniffs as `audio/mpeg`, which
        /// `validate_file_content` refuses on `/upload` and `upload_blob_result`
        /// refuses as `DisallowedContentType` on `/media/upload`.  Both map to
        /// 415 JSON (`buzz-media/src/error.rs`), after admission, the serving
        /// lease, and the body read, but before any storage call.
        const AUDIO_BODY: &[u8] = b"ID3\x04\x00\x00\x00\x00\x00\x00";
        const AUDIO_REJECTION: &[u8] = br#"{"error":"disallowed content type: audio/mpeg"}"#;

        fn sha256_hex(bytes: &[u8]) -> String {
            use sha2::Digest as _;
            hex::encode(sha2::Sha256::digest(bytes))
        }

        /// Assert an exact response: status, Content-Type, challenge, and body.
        fn assert_exact_response(
            (status, headers, body): &(StatusCode, axum::http::HeaderMap, bytes::Bytes),
            expected_status: StatusCode,
            expected_content_type: &str,
            expected_challenge: Option<&str>,
            expected_body: &[u8],
            context: &str,
        ) {
            assert_eq!(*status, expected_status, "{context}: status; body {body:?}");
            assert_eq!(
                headers.get("content-type").and_then(|v| v.to_str().ok()),
                Some(expected_content_type),
                "{context}: Content-Type"
            );
            assert_eq!(
                headers
                    .get("www-authenticate")
                    .and_then(|v| v.to_str().ok()),
                expected_challenge,
                "{context}: WWW-Authenticate"
            );
            assert_eq!(body.as_ref(), expected_body, "{context}: body");
        }

        // ── Upload proof matrix: Enforce mode, both upload routes ───────────
        //
        // `/upload` and the legacy `/media/upload` alias both route to
        // `upload_blob`.  Every Enforce case carries a valid assertion, so the
        // outer guard forwards the request and the handler's
        // `admit_nip_fi_http_on_state` produces the denial.  Removing the
        // handler admission (or routing an alias around it) lets the legacy
        // Blossom extractor answer with `{"error":"authentication failed"}`
        // JSON instead of these NIP-FI text/plain bytes.
        const UPLOAD_ROUTES: [&str; 2] = ["/upload", "/media/upload"];

        fn upload_request(
            rt: &tokio::runtime::Runtime,
            state: &Arc<AppState>,
            route: &str,
            host: &str,
            authorization: &[&str],
            assertion: Option<&str>,
        ) -> (StatusCode, axum::http::HeaderMap, bytes::Bytes) {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(
                "x-sha-256",
                sha256_hex(AUDIO_BODY).parse().expect("valid header"),
            );
            for value in authorization {
                headers.append(
                    axum::http::header::AUTHORIZATION,
                    value.parse().expect("valid header bytes"),
                );
            }
            if let Some(assertion) = assertion {
                headers.insert(
                    buzz_auth::CLIENT_ATTACHED_HEADER,
                    format!("Bearer {assertion}").parse().expect("valid header"),
                );
            }
            rt.block_on(media_oneshot(
                Arc::clone(state),
                "PUT",
                route,
                host,
                headers,
                AUDIO_BODY,
            ))
        }

        fn media_fixture(
            state_fn: impl std::future::Future<Output = Option<Arc<AppState>>>,
        ) -> (tokio::runtime::Runtime, Arc<AppState>, String) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            let Some(state) = rt.block_on(state_fn) else {
                panic!("local Postgres not reachable");
            };
            let host = format!("nip-fi-media-up-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");
            (rt, state, host)
        }

        /// Valid assertion, no Authorization → 401 MissingEvidence with challenge.
        #[test]
        #[ignore = "requires Postgres"]
        fn upload_enforce_missing_proof_is_401_nip_fi() {
            let (rt, state, host) = media_fixture(media_enforce_test_state());
            let assertion = signed_assertion(&Keys::generate().public_key().to_hex());
            for route in UPLOAD_ROUTES {
                assert_exact_response(
                    &upload_request(&rt, &state, route, &host, &[], Some(&assertion)),
                    StatusCode::UNAUTHORIZED,
                    "text/plain; charset=utf-8",
                    Some("Nostr"),
                    b"authentication required\n",
                    &format!("{route}: missing proof"),
                );
            }
        }

        /// Valid assertion, malformed Authorization → 403 EvidenceRejected.
        #[test]
        #[ignore = "requires Postgres"]
        fn upload_enforce_malformed_proof_is_403_nip_fi() {
            let (rt, state, host) = media_fixture(media_enforce_test_state());
            let assertion = signed_assertion(&Keys::generate().public_key().to_hex());
            for route in UPLOAD_ROUTES {
                assert_exact_response(
                    &upload_request(
                        &rt,
                        &state,
                        route,
                        &host,
                        &["Nostr !!!not-valid-base64!!!"],
                        Some(&assertion),
                    ),
                    StatusCode::FORBIDDEN,
                    "text/plain; charset=utf-8",
                    None,
                    b"evidence rejected\n",
                    &format!("{route}: malformed proof"),
                );
            }
        }

        /// Valid same-key assertion, the same valid proof twice → 403 from the
        /// cardinality gate.  Removing the gate admits the first proof and the
        /// request reaches the exact 415 of the same-key control below.
        #[test]
        #[ignore = "requires Postgres"]
        fn upload_enforce_duplicate_proof_is_403_cardinality() {
            let (rt, state, host) = media_fixture(media_enforce_test_state());
            let keys = Keys::generate();
            let assertion = signed_assertion(&keys.public_key().to_hex());
            let proof = blossom_upload_auth_value(&keys, &host, &sha256_hex(AUDIO_BODY));
            for route in UPLOAD_ROUTES {
                assert_exact_response(
                    &upload_request(
                        &rt,
                        &state,
                        route,
                        &host,
                        &[&proof, &proof],
                        Some(&assertion),
                    ),
                    StatusCode::FORBIDDEN,
                    "text/plain; charset=utf-8",
                    None,
                    b"evidence rejected\n",
                    &format!("{route}: duplicate proof"),
                );
            }
        }

        /// Valid assertion for `key_a`, valid proof signed by `key_b` → the
        /// handler's key-pairing check denies 403 `authorization denied\n`.
        /// Removing key pairing lets the request reach the exact 415 below.
        #[test]
        #[ignore = "requires Postgres"]
        fn upload_enforce_mismatched_key_is_handler_403() {
            let (rt, state, host) = media_fixture(media_enforce_test_state());
            let assertion = signed_assertion(&Keys::generate().public_key().to_hex());
            let proof =
                blossom_upload_auth_value(&Keys::generate(), &host, &sha256_hex(AUDIO_BODY));
            for route in UPLOAD_ROUTES {
                assert_exact_response(
                    &upload_request(&rt, &state, route, &host, &[&proof], Some(&assertion)),
                    StatusCode::FORBIDDEN,
                    "text/plain; charset=utf-8",
                    None,
                    b"authorization denied\n",
                    &format!("{route}: mismatched key"),
                );
            }
        }

        /// Same-key assertion + proof → admission passes and the request reaches
        /// content validation: exact 415 `AUDIO_REJECTION` on both routes, with
        /// no dependence on storage.
        #[test]
        #[ignore = "requires Postgres"]
        fn upload_enforce_same_key_admission_reaches_content_validation() {
            let (rt, state, host) = media_fixture(media_enforce_test_state());
            let keys = Keys::generate();
            let assertion = signed_assertion(&keys.public_key().to_hex());
            let proof = blossom_upload_auth_value(&keys, &host, &sha256_hex(AUDIO_BODY));
            for route in UPLOAD_ROUTES {
                assert_exact_response(
                    &upload_request(&rt, &state, route, &host, &[&proof], Some(&assertion)),
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "application/json",
                    None,
                    AUDIO_REJECTION,
                    &format!("{route}: same-key admission"),
                );
            }
        }

        /// Off mode skips the cardinality gate and `HeaderMap::get` takes the
        /// first Authorization value.  A single valid proof and a
        /// valid-first/malformed-second pair both reach the exact 415; applying
        /// cardinality in Off mode would return 403, and last-value selection
        /// would return the legacy 401 JSON.
        #[test]
        #[ignore = "requires Postgres"]
        fn upload_off_duplicate_auth_takes_first_value() {
            let (rt, state, host) = media_fixture(media_off_test_state());
            let proof =
                blossom_upload_auth_value(&Keys::generate(), &host, &sha256_hex(AUDIO_BODY));
            for (authorization, context) in [
                (vec![proof.as_str()], "single proof"),
                (
                    vec![proof.as_str(), "Nostr !!!not-valid-base64!!!"],
                    "valid-first/malformed-second",
                ),
            ] {
                assert_exact_response(
                    &upload_request(&rt, &state, "/upload", &host, &authorization, None),
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "application/json",
                    None,
                    AUDIO_REJECTION,
                    &format!("Off {context}"),
                );
            }
        }

        // ── Upload: Off mode, missing Blossom auth → legacy MediaError JSON ──

        /// Off mode + PUT /upload with no Authorization header must return the
        /// legacy MediaError JSON 401: `{"error":"authentication failed"}`,
        /// application/json, and NO `WWW-Authenticate` header.
        ///
        /// FI-INV-15: Off mode must propagate legacy MediaError responses
        /// unchanged.  The NIP-FI denial bytes (text/plain + challenge) MUST NOT
        /// appear in Off mode.
        ///
        /// Falsifying mutation: change Off mode to run NIP-FI admission on
        /// missing-auth → 401 `authentication required\n` text/plain with
        /// WWW-Authenticate → body and content-type assertions fire.
        #[test]
        #[ignore = "requires Postgres"]
        fn upload_off_missing_auth_is_legacy_json_401() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_off_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!("nip-fi-media-off-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");

            let sha256 = "d".repeat(64);
            let mut headers = axum::http::HeaderMap::new();
            headers.insert("x-sha-256", sha256.parse().expect("valid header"));
            // No Authorization header.

            let (status, resp_headers, body) = rt.block_on(media_oneshot(
                Arc::clone(&state),
                "PUT",
                "/upload",
                &host,
                headers,
                b"",
            ));

            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "Off mode + missing Blossom auth MUST return 401 legacy MediaError (FI-INV-15)."
            );
            assert_eq!(
                body.as_ref(),
                br#"{"error":"authentication failed"}"#,
                "Off mode 401 body MUST be exact legacy JSON bytes \
                 '{{\"error\":\"authentication failed\"}}' [FI-INV-15]. \
                 NIP-FI text/plain bytes would indicate Off mode is incorrectly applying \
                 active-mode denial."
            );
            assert_eq!(
                resp_headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                "application/json",
                "Off mode 401 Content-Type MUST be application/json (legacy MediaError) [FI-INV-15]."
            );
            assert!(
                resp_headers.get("www-authenticate").is_none(),
                "Off mode 401 MUST NOT carry WWW-Authenticate [FI-INV-15]."
            );
        }

        // ── GET /media: Enforce mode, missing Blossom auth → 401 NIP-FI ─────

        /// Enforce mode + GET /media/{sha256} with no Authorization header must
        /// return 401 `authentication required\n` + `WWW-Authenticate: Nostr`.
        ///
        /// Falsifying mutation: remove `admit_nip_fi_http_on_state` from
        /// `get_blob` → the legacy Blossom extractor fires → 401 JSON body, no
        /// challenge → body and header assertions fire.
        #[test]
        #[ignore = "requires Postgres"]
        fn get_blob_enforce_missing_proof_is_401_nip_fi() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_enforce_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!("nip-fi-media-enf-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");

            let keys = Keys::generate();
            let assertion = signed_assertion(&keys.public_key().to_hex());
            let sha256 = "e".repeat(64);
            let path = format!("/media/{sha256}.jpg");

            // Assertion present but NO Blossom Authorization header.
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(
                buzz_auth::CLIENT_ATTACHED_HEADER,
                format!("Bearer {assertion}").parse().expect("valid header"),
            );

            let (status, resp_headers, body) = rt.block_on(media_oneshot(
                Arc::clone(&state),
                "GET",
                &path,
                &host,
                headers,
                b"",
            ));

            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "Enforce mode + GET /media + missing Blossom auth MUST return 401 MissingEvidence. \
                 Falsifying mutation: remove admit_nip_fi_http_on_state from get_blob → \
                 legacy Blossom extractor fires → JSON body, no challenge."
            );
            assert_eq!(
                body.as_ref(),
                b"authentication required\n",
                "GET Enforce 401 body MUST be NIP-FI bytes 'authentication required\\n'."
            );
            assert_eq!(
                resp_headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                "text/plain; charset=utf-8",
                "GET Enforce 401 Content-Type MUST be text/plain; charset=utf-8."
            );
            assert_eq!(
                resp_headers
                    .get("www-authenticate")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                "Nostr",
                "GET Enforce 401 MUST carry WWW-Authenticate: Nostr."
            );
        }

        // ── GET /media: Off mode, missing Blossom auth → legacy JSON 401 ────

        /// Off mode + GET /media/{sha256} with no Authorization header must
        /// return the legacy MediaError JSON 401 (FI-INV-15).
        #[test]
        #[ignore = "requires Postgres"]
        fn get_blob_off_missing_auth_is_legacy_json_401() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_off_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!("nip-fi-media-off-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");

            let sha256 = "f".repeat(64);
            let path = format!("/media/{sha256}.jpg");

            let (status, resp_headers, body) = rt.block_on(media_oneshot(
                Arc::clone(&state),
                "GET",
                &path,
                &host,
                Default::default(),
                b"",
            ));

            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "Off GET missing auth → 401"
            );
            assert_eq!(
                body.as_ref(),
                br#"{"error":"authentication failed"}"#,
                "Off GET 401 body MUST be legacy JSON bytes [FI-INV-15]."
            );
            assert_eq!(
                resp_headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                "application/json",
                "Off GET 401 Content-Type MUST be application/json (legacy MediaError) [FI-INV-15]."
            );
            assert!(
                resp_headers.get("www-authenticate").is_none(),
                "Off GET 401 MUST NOT carry WWW-Authenticate [FI-INV-15]."
            );
        }

        // ── PUT /upload: Off mode, malformed Authorization → legacy JSON 401 ─
        //
        // Off mode does NOT apply NIP-FI cardinality or assertion checks.  A
        // malformed Nostr token still fails Blossom extraction and the legacy
        // MediaError response (application/json 401) is returned unchanged.
        //
        // This proves Off mode propagates Blossom errors as legacy JSON — not
        // the NIP-FI text/plain denial bytes that active modes would produce.
        //
        // Falsifying mutation A: map Blossom errors to NIP-FI denial bytes in
        //   Off mode → body changes to "evidence rejected\n" → assertion fires.
        // Falsifying mutation B: swap Off → Enforce → cardinality/assertion gates
        //   fire → NIP-FI 403 body → assertion fires.
        #[test]
        #[ignore = "requires Postgres"]
        fn upload_off_malformed_auth_is_legacy_json_401() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_off_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!("nip-fi-media-off-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");

            let sha256 = "a".repeat(64);
            let mut headers = axum::http::HeaderMap::new();
            headers.insert("x-sha-256", sha256.parse().expect("valid header"));
            headers.insert(
                axum::http::header::AUTHORIZATION,
                "Nostr !!!malformed!!!".parse().expect("valid header bytes"),
            );

            let (status, resp_headers, body) = rt.block_on(media_oneshot(
                Arc::clone(&state),
                "PUT",
                "/upload",
                &host,
                headers,
                b"",
            ));

            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "Off mode PUT /upload + malformed Authorization MUST return 401 \
                 legacy MediaError (FI-INV-15). \
                 Falsifying mutation: map Blossom errors to NIP-FI bytes in Off mode \
                 → NIP-FI body/CT."
            );
            assert_eq!(
                body.as_ref(),
                br#"{"error":"authentication failed"}"#,
                "Off mode malformed-auth 401 body MUST be exact legacy JSON bytes \
                 (not NIP-FI text/plain). [FI-INV-15]"
            );
            assert_eq!(
                resp_headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                "application/json",
                "Off mode malformed-auth 401 Content-Type MUST be application/json \
                 (legacy MediaError). [FI-INV-15]"
            );
            assert!(
                resp_headers.get("www-authenticate").is_none(),
                "Off mode malformed-auth 401 MUST NOT carry WWW-Authenticate. [FI-INV-15]"
            );
        }

        // ── GET /media: Off mode, malformed Authorization → legacy JSON 401 ───
        //
        // Mirror of the upload Off+malformed case for the GET path.
        //
        // Falsifying mutation: map Blossom errors to NIP-FI bytes in Off mode
        // on GET → NIP-FI body/CT → assertion fires.
        #[test]
        #[ignore = "requires Postgres"]
        fn get_blob_off_malformed_auth_is_legacy_json_401() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_off_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!("nip-fi-media-off-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");

            let sha256 = "c".repeat(64);
            let path = format!("/media/{sha256}.jpg");

            let mut headers = axum::http::HeaderMap::new();
            headers.insert(
                axum::http::header::AUTHORIZATION,
                "Nostr !!!malformed!!!".parse().expect("valid header bytes"),
            );

            let (status, resp_headers, body) = rt.block_on(media_oneshot(
                Arc::clone(&state),
                "GET",
                &path,
                &host,
                headers,
                b"",
            ));

            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "Off mode GET /media + malformed Authorization MUST return 401 \
                 legacy MediaError (FI-INV-15). \
                 Falsifying mutation: remap Blossom error to NIP-FI bytes in Off mode."
            );
            assert_eq!(
                body.as_ref(),
                br#"{"error":"authentication failed"}"#,
                "Off GET malformed-auth 401 body MUST be exact legacy JSON bytes \
                 (not NIP-FI text/plain). [FI-INV-15]"
            );
            assert_eq!(
                resp_headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                "application/json",
                "Off GET malformed-auth 401 Content-Type MUST be application/json. [FI-INV-15]"
            );
            assert!(
                resp_headers.get("www-authenticate").is_none(),
                "Off GET malformed-auth 401 MUST NOT carry WWW-Authenticate. [FI-INV-15]"
            );
        }

        // ── GET/HEAD /media: Off mode, legacy bytes on both read methods ──────
        //
        // Off mode skips the cardinality gate and `HeaderMap::get` takes the
        // first Authorization value, so a single valid get proof and a
        // valid-first/malformed-second pair both pass the legacy extractor and
        // reach `serve_blob_for_tenant` / `head_blob`, whose sidecar gate
        // returns `MediaError::NotFound` for the unstored blob → 404
        // `{"error":"not found"}` JSON (`buzz-media/src/error.rs`).  With no
        // Authorization at all the legacy extractor returns 401
        // `{"error":"authentication failed"}` JSON.  HEAD carries the same
        // status and headers with the body stripped by axum.
        //
        // Applying cardinality in Off mode would return NIP-FI 403 text/plain;
        // last-value selection would return the legacy 401 for the duplicate.
        fn media_read_off(
            rt: &tokio::runtime::Runtime,
            state: &Arc<AppState>,
            method: &str,
            path: &str,
            host: &str,
            authorization: &[&str],
        ) -> (StatusCode, axum::http::HeaderMap, bytes::Bytes) {
            let mut headers = axum::http::HeaderMap::new();
            for value in authorization {
                headers.append(
                    axum::http::header::AUTHORIZATION,
                    value.parse().expect("valid header bytes"),
                );
            }
            rt.block_on(media_oneshot(
                Arc::clone(state),
                method,
                path,
                host,
                headers,
                b"",
            ))
        }

        #[test]
        #[ignore = "requires Postgres"]
        fn media_read_off_duplicate_auth_takes_first_value() {
            let (rt, state, host) = media_fixture(media_off_test_state());
            let sha256 = "d0".repeat(32);
            let path = format!("/media/{sha256}.jpg");
            let proof = blossom_get_auth_value(&Keys::generate(), &host, &sha256);
            for (method, body) in [
                ("GET", br#"{"error":"not found"}"#.as_slice()),
                ("HEAD", b"".as_slice()),
            ] {
                for (authorization, context) in [
                    (vec![proof.as_str()], "single proof"),
                    (
                        vec![proof.as_str(), "Nostr !!!not-valid-base64!!!"],
                        "valid-first/malformed-second",
                    ),
                ] {
                    assert_exact_response(
                        &media_read_off(&rt, &state, method, &path, &host, &authorization),
                        StatusCode::NOT_FOUND,
                        "application/json",
                        None,
                        body,
                        &format!("Off {method} {context}"),
                    );
                }
            }
        }

        /// Off HEAD witness: missing Authorization → the legacy 401 JSON
        /// headers, no challenge, body stripped.  Running NIP-FI denial in Off
        /// mode would add `WWW-Authenticate: Nostr` and text/plain.
        #[test]
        #[ignore = "requires Postgres"]
        fn head_blob_off_missing_auth_is_legacy_json_401() {
            let (rt, state, host) = media_fixture(media_off_test_state());
            let path = format!("/media/{}.jpg", "f".repeat(64));
            assert_exact_response(
                &media_read_off(&rt, &state, "HEAD", &path, &host, &[]),
                StatusCode::UNAUTHORIZED,
                "application/json",
                None,
                b"",
                "Off HEAD missing auth",
            );
        }

        // ── HEAD /media: Enforce mode, missing Blossom auth → 401 NIP-FI ────

        /// Enforce mode + HEAD /media/{sha256} with no Authorization header must
        /// return 401 + `WWW-Authenticate: Nostr`.  HEAD suppresses the body per
        /// RFC 9110; we check status and headers only.
        ///
        /// Falsifying mutation: remove `admit_nip_fi_http_on_state` from
        /// `head_blob` → legacy path fires → 401 but no challenge → assertion fires.
        #[test]
        #[ignore = "requires Postgres"]
        fn head_blob_enforce_missing_proof_is_401_nip_fi() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_enforce_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!("nip-fi-media-enf-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");

            let keys = Keys::generate();
            let assertion = signed_assertion(&keys.public_key().to_hex());
            let sha256 = "e".repeat(64);
            let path = format!("/media/{sha256}.jpg");

            let mut headers = axum::http::HeaderMap::new();
            headers.insert(
                buzz_auth::CLIENT_ATTACHED_HEADER,
                format!("Bearer {assertion}").parse().expect("valid header"),
            );

            let (status, resp_headers, head_body) = rt.block_on(media_oneshot(
                Arc::clone(&state),
                "HEAD",
                &path,
                &host,
                headers,
                b"",
            ));

            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "Enforce mode + HEAD /media + missing Blossom auth MUST return 401. \
                 Falsifying mutation: remove admit_nip_fi_http_on_state from head_blob → \
                 legacy path fires → no WWW-Authenticate."
            );
            assert!(
                head_body.is_empty(),
                "HEAD MUST suppress the response body (RFC 9110 §9.3.2). \
                 Got {} bytes: {:?}",
                head_body.len(),
                &head_body.as_ref()[..head_body.len().min(64)]
            );
            assert_eq!(
                resp_headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                "text/plain; charset=utf-8",
                "HEAD Enforce 401 Content-Type MUST be text/plain; charset=utf-8."
            );
            assert_eq!(
                resp_headers
                    .get("www-authenticate")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                "Nostr",
                "HEAD Enforce 401 MUST carry WWW-Authenticate: Nostr."
            );
        }

        // ── GET /media: Enforce mode, valid Blossom but NO assertion → 401 ──

        /// Enforce mode + GET /media/{sha256} with a valid Blossom auth but NO
        /// `Nostr-Federated-Identity` header must return 401 `authentication
        /// required\n` — both the outer guard (`router.rs`) and the per-handler
        /// `admit_nip_fi_http_on_state` in `get_blob()` deny at the same point.
        ///
        /// This is distinct from the "missing Blossom" case: here the Blossom
        /// proof IS present, but no assertion was supplied.  Either the outer
        /// guard or the handler-level check produces MissingEvidence → 401.
        ///
        /// Falsifying mutation: remove BOTH the outer guard and the handler-level
        /// `admit_nip_fi_http_on_state` from `get_blob` → valid Blossom accepted
        /// → proceeds to membership/storage → different status → assertion fires.
        /// Removing only one layer is insufficient: the other still denies 401.
        #[test]
        #[ignore = "requires Postgres"]
        fn get_blob_enforce_valid_blossom_no_assertion_is_401() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_enforce_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!("nip-fi-media-enf-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");

            let keys = Keys::generate();
            let sha256 = "a".repeat(64);
            let path = format!("/media/{sha256}.jpg");
            let auth_val = blossom_get_auth_value(&keys, &host, &sha256);

            // Valid Blossom auth but NO Nostr-Federated-Identity header.
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(
                axum::http::header::AUTHORIZATION,
                auth_val.parse().expect("valid header"),
            );

            let (status, resp_headers, body) = rt.block_on(media_oneshot(
                Arc::clone(&state),
                "GET",
                &path,
                &host,
                headers,
                b"",
            ));

            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "Enforce mode + valid Blossom + NO NIP-FI assertion MUST return 401. \
                 Falsifying mutation: remove assertion guard from get_blob → valid Blossom \
                 accepted → proceeds to membership/storage → 404 or membership 403."
            );
            assert_eq!(
                body.as_ref(),
                b"authentication required\n",
                "GET Enforce 401 (no assertion) body MUST be NIP-FI bytes."
            );
            assert_eq!(
                resp_headers
                    .get("www-authenticate")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                "Nostr",
                "GET Enforce 401 (no assertion) MUST carry WWW-Authenticate: Nostr."
            );
        }

        // ── GET /media: Enforce mode, malformed proof → 403 EvidenceRejected ─
        // Authorization header triggers EvidenceRejected before the NIP-FI
        // assertion check.  403 + exact body + CT + no challenge.
        //
        // The fixture supplies a valid assertion so the outer guard forwards
        // the request and the handler's Blossom extraction failure is mapped
        // to EvidenceRejected.  Removing handler admission from `get_blob`
        // lets the legacy extractor answer with 401 `{"error":"authentication
        // failed"}` JSON instead.
        #[test]
        #[ignore = "requires Postgres"]
        fn get_blob_enforce_malformed_proof_is_403_nip_fi() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_enforce_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!("nip-fi-media-enf-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");

            let sha256 = "c".repeat(64);
            let path = format!("/media/{sha256}.jpg");

            // Malformed: valid Nostr scheme prefix, invalid base64 payload.
            // "!!!" is not valid base64 and decodes to an error in the verifier.
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(
                axum::http::header::AUTHORIZATION,
                "Nostr !!!bad!!!".parse().expect("valid header bytes"),
            );
            headers.insert(
                buzz_auth::CLIENT_ATTACHED_HEADER,
                format!(
                    "Bearer {}",
                    signed_assertion(&Keys::generate().public_key().to_hex())
                )
                .parse()
                .expect("valid header"),
            );

            let (status, resp_headers, body) = rt.block_on(media_oneshot(
                Arc::clone(&state),
                "GET",
                &path,
                &host,
                headers,
                b"",
            ));

            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "GET Enforce + malformed Nostr token MUST return 403 EvidenceRejected. \
                 Removing handler admission → legacy 401 JSON instead."
            );
            assert_eq!(
                body.as_ref(),
                b"evidence rejected\n",
                "GET Enforce malformed 403 body MUST be exact 'evidence rejected\\n'. \
                 [FI-TRACE-DENIAL-ORACLE]"
            );
            assert_eq!(
                resp_headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                "text/plain; charset=utf-8",
                "GET Enforce malformed 403 Content-Type MUST be text/plain; charset=utf-8."
            );
            assert!(
                resp_headers.get("www-authenticate").is_none(),
                "GET Enforce malformed 403 MUST NOT carry WWW-Authenticate. \
                 [FI-TRACE-DENIAL-ORACLE]"
            );
        }

        // ── GET /media: Enforce mode, duplicate Authorization → 403 cardinality ─
        //
        // Two identical Blossom Authorization headers in Enforce mode trigger
        // the cardinality gate inside `admit_nip_fi_http_on_state`.
        // 403 + exact body + CT + no challenge.
        //
        // The fixture supplies a same-key valid assertion so cardinality is the
        // only denial source.  Removing the cardinality gate admits the first
        // proof and the request reaches the sidecar gate → 404 NotFound (the
        // same-key control below), not 403 EvidenceRejected.
        #[test]
        #[ignore = "requires Postgres"]
        fn get_blob_enforce_duplicate_proof_is_403_cardinality() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_enforce_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!("nip-fi-media-enf-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");

            let sha256 = "e".repeat(64);
            let path = format!("/media/{sha256}.jpg");
            let keys = Keys::generate();
            let blossom_val = blossom_get_auth_value(&keys, &host, &sha256);
            let assertion = signed_assertion(&keys.public_key().to_hex());

            let mut headers = axum::http::HeaderMap::new();
            // Two identical Blossom Authorization headers → cardinality 2.
            headers.append(
                axum::http::header::AUTHORIZATION,
                blossom_val.parse().expect("valid header"),
            );
            headers.append(
                axum::http::header::AUTHORIZATION,
                blossom_val.parse().expect("valid header"),
            );
            headers.insert(
                buzz_auth::CLIENT_ATTACHED_HEADER,
                format!("Bearer {assertion}").parse().expect("valid header"),
            );

            let (status, resp_headers, body) = rt.block_on(media_oneshot(
                Arc::clone(&state),
                "GET",
                &path,
                &host,
                headers,
                b"",
            ));

            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "GET Enforce + duplicate Authorization MUST return 403 EvidenceRejected. \
                 Removing the cardinality gate → admitted → sidecar 404 instead."
            );
            assert_eq!(
                body.as_ref(),
                b"evidence rejected\n",
                "GET Enforce cardinality 403 body MUST be exact 'evidence rejected\\n'. \
                 [FI-TRACE-DENIAL-ORACLE]"
            );
            assert_eq!(
                resp_headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                "text/plain; charset=utf-8",
                "GET Enforce cardinality 403 Content-Type MUST be text/plain; charset=utf-8."
            );
            assert!(
                resp_headers.get("www-authenticate").is_none(),
                "GET Enforce cardinality 403 MUST NOT carry WWW-Authenticate. \
                 [FI-TRACE-DENIAL-ORACLE]"
            );
        }

        // ── GET /media: Enforce mode, same-key success → 404 sidecar not found ─
        //
        // A valid NIP-FI assertion + valid Blossom get proof (same key) passes
        // admission and proceeds to the sidecar lookup.  The sidecar blob does
        // not exist in the test state → 404.  This proves admission was NOT the
        // denial point — an always-denying implementation would return 401/403,
        // not 404.
        //
        // Falsifying mutation: lower the NIP-FI gate to always-deny →
        // same-key request returns 401/403 (admission fails) → 404 assertion fires.
        #[test]
        #[ignore = "requires Postgres"]
        fn get_blob_enforce_same_key_admission_succeeds_returns_404() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_enforce_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!("nip-fi-media-enf-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");

            let keys = Keys::generate();
            let sha256 = "0".repeat(64);
            let path = format!("/media/{sha256}.jpg");
            let assertion = signed_assertion(&keys.public_key().to_hex());
            let blossom_val = blossom_get_auth_value(&keys, &host, &sha256);

            let mut headers = axum::http::HeaderMap::new();
            headers.insert(
                axum::http::header::AUTHORIZATION,
                blossom_val.parse().expect("valid blossom header"),
            );
            headers.insert(
                buzz_auth::CLIENT_ATTACHED_HEADER,
                format!("Bearer {assertion}")
                    .parse()
                    .expect("valid assertion header"),
            );

            let (status, _resp_headers, _body) = rt.block_on(media_oneshot(
                Arc::clone(&state),
                "GET",
                &path,
                &host,
                headers,
                b"",
            ));

            // Admission passes → reaches sidecar lookup → blob absent → 404.
            // 401 or 403 would indicate admission failure, not sidecar absence.
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "GET Enforce same-key admission MUST pass NIP-FI and reach sidecar lookup → 404. \
                 401/403 means admission failed (always-deny implementation). \
                 Falsifying mutation: make NIP-FI verifier always-deny → 403 instead of 404."
            );
        }

        // ── HEAD /media: Enforce mode, malformed proof → 403 EvidenceRejected ─
        //
        // Same contract as GET malformed, but for HEAD.  RFC 9110 §9.3.2 suppresses
        // the body; we assert status + CT + no challenge only.
        #[test]
        #[ignore = "requires Postgres"]
        fn head_blob_enforce_malformed_proof_is_403_nip_fi() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_enforce_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!("nip-fi-media-enf-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");

            let sha256 = "1".repeat(64);
            let path = format!("/media/{sha256}.jpg");

            let mut headers = axum::http::HeaderMap::new();
            headers.insert(
                axum::http::header::AUTHORIZATION,
                "Nostr !!!bad!!!".parse().expect("valid header bytes"),
            );
            headers.insert(
                buzz_auth::CLIENT_ATTACHED_HEADER,
                format!(
                    "Bearer {}",
                    signed_assertion(&Keys::generate().public_key().to_hex())
                )
                .parse()
                .expect("valid header"),
            );

            let (status, resp_headers, head_body) = rt.block_on(media_oneshot(
                Arc::clone(&state),
                "HEAD",
                &path,
                &host,
                headers,
                b"",
            ));

            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "HEAD Enforce + malformed Nostr token MUST return 403 EvidenceRejected. \
                 [FI-TRACE-DENIAL-ORACLE]"
            );
            assert!(
                head_body.is_empty(),
                "HEAD MUST suppress the response body (RFC 9110 §9.3.2). \
                 Got {} bytes.",
                head_body.len()
            );
            assert_eq!(
                resp_headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                "text/plain; charset=utf-8",
                "HEAD Enforce malformed 403 Content-Type MUST be text/plain; charset=utf-8."
            );
            assert!(
                resp_headers.get("www-authenticate").is_none(),
                "HEAD Enforce malformed 403 MUST NOT carry WWW-Authenticate."
            );
        }

        // ── HEAD /media: Enforce mode, duplicate Authorization → 403 cardinality ─
        //
        // Same contract as GET cardinality, but for HEAD.  Body suppressed.
        #[test]
        #[ignore = "requires Postgres"]
        fn head_blob_enforce_duplicate_proof_is_403_cardinality() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_enforce_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!("nip-fi-media-enf-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");

            let sha256 = "2".repeat(64);
            let path = format!("/media/{sha256}.jpg");
            let keys = Keys::generate();
            let blossom_val = blossom_get_auth_value(&keys, &host, &sha256);
            let assertion = signed_assertion(&keys.public_key().to_hex());

            let mut headers = axum::http::HeaderMap::new();
            headers.append(
                axum::http::header::AUTHORIZATION,
                blossom_val.parse().expect("valid header"),
            );
            headers.append(
                axum::http::header::AUTHORIZATION,
                blossom_val.parse().expect("valid header"),
            );
            headers.insert(
                buzz_auth::CLIENT_ATTACHED_HEADER,
                format!("Bearer {assertion}").parse().expect("valid header"),
            );

            let (status, resp_headers, head_body) = rt.block_on(media_oneshot(
                Arc::clone(&state),
                "HEAD",
                &path,
                &host,
                headers,
                b"",
            ));

            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "HEAD Enforce + duplicate Authorization MUST return 403 EvidenceRejected. \
                 [FI-TRACE-DENIAL-ORACLE]"
            );
            assert!(
                head_body.is_empty(),
                "HEAD MUST suppress the response body (RFC 9110 §9.3.2). \
                 Got {} bytes.",
                head_body.len()
            );
            assert_eq!(
                resp_headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                "text/plain; charset=utf-8",
                "HEAD Enforce cardinality 403 Content-Type MUST be text/plain; charset=utf-8."
            );
            assert!(
                resp_headers.get("www-authenticate").is_none(),
                "HEAD Enforce cardinality 403 MUST NOT carry WWW-Authenticate."
            );
        }

        // ── HEAD /media: Enforce mode, same-key admission → reaches handler ─
        //
        // Same-key Blossom get-auth + assertion → admission passes → handler
        // attempts sidecar lookup → `read_sidecar_mime` yields `None` → 404.
        //
        // Falsifying mutation: always-deny key pairing → 403 → assertion fires.
        #[test]
        #[ignore = "requires Postgres"]
        fn head_blob_enforce_same_key_admission_succeeds_returns_404() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_enforce_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!(
                "nip-fi-media-enf-hdpos-{}.local",
                uuid::Uuid::new_v4().simple()
            );
            rt.block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community");

            let keys = Keys::generate();
            let sha256 = "5".repeat(64);
            let path = format!("/media/{sha256}.jpg");
            let assertion = signed_assertion(&keys.public_key().to_hex());
            let blossom_token = blossom_get_auth_value(&keys, &host, &sha256);

            let mut headers = axum::http::HeaderMap::new();
            headers.insert(
                axum::http::header::AUTHORIZATION,
                blossom_token.parse().expect("valid header"),
            );
            headers.insert(
                buzz_auth::CLIENT_ATTACHED_HEADER,
                format!("Bearer {assertion}").parse().expect("valid header"),
            );

            let (status, resp_headers, _body_bytes) = rt.block_on(media_oneshot(
                Arc::clone(&state),
                "HEAD",
                &path,
                &host,
                headers,
                b"",
            ));

            // Admission passes → handler proceeds to sidecar lookup → blob absent → 404.
            // 401/403 would indicate NIP-FI admission failure.
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "HEAD /media same-key admission MUST reach handler → 404 (blob not found). \
                 If 401: NIP-FI MissingEvidence — outer guard or assertion check denying. \
                 If 403: NIP-FI AuthorizationDenied — key pairing denying. \
                 Resp headers: {resp_headers:?}. \
                 Falsifying mutation: always-deny pairing → 403 instead of 404."
            );
        }

        // ── Upload resource witness: handler denial precedes body read and permits ─
        //
        // The outer guard (router.rs) forwards any request carrying a
        // cryptographically valid assertion, so every denied request below
        // carries one (for `key_a`) plus a valid Blossom upload proof signed by
        // `key_b`, the real body hash, and the matching host.  Only the
        // handler's `admit_nip_fi_http_on_state` key-pairing check can deny it
        // (403 `authorization denied\n`).
        //
        // Production order in `upload_blob`: admission → x-sha-256 checks →
        // membership → rate limit → `acquire_upload_permit` → body read in
        // `upload_blob_result`.
        //
        // - Denied, instrumented body: zero polls.  Moving admission after the
        //   body read would poll it first.
        // - Denied, all global permits held: still 403, not 429.  Moving
        //   admission after `acquire_upload_permit` would return 429.
        // - `media_uploads_in_flight` for `key_b` is identical before and after
        //   each denied request.  This proves no leaked per-key accounting; it
        //   cannot observe a transient acquire-and-release.
        // - Admitted controls (assertion for `key_b`, same proof) show the
        //   resource boundaries are reachable: with permits held → exact 429
        //   `upload concurrency limit reached` (`acquire_upload_permit` →
        //   `buzz-media/src/error.rs`); with permits free → body polled and
        //   exact 415 `AUDIO_REJECTION`.
        #[test]
        #[ignore = "requires Postgres"]
        fn upload_enforce_denial_does_not_poll_body_or_consume_permit() {
            use std::sync::atomic::{AtomicUsize, Ordering};

            struct CountingBody {
                inner: Option<bytes::Bytes>,
                polls: Arc<AtomicUsize>,
            }
            impl http_body::Body for CountingBody {
                type Data = bytes::Bytes;
                type Error = std::convert::Infallible;
                fn poll_frame(
                    mut self: std::pin::Pin<&mut Self>,
                    _cx: &mut std::task::Context<'_>,
                ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>>
                {
                    self.polls.fetch_add(1, Ordering::SeqCst);
                    std::task::Poll::Ready(self.inner.take().map(|b| Ok(http_body::Frame::data(b))))
                }
            }

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");

            let Some(state) = rt.block_on(media_enforce_test_state()) else {
                panic!("local Postgres not reachable");
            };
            let host = format!(
                "nip-fi-media-witness-{}.local",
                uuid::Uuid::new_v4().simple()
            );
            let community = rt
                .block_on(state.db.ensure_configured_community(&host))
                .expect("ensure community")
                .id;

            let key_a = Keys::generate();
            let key_b = Keys::generate();
            let sha256 = sha256_hex(AUDIO_BODY);
            let proof_b = blossom_upload_auth_value(&key_b, &host, &sha256);
            let mismatched_assertion = signed_assertion(&key_a.public_key().to_hex());
            let same_key_assertion = signed_assertion(&key_b.public_key().to_hex());
            let accounting_key = (community, key_b.public_key().to_bytes());
            let in_flight = || {
                state
                    .media_uploads_in_flight
                    .get(&accounting_key)
                    .map(|count| *count)
            };

            // Send PUT /upload with `assertion`, returning (status, headers, body, polls).
            let send = |assertion: &str| {
                let polls = Arc::new(AtomicUsize::new(0));
                let request = Request::builder()
                    .method("PUT")
                    .uri("/upload")
                    .header("host", &host)
                    .header("authorization", &proof_b)
                    .header("x-sha-256", &sha256)
                    .header(
                        buzz_auth::CLIENT_ATTACHED_HEADER,
                        format!("Bearer {assertion}"),
                    )
                    .body(Body::new(CountingBody {
                        inner: Some(bytes::Bytes::from_static(AUDIO_BODY)),
                        polls: Arc::clone(&polls),
                    }))
                    .expect("build request");
                let response = rt.block_on(async {
                    let resp = crate::router::build_router(Arc::clone(&state))
                        .oneshot(request)
                        .await
                        .expect("router oneshot");
                    let status = resp.status();
                    let headers = resp.headers().clone();
                    let body = to_bytes(resp.into_body(), 8192).await.unwrap_or_default();
                    (status, headers, body)
                });
                (response, polls.load(Ordering::SeqCst))
            };
            let denied = |response: &(StatusCode, axum::http::HeaderMap, bytes::Bytes),
                          context: &str| {
                assert_exact_response(
                    response,
                    StatusCode::FORBIDDEN,
                    "text/plain; charset=utf-8",
                    None,
                    b"authorization denied\n",
                    context,
                );
            };

            // ── Denied request, permits free: body never polled ─────────────
            let before = in_flight();
            let (response, polls) = send(&mismatched_assertion);
            denied(&response, "mismatched key, permits free");
            assert_eq!(polls, 0, "handler denial MUST precede the body read");
            assert_eq!(
                in_flight(),
                before,
                "denial MUST leave per-key accounting unchanged"
            );

            // ── All global permits held ─────────────────────────────────────
            let semaphore = Arc::clone(&state.media_upload_semaphore);
            let held: Vec<_> =
                std::iter::from_fn(|| semaphore.clone().try_acquire_owned().ok()).collect();
            assert!(
                !held.is_empty(),
                "fixture must hold at least one upload permit"
            );

            let before = in_flight();
            let (response, polls) = send(&mismatched_assertion);
            denied(&response, "mismatched key, permits exhausted");
            assert_eq!(polls, 0, "handler denial MUST precede the body read");
            assert_eq!(
                in_flight(),
                before,
                "denial MUST leave per-key accounting unchanged"
            );

            let (response, polls) = send(&same_key_assertion);
            assert_exact_response(
                &response,
                StatusCode::TOO_MANY_REQUESTS,
                "application/json",
                None,
                br#"{"error":"upload concurrency limit reached"}"#,
                "admitted request, permits exhausted",
            );
            assert_eq!(polls, 0, "permit rejection precedes the body read");
            drop(held);

            // ── Admitted request, permits free: body read, exact 415 ────────
            let (response, polls) = send(&same_key_assertion);
            assert_exact_response(
                &response,
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "application/json",
                None,
                AUDIO_REJECTION,
                "admitted request, permits free",
            );
            assert!(polls > 0, "admitted upload MUST read the instrumented body");
            assert_eq!(
                in_flight(),
                None,
                "admitted upload MUST release its per-key slot"
            );
        }
    }
}
