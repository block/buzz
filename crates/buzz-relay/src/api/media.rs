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
use axum::{
    extract::{FromRequestParts, Path, State},
    http::{request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use buzz_audit::{AuditAction, NewAuditEntry};
use buzz_auth::{AuthTransport, AuthorizationCapability, VerifiedEvidenceAdapter};
use buzz_core::tenant::TenantContext;
use buzz_media::{
    BlobDescriptor, MediaError, PreparedUpload, UploadAttribution, UploadNetworkInfo,
    UploadPublicationMode,
};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};

use crate::authorization_runtime::executor::{
    begin_authorized_operation, AuthorizedOperationStart, ProtectedOperationId,
};
use crate::authorization_runtime::transport::{authorize_if_configured, ProtectedAuthorization};
use crate::state::AppState;

fn stable_media_correlation(proof: &buzz_auth::VerifiedNostrProof) -> uuid::Uuid {
    let fingerprint = proof.operation_binding().fingerprint();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&fingerprint[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

/// Axum extractor that validates Blossom auth, the BUD-11 hash binding, and
/// relay membership (NIP-43, when enabled) from headers BEFORE the request
/// body is read. This prevents unauthenticated clients from forcing the
/// server to buffer up to 50MB of body data.
///
/// Axum processes `FromRequestParts` extractors before `FromRequest` (body)
/// extractors, so auth rejection happens before any body buffering.
pub(crate) struct AuthenticatedUpload {
    auth_event: nostr::Event,
    /// Community resolved from the request host at extraction time (row zero for
    /// this HTTP door), identical to the WS door in `router.rs` and the bridge
    /// door in `bridge.rs`. Server-resolved, never client-supplied.
    tenant: TenantContext,
    route_mode: UploadRouteMode,
    protected_authority: Arc<ProtectedAuthorization>,
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

struct MediaReadAuth {
    tenant: TenantContext,
    protected_authority: Option<Arc<ProtectedAuthorization>>,
}

fn protected_media_denied(error: impl std::fmt::Display) -> MediaError {
    tracing::warn!(error = %error, "media: protected authorization denied");
    MediaError::Unauthorized
}

fn release_media_fetched<T>(
    authority: &Option<Arc<ProtectedAuthorization>>,
    value: T,
) -> Result<T, MediaError> {
    release_media_outcome(
        authority
            .as_deref()
            .map(|authority| authority as &dyn crate::connection::OutboundReleaseFence),
        value,
    )
}

fn release_media_outcome<T>(
    authority: Option<&dyn crate::connection::OutboundReleaseFence>,
    value: T,
) -> Result<T, MediaError> {
    if authority.is_some_and(|authority| !authority.release()) {
        return Err(MediaError::Unauthorized);
    }
    Ok(value)
}

impl buzz_media::UploadCommitGuard for ProtectedAuthorization {
    fn revalidate(&self) -> Result<(), MediaError> {
        ProtectedAuthorization::revalidate(self).map_err(protected_media_denied)
    }
}

async fn verify_media_corporate_identity(
    state: &AppState,
    tenant: &TenantContext,
    headers: &HeaderMap,
    pubkey: nostr::PublicKey,
) -> Result<Option<crate::corporate_identity::CorporateIdentityProof>, MediaError> {
    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    let identity_assertion = crate::corporate_identity::identity_assertion_from_headers(
        state,
        tenant.community(),
        headers,
    )
    .map_err(protected_media_denied)?;
    if identity_assertion.is_none()
        && crate::authorization_runtime::transport::provider_evidence_resolver_is_installed(
            state,
            tenant.community(),
        )
    {
        return Ok(None);
    }
    match crate::corporate_identity::verify_corporate_identity(
        state,
        tenant.community(),
        pubkey,
        identity_assertion.as_ref(),
        auth_tag,
    )
    .await
    {
        Ok(proof) => Ok(Some(proof)),
        Err(error)
            if crate::authorization_runtime::transport::legacy_identity_lane(
                state,
                tenant.community(),
            ) == crate::authorization_runtime::transport::LegacyIdentityLane::ObserveOnly =>
        {
            tracing::warn!(error = ?error, "observational media identity verification unavailable");
            Ok(None)
        }
        Err(error) => {
            tracing::warn!(error = ?error, "media: corporate identity denied");
            if error.status_code() == StatusCode::UNAUTHORIZED {
                Err(MediaError::Unauthorized)
            } else {
                Err(MediaError::RelayMembershipRequired)
            }
        }
    }
}

fn seal_media_assertion(
    state: &AppState,
    tenant: &TenantContext,
    proof: Option<&crate::corporate_identity::CorporateIdentityProof>,
    transport: AuthTransport,
) -> Result<Option<Arc<buzz_auth::VerifiedFederatedAssertion>>, MediaError> {
    let Some(proof) = proof else {
        return Ok(None);
    };
    match crate::corporate_identity::current_verified_assertion_for_proof(
        state,
        proof,
        tenant.community(),
        transport,
    ) {
        Ok(assertion) => Ok(assertion.map(Arc::new)),
        Err(error)
            if crate::authorization_runtime::transport::legacy_identity_lane(
                state,
                tenant.community(),
            ) == crate::authorization_runtime::transport::LegacyIdentityLane::ObserveOnly =>
        {
            tracing::warn!(error = %error, "observational media assertion sealing unavailable");
            Ok(None)
        }
        Err(error) => Err(protected_media_denied(error)),
    }
}

async fn finalize_media_corporate_identity(
    state: &AppState,
    tenant: &TenantContext,
    pubkey: nostr::PublicKey,
    proof: Option<crate::corporate_identity::CorporateIdentityProof>,
) -> Result<(), MediaError> {
    if crate::authorization_runtime::transport::legacy_identity_lane(state, tenant.community())
        != crate::authorization_runtime::transport::LegacyIdentityLane::Legacy
    {
        return Ok(());
    }
    let Some(proof) = proof else {
        return Ok(());
    };
    crate::corporate_identity::finalize_corporate_identity(state, tenant.community(), pubkey, proof)
        .await
        .map(|_| ())
        .map_err(|e| {
            tracing::warn!(error = ?e, "media: corporate identity finalization denied");
            if e.status_code() == StatusCode::UNAUTHORIZED {
                MediaError::Unauthorized
            } else {
                MediaError::RelayMembershipRequired
            }
        })
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

fn acquire_protected_upload_permit(
    state: &AppState,
    community_id: buzz_core::CommunityId,
    pubkey: &nostr::PublicKey,
    require_authority: impl FnOnce() -> Result<(), MediaError>,
) -> Result<UploadPermit, MediaError> {
    require_authority()?;
    if upload_rate_limited(state, community_id, pubkey) {
        metrics::counter!("buzz_media_upload_rejections_total", "reason" => "rate_limit")
            .increment(1);
        return Err(MediaError::UploadRateLimitExceeded);
    }
    acquire_upload_permit(state, community_id, pubkey).inspect_err(|_| {
        metrics::counter!("buzz_media_upload_rejections_total", "reason" => "concurrency")
            .increment(1);
    })
}

impl FromRequestParts<Arc<AppState>> for AuthenticatedUpload {
    type Rejection = MediaError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let headers = &parts.headers;

        // 1. Row zero: bind this upload to its community from the request host,
        // identical to the WS door in `router.rs` and the bridge door in
        // `bridge.rs`. Fail-closed: an unmapped host or lookup failure is a
        // generic `NotFound` (404) — never a default tenant, never echoing the
        // host, so an unauthenticated caller cannot probe which communities
        // exist on this deployment.
        //
        // This MUST run before Blossom auth verification (step 2) so the
        // `server`-tag check validates against the *bound tenant host*, not a
        // process-global domain — a relay process serves many tenant hosts, and
        // the stock CLI tags its own configured relay host (conformance row 52).
        // Binding only reads the Host header — no request body is buffered — so
        // doing it first preserves the pre-body auth-rejection guarantee.
        let raw_host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let tenant = crate::tenant::bind_community(&state.db, raw_host)
            .await
            .map_err(|_| MediaError::NotFound)?;

        let route_mode = upload_route_mode(parts.uri.path())?;

        // 2. Extract and validate Blossom auth event against the bound host.
        let auth_event = extract_blossom_auth(headers)?;
        // Use the permissive window (3600s) here because we don't know the
        // content type yet.  The upload functions re-verify with the correct
        // per-type window (600s for images, 3600s for video) after the body
        // has been consumed and the SHA-256 computed.
        buzz_media::auth::verify_blossom_auth_event(&auth_event, Some(tenant.host()), 3600)?;

        // 3. Require X-SHA-256 header (BUD-11: mandatory for PUT /upload)
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

        // 4. Full exact-operation verification in the sealed-evidence adapter.
        // This rechecks signature, upload verb, hash, server, and age together;
        // a different valid kind:24242 event cannot be substituted afterward.
        let verified_blossom = VerifiedEvidenceAdapter::new()
            .verify_blossom_upload(
                tenant.community(),
                &auth_event,
                claimed_hash,
                Some(tenant.host()),
                3600,
            )
            .map_err(protected_media_denied)?;

        // 5. Relay membership gate (NIP-43). Blossom auth proves the signer
        // authorized this exact upload hash for this server; NIP-43 answers
        // whether that Nostr key may use this community's media store. This is
        // the only upload authority: independent of bearer-token / api_tokens
        // storage and of `require_auth_token` (which governs the REST API, not
        // media). On open relays (membership disabled) any valid Blossom signer
        // may upload, matching the WS door's admission policy.
        let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
        let identity_proof =
            verify_media_corporate_identity(state, &tenant, headers, auth_event.pubkey).await?;

        crate::api::relay_members::enforce_relay_membership(
            state,
            tenant.community(),
            auth_event.pubkey.as_bytes(),
            auth_tag,
        )
        .await
        .map_err(|_| MediaError::RelayMembershipRequired)?;
        let verified_blossom =
            match crate::corporate_identity::verify_unconditional_nip_oa_relationship(
                auth_event.pubkey,
                auth_tag,
            ) {
                Some(relationship) => VerifiedEvidenceAdapter::new()
                    .attach_transport_delegation(
                        verified_blossom,
                        buzz_auth::VerifiedDelegationOutput::from_workspace_verifier(
                            relationship.owner_pubkey(),
                            auth_event.pubkey,
                            relationship.relationship_id(),
                            relationship.relationship_revision(),
                            None,
                            true,
                        ),
                    )
                    .map_err(protected_media_denied)?,
                None => verified_blossom,
            };
        let correlation_id = stable_media_correlation(&verified_blossom);
        let verified_assertion = seal_media_assertion(
            state,
            &tenant,
            identity_proof.as_ref(),
            AuthTransport::MediaUpload,
        )?;
        let protected_authority = Arc::new(
            authorize_if_configured(
                state,
                Arc::new(verified_blossom),
                verified_assertion,
                AuthorizationCapability::MediaWrite,
                correlation_id,
                "media.upload",
            )
            .await
            .map_err(protected_media_denied)?,
        );
        let upload_permit =
            acquire_protected_upload_permit(state, tenant.community(), &auth_event.pubkey, || {
                protected_authority
                    .revalidate()
                    .map_err(protected_media_denied)
            })?;
        finalize_media_corporate_identity(state, &tenant, auth_event.pubkey, identity_proof)
            .await?;

        Ok(AuthenticatedUpload {
            auth_event,
            tenant,
            route_mode,
            protected_authority,
            _upload_permit: upload_permit,
        })
    }
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

/// PUT `/upload` or the temporary media-only `/media/upload` alias.
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
///   - `Content-Type` is advisory only; a bounded body prefix selects the
///     streaming video path from actual bytes
///   - Raw binary body (the file bytes)
///
/// Returns a [`BlobDescriptor`] JSON on success.
// TODO(v2): Add persistent per-pubkey storage quotas. Admission limits below
// bound active parser/storage work, but they do not cap durable bytes stored.
pub async fn upload_blob(
    State(state): State<Arc<AppState>>,
    auth: AuthenticatedUpload,
    headers: HeaderMap,
    body: axum::body::Body,
) -> Result<Json<BlobDescriptor>, MediaError> {
    let attribution = upload_attribution(&state, &auth, &headers).await;
    let visibility = state
        .db
        .protected_object_authority(
            auth.tenant.community(),
            buzz_db::protected_visibility::ProtectedObjectSurface::Media,
        )
        .await
        .map_err(|_| MediaError::Internal)?;
    let mut postgresql_visibility = visibility.state
        == buzz_db::protected_visibility::ProtectedObjectAuthorityState::PostgreSql;
    if auth.protected_authority.is_enforcing() && !postgresql_visibility {
        crate::api::media_migration::require_reconciled_authority(&state, &auth.tenant)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "protected media authority migration unavailable");
                MediaError::Unauthorized
            })?;
        postgresql_visibility = true;
    }
    let publication_mode = if postgresql_visibility {
        UploadPublicationMode::ProtectedStaging
    } else {
        UploadPublicationMode::Legacy
    };
    let legacy_visibility = if publication_mode == UploadPublicationMode::ProtectedStaging {
        None
    } else {
        let guard = state
            .db
            .begin_legacy_visibility_write(
                auth.tenant.community(),
                buzz_db::protected_visibility::ProtectedObjectSurface::Media,
            )
            .await
            .map_err(|error| {
                tracing::warn!(%error, "legacy media publication is fenced");
                MediaError::Unauthorized
            })?;
        crate::api::media_migration::require_legacy_sentinel_absent(&state, &auth.tenant)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "legacy media publication is permanently fenced");
                MediaError::Unauthorized
            })?;
        Some(guard)
    };

    if auth.route_mode == UploadRouteMode::LegacyMedia {
        metrics::counter!("buzz_media_legacy_upload_route_total").increment(1);
    }

    // Probe actual bytes without trusting Content-Type. Keep the chunks used
    // for the bounded probe and replay them into the selected pipeline so the
    // stored/hash-verified body remains byte-identical.
    const SNIFF_BYTES: usize = 4096;
    let mut source = body.into_data_stream();
    let mut replay_chunks = Vec::new();
    let mut sniff = Vec::with_capacity(SNIFF_BYTES);
    while sniff.len() < SNIFF_BYTES {
        auth.protected_authority
            .revalidate()
            .map_err(protected_media_denied)?;
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
    let stream_authority = Arc::clone(&auth.protected_authority);
    let guarded_source = source.map(move |item| {
        stream_authority.revalidate().map_err(|error| {
            axum::Error::new(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                error.to_string(),
            ))
        })?;
        item
    });
    let replay =
        futures_util::stream::iter(replay_chunks.into_iter().map(Ok)).chain(guarded_source);

    let prepared = if should_stream_as_video(&sniff) {
        // Video path: stream body directly to disk — never fully buffered in RAM.
        let content_length = headers
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        auth.protected_authority
            .revalidate()
            .map_err(protected_media_denied)?;
        buzz_media::process_video_upload(
            &state.media_storage,
            &state.config.media,
            &auth.tenant,
            &auth.auth_event,
            replay,
            content_length,
            attribution,
            auth.protected_authority.as_ref(),
            publication_mode,
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
        let bytes = axum::body::to_bytes(axum::body::Body::from_stream(replay), max as usize)
            .await
            .map_err(|_| MediaError::FileTooLarge { size: 0, max })?;

        let is_image = matches!(
            infer::get(&bytes).map(|t| t.mime_type()),
            Some("image/jpeg" | "image/png" | "image/gif" | "image/webp")
        );

        if is_image {
            auth.protected_authority
                .revalidate()
                .map_err(protected_media_denied)?;
            buzz_media::process_upload(buzz_media::upload::BufferedUploadRequest {
                storage: &state.media_storage,
                config: &state.config.media,
                ctx: &auth.tenant,
                auth_event: &auth.auth_event,
                body: bytes,
                attribution,
                commit_guard: auth.protected_authority.as_ref(),
                publication_mode,
            })
            .await?
        } else if auth.route_mode == UploadRouteMode::LegacyMedia {
            let mime = infer::get(&bytes)
                .map(|kind| kind.mime_type().to_string())
                .unwrap_or_else(|| "application/octet-stream".to_string());
            return Err(MediaError::DisallowedContentType(mime));
        } else {
            auth.protected_authority
                .revalidate()
                .map_err(protected_media_denied)?;
            buzz_media::process_file_upload(buzz_media::upload::BufferedUploadRequest {
                storage: &state.media_storage,
                config: &state.config.media,
                ctx: &auth.tenant,
                auth_event: &auth.auth_event,
                body: bytes,
                attribution,
                commit_guard: auth.protected_authority.as_ref(),
                publication_mode,
            })
            .await?
        }
    };

    let mut prepared = prepared;
    rewrite_descriptor_urls_for_tenant(
        &mut prepared.descriptor,
        &state.config.relay_url,
        auth.tenant.host(),
    );

    let descriptor =
        commit_media_publication(&state, &auth, prepared, postgresql_visibility).await?;
    if let Some(legacy_visibility) = legacy_visibility {
        legacy_visibility.commit().await.map_err(|error| {
            tracing::error!(%error, "legacy media publication fence commit failed");
            MediaError::Internal
        })?;
    }

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
        "community" => crate::metrics::community_label(auth.tenant.community())
    )
    .increment(1);

    // Audit via bounded channel — same pattern as event audit.
    if crate::protected_surface::require_effect_permit(
        state
            .protected_transport()
            .and_then(|runtime| runtime.mode_for_domain(auth.tenant.community())),
        crate::protected_surface::EffectSurfaceId::LegacyAuditDelivery,
    )
    .is_ok()
    {
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
    }

    Ok(Json(descriptor))
}

async fn commit_media_publication(
    state: &AppState,
    auth: &AuthenticatedUpload,
    prepared: PreparedUpload,
    postgresql_visibility: bool,
) -> Result<BlobDescriptor, MediaError> {
    if !postgresql_visibility {
        return Ok(prepared.descriptor);
    }
    let PreparedUpload {
        descriptor,
        metadata,
        object_key,
        thumbnail_key,
    } = prepared;
    let metadata_json = serde_json::to_value(&metadata).map_err(|_| MediaError::Internal)?;
    let publication = buzz_db::protected_publication::MediaPublication {
        sha256: descriptor.sha256.clone(),
        object_key,
        extension: metadata.ext.clone(),
        mime_type: metadata.mime_type.clone(),
        object_size: metadata.size,
        metadata: metadata_json,
        thumbnail_key,
        publication_version: 1,
    };
    if auth.protected_authority.is_enforcing() {
        let operation_id = ProtectedOperationId::derive(
            auth.tenant.community(),
            "media.upload",
            auth.auth_event.id.as_bytes(),
        )
        .map_err(protected_media_denied)?;
        let mut request_digest = Sha256::new();
        request_digest.update(b"buzz-media-publication-v1");
        request_digest.update(descriptor.sha256.as_bytes());
        request_digest.update(metadata.ext.as_bytes());
        request_digest.update(metadata.mime_type.as_bytes());
        request_digest.update(metadata.size.to_be_bytes());
        let request_fingerprint: [u8; 32] = request_digest.finalize().into();
        let permit = auth
            .protected_authority
            .seal_postgres_mutation(operation_id, "media.upload", request_fingerprint)
            .map_err(protected_media_denied)?
            .ok_or(MediaError::Unauthorized)?;
        match begin_authorized_operation(state, permit)
            .await
            .map_err(protected_media_denied)?
        {
            AuthorizedOperationStart::Replay(payload) => {
                serde_json::from_slice(&payload).map_err(|_| MediaError::Internal)
            }
            AuthorizedOperationStart::Execute(mut operation) => {
                buzz_db::protected_publication::publish_media(
                    operation.transaction(),
                    auth.tenant.community(),
                    &publication,
                )
                .await
                .map_err(protected_media_denied)?;
                let payload = serde_json::to_vec(&descriptor).map_err(|_| MediaError::Internal)?;
                operation
                    .commit(&payload)
                    .await
                    .map_err(protected_media_denied)?;
                Ok(descriptor)
            }
        }
    } else {
        // Storage authority remains PostgreSQL after cutover even when the
        // protected authorization mode is Off, Shadow, or VerifyOnly. Preserve
        // legacy authorization semantics, publish no sidecar, and commit the
        // immutable descriptor through the PostgreSQL visibility transaction.
        let mut transaction = state
            .db
            .begin_transaction()
            .await
            .map_err(protected_media_denied)?;
        buzz_db::protected_publication::publish_media(
            &mut transaction,
            auth.tenant.community(),
            &publication,
        )
        .await
        .map_err(protected_media_denied)?;
        transaction.commit().await.map_err(protected_media_denied)?;
        Ok(descriptor)
    }
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

async fn authenticate_media_read(
    state: &AppState,
    headers: &HeaderMap,
    sha256_ext: &str,
) -> Result<MediaReadAuth, MediaError> {
    let tenant = bind_media_read_tenant(state, headers).await?;

    let enforcing =
        crate::authorization_runtime::transport::legacy_identity_lane(state, tenant.community())
            == crate::authorization_runtime::transport::LegacyIdentityLane::ProtectedEnforce;
    if !state.config.require_media_get_auth && !enforcing {
        return Ok(MediaReadAuth {
            tenant,
            protected_authority: None,
        });
    }

    let auth_event = extract_blossom_auth(headers)?;
    let sha256 = sha256_ext.split('.').next().unwrap_or(sha256_ext);
    let verified_blossom = VerifiedEvidenceAdapter::new()
        .verify_blossom_download(
            tenant.community(),
            &auth_event,
            sha256,
            Some(tenant.host()),
            3600,
        )
        .map_err(protected_media_denied)?;

    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    let identity_proof =
        verify_media_corporate_identity(state, &tenant, headers, auth_event.pubkey).await?;
    crate::api::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        auth_event.pubkey.as_bytes(),
        auth_tag,
    )
    .await
    .map_err(|_| MediaError::RelayMembershipRequired)?;
    let verified_blossom = match crate::corporate_identity::verify_unconditional_nip_oa_relationship(
        auth_event.pubkey,
        auth_tag,
    ) {
        Some(relationship) => VerifiedEvidenceAdapter::new()
            .attach_transport_delegation(
                verified_blossom,
                buzz_auth::VerifiedDelegationOutput::from_workspace_verifier(
                    relationship.owner_pubkey(),
                    auth_event.pubkey,
                    relationship.relationship_id(),
                    relationship.relationship_revision(),
                    None,
                    true,
                ),
            )
            .map_err(protected_media_denied)?,
        None => verified_blossom,
    };
    let correlation_id = stable_media_correlation(&verified_blossom);
    let verified_assertion = seal_media_assertion(
        state,
        &tenant,
        identity_proof.as_ref(),
        AuthTransport::MediaDownload,
    )?;
    let protected_authority = Arc::new(
        authorize_if_configured(
            state,
            Arc::new(verified_blossom),
            verified_assertion,
            AuthorizationCapability::MediaRead,
            correlation_id,
            "media.read",
        )
        .await
        .map_err(protected_media_denied)?,
    );
    protected_authority
        .revalidate()
        .map_err(protected_media_denied)?;
    finalize_media_corporate_identity(state, &tenant, auth_event.pubkey, identity_proof).await?;

    Ok(MediaReadAuth {
        tenant,
        protected_authority: Some(protected_authority),
    })
}

fn blob_cache_control(require_auth: bool) -> &'static str {
    if require_auth {
        "private, max-age=31536000, immutable"
    } else {
        "public, max-age=31536000, immutable"
    }
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
    let media_auth = authenticate_media_read(&state, &req_headers, &sha256_ext).await?;
    serve_blob_for_tenant(
        &state,
        &media_auth.tenant,
        &sha256_ext,
        &req_headers,
        media_auth.protected_authority,
    )
    .await
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
    protected_authority: Option<Arc<ProtectedAuthorization>>,
) -> Result<Response, MediaError> {
    validate_media_path(sha256_ext)?;
    if let Some(authority) = &protected_authority {
        authority.revalidate().map_err(protected_media_denied)?;
    }
    let cache_control =
        blob_cache_control(state.config.require_media_get_auth || protected_authority.is_some());

    let (content_type, key) =
        resolve_visible_media(state, tenant, sha256_ext, &protected_authority).await?;

    // Images and video render inline; generic files force download. This is the
    // primary defence for non-previewable types — combined with `nosniff` and
    // `CSP: default-src 'none'`, an attachment disposition prevents an uploaded
    // file from ever executing or rendering as active content in the client.
    let disposition = if buzz_media::serve_inline(&content_type) {
        "inline"
    } else {
        "attachment"
    };

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
            let total = release_media_fetched(
                &protected_authority,
                state.media_storage.head_with_metadata(&key).await,
            )??
            .ok_or(MediaError::NotFound)?
            .size;
            let stream = release_media_fetched(
                &protected_authority,
                state.media_storage.get_stream(&key).await,
            )??;
            let stream = stream.map(move |item| {
                if let Some(authority) = &protected_authority {
                    authority
                        .release_fetched(())
                        .map_err(protected_media_denied)?;
                }
                item
            });
            let resp = axum::response::Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, &content_type)
                .header(header::CONTENT_LENGTH, total.to_string())
                .header(header::CONTENT_DISPOSITION, disposition)
                .header(header::CACHE_CONTROL, cache_control)
                .header(header::CONTENT_SECURITY_POLICY, "default-src 'none'")
                .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                .header(header::ACCEPT_RANGES, "bytes")
                .body(axum::body::Body::from_stream(stream))
                .map_err(|_| MediaError::Internal)?;
            Ok(resp)
        }
        Some(range_str) => {
            // S3-native single-range response, capped to bound request memory.
            let total = release_media_fetched(
                &protected_authority,
                state.media_storage.head_with_metadata(&key).await,
            )??
            .ok_or(MediaError::NotFound)?
            .size;

            let parsed = parse_byte_range(&range_str, total);
            match parsed {
                Some((start, end)) => {
                    if start >= total {
                        if let Some(authority) = &protected_authority {
                            authority
                                .release_fetched(())
                                .map_err(protected_media_denied)?;
                        }
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
                    if let Some(authority) = &protected_authority {
                        authority.revalidate().map_err(protected_media_denied)?;
                    }
                    let chunk = release_media_fetched(
                        &protected_authority,
                        state.media_storage.get_range(&key, start, end).await,
                    )??;
                    let content_range = format!("bytes {start}-{end}/{total}");

                    Ok(axum::response::Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::CONTENT_TYPE, &content_type)
                        .header(header::CONTENT_RANGE, content_range)
                        .header(header::CONTENT_LENGTH, chunk.len().to_string())
                        .header(header::CONTENT_DISPOSITION, disposition)
                        .header(header::ACCEPT_RANGES, "bytes")
                        .header(header::CACHE_CONTROL, cache_control)
                        .header(header::CONTENT_SECURITY_POLICY, "default-src 'none'")
                        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                        .body(axum::body::Body::from(chunk))
                        .map_err(|_| MediaError::Internal)?)
                }
                None => {
                    if let Some(authority) = &protected_authority {
                        authority
                            .release_fetched(())
                            .map_err(protected_media_denied)?;
                    }
                    Ok(axum::response::Response::builder()
                        .status(StatusCode::RANGE_NOT_SATISFIABLE)
                        .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                        .body(axum::body::Body::empty())
                        .map_err(|_| MediaError::Internal)?)
                }
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
    headers: HeaderMap,
    Path(sha256_ext): Path<String>,
) -> Result<Response, MediaError> {
    validate_media_path(&sha256_ext)?;
    let media_auth = authenticate_media_read(&state, &headers, &sha256_ext).await?;
    if let Some(authority) = &media_auth.protected_authority {
        authority.revalidate().map_err(protected_media_denied)?;
    }
    let tenant = media_auth.tenant;
    let cache_control = blob_cache_control(
        state.config.require_media_get_auth || media_auth.protected_authority.is_some(),
    );

    let (content_type, key) = resolve_visible_media(
        &state,
        &tenant,
        &sha256_ext,
        &media_auth.protected_authority,
    )
    .await?;
    if let Some(authority) = &media_auth.protected_authority {
        authority.revalidate().map_err(protected_media_denied)?;
    }
    let metadata = release_media_fetched(
        &media_auth.protected_authority,
        state.media_storage.head_with_metadata(&key).await,
    )??;
    match metadata {
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

pub(crate) async fn resolve_visible_media(
    state: &AppState,
    tenant: &TenantContext,
    sha256_ext: &str,
    authority: &Option<Arc<ProtectedAuthorization>>,
) -> Result<(String, String), MediaError> {
    let enforcing =
        crate::authorization_runtime::transport::legacy_identity_lane(state, tenant.community())
            == crate::authorization_runtime::transport::LegacyIdentityLane::ProtectedEnforce;
    let mut visibility = state
        .db
        .protected_object_authority(
            tenant.community(),
            buzz_db::protected_visibility::ProtectedObjectSurface::Media,
        )
        .await
        .map_err(|_| MediaError::Internal)?;
    if enforcing
        && visibility.state
            != buzz_db::protected_visibility::ProtectedObjectAuthorityState::PostgreSql
    {
        crate::api::media_migration::require_reconciled_authority(state, tenant)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "protected media authority migration unavailable");
                MediaError::Unauthorized
            })?;
        visibility = state
            .db
            .protected_object_authority(
                tenant.community(),
                buzz_db::protected_visibility::ProtectedObjectSurface::Media,
            )
            .await
            .map_err(|_| MediaError::Internal)?;
    }
    if visibility.state == buzz_db::protected_visibility::ProtectedObjectAuthorityState::PostgreSql
    {
        // The one-way cutover selects PostgreSQL visibility in every mode.
        // Off, Shadow, and VerifyOnly preserve their legacy authorization
        // decision but never regress to mutable sidecars after the sentinel.
        let sha256 = sha256_ext.split('.').next().unwrap_or(sha256_ext);
        let publication = release_media_fetched(
            authority,
            state.db.media_publication(tenant.community(), sha256).await,
        )?
        .map_err(protected_media_denied)?
        .ok_or(MediaError::NotFound)?;
        if sha256_ext.ends_with(".thumb.jpg") {
            return Ok((
                "image/jpeg".to_string(),
                publication.thumbnail_key.ok_or(MediaError::NotFound)?,
            ));
        }
        if sha256_ext.contains('.') {
            let requested_ext = sha256_ext.rsplit('.').next().unwrap_or("");
            if requested_ext != publication.extension {
                return Err(MediaError::NotFound);
            }
        }
        return Ok((publication.mime_type, publication.object_key));
    }

    if enforcing {
        return Err(MediaError::Unauthorized);
    }
    crate::api::media_migration::require_legacy_sentinel_absent(state, tenant)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "legacy media visibility is permanently fenced");
            MediaError::Unauthorized
        })?;
    // Before cutover, Off, Shadow, and VerifyOnly retain the exact tenant-sidecar
    // visibility contract. A PostgreSQL marker above is monotonic and never
    // falls back to the mutable sidecar state.
    let content_type = if sha256_ext.ends_with(".thumb.jpg") {
        let parent_hash = sha256_ext.strip_suffix(".thumb.jpg").unwrap_or(sha256_ext);
        let _ = release_media_fetched(
            authority,
            state
                .media_storage
                .read_sidecar_mime(tenant, parent_hash)
                .await,
        )?
        .ok_or(MediaError::NotFound)?;
        "image/jpeg".to_string()
    } else {
        let sidecar_mime = release_media_fetched(
            authority,
            state
                .media_storage
                .read_sidecar_mime(tenant, sha256_ext)
                .await,
        )?
        .ok_or(MediaError::NotFound)?;
        if sha256_ext.contains('.') {
            let requested_ext = sha256_ext.rsplit('.').next().unwrap_or("");
            let sidecar = release_media_fetched(
                authority,
                state
                    .media_storage
                    .get_sidecar(tenant, sha256_ext.split('.').next().unwrap_or(sha256_ext))
                    .await,
            )?
            .map_err(|_| MediaError::NotFound)?;
            if requested_ext != sidecar.ext {
                return Err(MediaError::NotFound);
            }
        }
        sidecar_mime
    };
    let key = release_media_fetched(
        authority,
        resolve_s3_key(&state.media_storage, tenant, sha256_ext).await,
    )??;
    Ok((content_type, key))
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
    };
    use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
    use tower::ServiceExt;
    use uuid::Uuid;

    struct ScriptedMediaFence(AtomicBool);

    impl crate::connection::OutboundReleaseFence for ScriptedMediaFence {
        fn release(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn media_outcomes_release_only_after_post_fetch_authority_check() {
        let fence = ScriptedMediaFence(AtomicBool::new(true));
        let success = release_media_outcome(Some(&fence), Ok::<_, &'static str>(Some("blob")))
            .expect("current authority releases fetched success");
        assert_eq!(success, Ok(Some("blob")));

        fence.0.store(false, Ordering::SeqCst);
        for fetched in [Ok(Some("blob")), Ok(None), Err("storage unavailable")] {
            assert!(matches!(
                release_media_outcome(Some(&fence), fetched),
                Err(MediaError::Unauthorized)
            ));
        }
    }

    const VALID_HASH: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

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

    async fn test_state() -> Arc<AppState> {
        test_state_with_media_auth(false, false).await
    }

    async fn test_state_with_media_get_auth(require_media_get_auth: bool) -> Arc<AppState> {
        test_state_with_media_auth(require_media_get_auth, false).await
    }

    async fn test_state_with_media_auth(
        require_media_get_auth: bool,
        require_corporate_identity: bool,
    ) -> Arc<AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.require_media_get_auth = require_media_get_auth;
        config.corporate_identity.require = require_corporate_identity;
        if require_corporate_identity {
            config.corporate_identity.jwks_uri = "http://127.0.0.1:9/jwks".to_string();
            config.corporate_identity.issuer = "https://idp.example".to_string();
            config.corporate_identity.audience = "buzz-relay".to_string();
        }
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

    async fn media_get_auth_router(require_media_get_auth: bool) -> axum::Router {
        let state = test_state_with_media_get_auth(require_media_get_auth).await;
        axum::Router::new()
            .route(
                "/media/{sha256_ext}",
                axum::routing::get(get_blob).head(head_blob),
            )
            .with_state(state)
    }

    async fn media_get_auth_router_with_corporate_identity() -> axum::Router {
        let state = test_state_with_media_auth(true, true).await;
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
    async fn media_get_auth_flag_off_allows_unauthenticated_read_until_sidecar_gate() {
        let response = media_get_auth_router(false)
            .await
            .oneshot(media_request("GET", None))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn media_get_auth_flag_on_rejects_unauthenticated_get_and_head_before_sidecar_gate() {
        for method in ["GET", "HEAD"] {
            let response = media_get_auth_router(true)
                .await
                .oneshot(media_request(method, None))
                .await
                .expect("response");

            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{method}");
        }
    }

    #[tokio::test]
    async fn media_get_auth_flag_on_valid_server_scoped_token_reaches_sidecar_gate() {
        let keys = Keys::generate();
        let auth = media_get_auth_header(&keys, media_get_tags_for("relay.example", None));
        let response = media_get_auth_router(true)
            .await
            .oneshot(media_request("GET", Some(auth)))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn protected_media_reads_require_corporate_identity_for_get_and_head() {
        let keys = Keys::generate();

        for method in ["GET", "HEAD"] {
            let auth = media_get_auth_header(&keys, media_get_tags_for("relay.example", None));
            let response = media_get_auth_router_with_corporate_identity()
                .await
                .oneshot(media_request(method, Some(auth)))
                .await
                .expect("response");

            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{method}");
        }
    }

    #[tokio::test]
    async fn media_get_auth_flag_on_rejects_upload_verb_wrong_server_and_wrong_x() {
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
            let response = media_get_auth_router(true)
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
    async fn media_get_auth_flag_on_accepts_range_header_only_after_auth() {
        let keys = Keys::generate();
        let auth = media_get_auth_header(&keys, media_get_tags_for("relay.example", None));
        let mut request = media_request("GET", Some(auth));
        request
            .headers_mut()
            .insert(header::RANGE, "bytes=0-0".parse().expect("range header"));

        let response = media_get_auth_router(true)
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

    #[tokio::test]
    async fn denied_protected_upload_consumes_no_rate_or_concurrency_state() {
        let state = test_state().await;
        let pubkey = nostr::Keys::generate().public_key();
        let community = buzz_core::CommunityId::from_uuid(Uuid::from_u128(0xCA5));
        let key = (community, pubkey.to_bytes());

        assert!(matches!(
            acquire_protected_upload_permit(&state, community, &pubkey, || {
                Err(MediaError::Unauthorized)
            }),
            Err(MediaError::Unauthorized)
        ));
        assert!(!state.media_upload_rate_limiter.contains_key(&key));
        assert!(!state.media_uploads_in_flight.contains_key(&key));

        assert!(
            !upload_rate_limited(&state, community, &pubkey),
            "the first legacy retry must retain its full rate budget"
        );
        let permit = acquire_upload_permit(&state, community, &pubkey)
            .expect("the first legacy retry must retain its concurrency slot");
        drop(permit);
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
}
