//! Media error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Coarse Blossom denial kind for NIP-FI response shaping.
///
/// Callers that know the active [`crate::auth::BlossomStrictness`] use this to
/// choose the correct HTTP response shape — NIP-FI fixed text/plain in Strict
/// mode, legacy JSON in Permissive mode — without requiring `buzz-media` to
/// depend on `buzz-auth`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlossomDenialKind {
    /// `Authorization` header was absent. Maps to HTTP 401 + `WWW-Authenticate: Nostr`.
    MissingEvidence,
    /// Authorization header or proof was present but structurally invalid,
    /// malformed, expired, or otherwise rejected. Maps to HTTP 403.
    EvidenceRejected,
}

/// Errors from media operations.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("unknown content type")]
    UnknownContentType,
    #[error("disallowed content type: {0}")]
    DisallowedContentType(String),
    #[error("file too large: {size} bytes (max {max})")]
    FileTooLarge { size: u64, max: u64 },
    #[error("image dimensions too large")]
    ImageTooLarge,
    #[error("invalid image data")]
    InvalidImage,
    #[error("media contains metadata or a non-canonical metadata channel")]
    MetadataForbidden,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("invalid auth event kind")]
    InvalidAuthKind,
    #[error("invalid auth verb")]
    InvalidAuthVerb,
    #[error("missing required tag: {0}")]
    MissingTag(&'static str),
    /// A tag that must appear exactly once appeared more than once.
    #[error("duplicate tag: {0}")]
    DuplicateTag(&'static str),
    #[error("hash mismatch")]
    HashMismatch,
    #[error("server mismatch")]
    ServerMismatch,
    #[error("token expired")]
    TokenExpired,
    #[error("timestamp out of window")]
    TimestampOutOfWindow,
    #[error("storage error: {0}")]
    StorageError(String),
    #[error("internal error")]
    Internal,
    #[error("not found")]
    NotFound,
    #[error("missing authorization header")]
    MissingAuth,
    #[error("invalid authorization scheme")]
    InvalidAuthScheme,
    #[error("invalid base64 encoding")]
    InvalidBase64,
    #[error("invalid auth event")]
    InvalidAuthEvent,
    #[error("unauthorized")]
    Unauthorized,
    #[error("insufficient scope")]
    InsufficientScope,
    #[error("relay membership required")]
    RelayMembershipRequired,
    #[error("community writes are fenced")]
    CommunityWriteFenced,
    #[error("media service temporarily unavailable")]
    ServiceUnavailable,
    #[error("token revoked")]
    TokenRevoked,
    #[error("pubkey mismatch")]
    PubkeyMismatch,
    #[error("upload rate limit exceeded")]
    UploadRateLimitExceeded,
    #[error("upload concurrency limit reached")]
    UploadConcurrencyLimitReached,
    /// A video/audio track does not use the canonical H.264/AAC codecs.
    #[error("unsupported media codec: only H.264 video and AAC audio are accepted")]
    WrongCodec,
    /// Video duration exceeds the 600-second limit.
    #[error("video too long: duration exceeds 600 seconds")]
    DurationTooLong,
    /// Video resolution exceeds the 2160 short-edge / 3840 long-edge envelope.
    #[error(
        "video resolution too high: maximum is 2160 on the short edge and 3840 on the long edge"
    )]
    ResolutionTooHigh,
    /// MP4 moov atom appears after mdat — not fast-start.
    #[error("moov atom not at front of file (not fast-start)")]
    MoovNotAtFront,
    /// Container is not MP4 (e.g. MOV, MKV).
    #[error("unsupported container: only MP4 is accepted")]
    UnsupportedContainer,
    /// MP4 metadata could not be parsed.
    #[error("invalid video data")]
    InvalidVideo,
    /// I/O error during streaming upload.
    #[error("io error: {0}")]
    Io(String),
}

impl From<image::ImageError> for MediaError {
    fn from(_: image::ImageError) -> Self {
        Self::InvalidImage
    }
}

impl From<s3::error::S3Error> for MediaError {
    fn from(e: s3::error::S3Error) -> Self {
        Self::StorageError(e.to_string())
    }
}

impl From<serde_json::Error> for MediaError {
    fn from(e: serde_json::Error) -> Self {
        Self::StorageError(e.to_string())
    }
}

impl MediaError {
    /// Classify this error as a Blossom denial kind for response-shape selection.
    ///
    /// Returns `Some(BlossomDenialKind::MissingEvidence)` when the
    /// `Authorization` header was absent, `Some(BlossomDenialKind::EvidenceRejected)`
    /// for any structurally present but invalid/malformed/expired proof, and
    /// `None` for non-auth errors.
    ///
    /// Relay call sites that know the active `BlossomStrictness` use this to
    /// select the appropriate response shape: NIP-FI fixed text/plain in Strict
    /// mode, legacy JSON 401 in Permissive mode.
    pub fn blossom_denial_kind(&self) -> Option<BlossomDenialKind> {
        match self {
            Self::MissingAuth => Some(BlossomDenialKind::MissingEvidence),
            Self::InvalidAuthScheme
            | Self::InvalidBase64
            | Self::InvalidAuthEvent
            | Self::InvalidAuthKind
            | Self::InvalidAuthVerb
            | Self::DuplicateTag(_)
            | Self::InvalidSignature
            | Self::TokenExpired
            | Self::TimestampOutOfWindow
            | Self::Unauthorized
            | Self::TokenRevoked
            | Self::PubkeyMismatch
            | Self::HashMismatch
            | Self::ServerMismatch
            | Self::MissingTag(_) => Some(BlossomDenialKind::EvidenceRejected),
            _ => None,
        }
    }
}

impl IntoResponse for MediaError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            Self::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            Self::DisallowedContentType(_) => {
                (StatusCode::UNSUPPORTED_MEDIA_TYPE, self.to_string())
            }
            Self::FileTooLarge { .. } | Self::ImageTooLarge => {
                (StatusCode::PAYLOAD_TOO_LARGE, self.to_string())
            }
            // All authentication failures return the same generic 401 to prevent oracle
            // enumeration in the legacy/Permissive path [FI-INV-15].  Off-mode deployments
            // preserve this pre-NIP-FI behavior.  InsufficientScope is intentionally 403
            // below — it is an authorization (not authentication) failure and is safe to
            // distinguish because it requires a valid identity first.
            //
            // Strict mode routes through MediaDenial (buzz-relay) and applies the full
            // NIP-FI rejection table (missing_evidence → 401, evidence_rejected → 403)
            // independently of this path.
            Self::MissingAuth
            | Self::InvalidAuthScheme
            | Self::InvalidBase64
            | Self::InvalidAuthEvent
            | Self::InvalidSignature
            | Self::InvalidAuthKind
            | Self::InvalidAuthVerb
            | Self::DuplicateTag(_)
            | Self::TokenExpired
            | Self::TimestampOutOfWindow
            | Self::Unauthorized
            | Self::TokenRevoked
            | Self::PubkeyMismatch
            | Self::HashMismatch
            | Self::ServerMismatch
            | Self::MissingTag(_) => {
                tracing::warn!(error = %self, "authentication failed");
                (
                    StatusCode::UNAUTHORIZED,
                    "authentication failed".to_string(),
                )
            }
            Self::InsufficientScope => (StatusCode::FORBIDDEN, self.to_string()),
            Self::RelayMembershipRequired | Self::CommunityWriteFenced => {
                (StatusCode::FORBIDDEN, self.to_string())
            }
            Self::ServiceUnavailable => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            Self::UploadRateLimitExceeded | Self::UploadConcurrencyLimitReached => {
                (StatusCode::TOO_MANY_REQUESTS, self.to_string())
            }
            Self::UnknownContentType | Self::UnsupportedContainer | Self::WrongCodec => {
                (StatusCode::UNSUPPORTED_MEDIA_TYPE, self.to_string())
            }
            Self::DurationTooLong
            | Self::ResolutionTooHigh
            | Self::MoovNotAtFront
            | Self::InvalidVideo
            | Self::InvalidImage
            | Self::MetadataForbidden => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            Self::Io(_) | Self::StorageError(_) | Self::Internal => {
                tracing::error!(error = %self, "media storage error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
        };
        (status, axum::Json(serde_json::json!({"error": msg}))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── BlossomDenialKind classification ─────────────────────────────────────

    #[test]
    fn missing_auth_is_missing_evidence() {
        assert_eq!(
            MediaError::MissingAuth.blossom_denial_kind(),
            Some(BlossomDenialKind::MissingEvidence)
        );
    }

    #[test]
    fn structural_proof_errors_are_evidence_rejected() {
        for error in [
            MediaError::InvalidAuthScheme,
            MediaError::InvalidBase64,
            MediaError::InvalidAuthEvent,
            MediaError::InvalidAuthKind,
            MediaError::InvalidAuthVerb,
            MediaError::DuplicateTag("Authorization"),
            MediaError::InvalidSignature,
            MediaError::TokenExpired,
            MediaError::TimestampOutOfWindow,
            MediaError::Unauthorized,
            MediaError::TokenRevoked,
            MediaError::PubkeyMismatch,
            MediaError::HashMismatch,
            MediaError::ServerMismatch,
            MediaError::MissingTag("t"),
        ] {
            assert_eq!(
                error.blossom_denial_kind(),
                Some(BlossomDenialKind::EvidenceRejected),
                "expected EvidenceRejected for {error:?}"
            );
        }
    }

    #[test]
    fn non_auth_errors_have_no_denial_kind() {
        for error in [
            MediaError::NotFound,
            MediaError::FileTooLarge { size: 1, max: 0 },
            MediaError::Internal,
            MediaError::ServiceUnavailable,
            MediaError::InsufficientScope,
        ] {
            assert_eq!(
                error.blossom_denial_kind(),
                None,
                "expected None for {error:?}"
            );
        }
    }

    // ── IntoResponse status code pins ──────────────────────────────────────
    // These pins cover the legacy/Permissive path (MediaError::into_response).
    // Strict mode overrides this via MediaDenial in buzz-relay [FI-INV-15].

    #[tokio::test]
    async fn all_auth_failures_return_json_401_in_permissive_path() {
        // In the legacy/Permissive path all auth failures collapse to a single
        // JSON 401 to prevent oracle enumeration [FI-INV-15].  Strict mode
        // (MediaDenial in buzz-relay) applies the NIP-FI 401/403 split instead.
        for error in [
            MediaError::MissingAuth,
            MediaError::InvalidAuthScheme,
            MediaError::InvalidBase64,
            MediaError::InvalidAuthEvent,
            MediaError::InvalidAuthKind,
            MediaError::InvalidAuthVerb,
            MediaError::DuplicateTag("Authorization"),
            MediaError::InvalidSignature,
            MediaError::TokenExpired,
            MediaError::TimestampOutOfWindow,
            MediaError::Unauthorized,
            MediaError::TokenRevoked,
            MediaError::PubkeyMismatch,
            MediaError::HashMismatch,
            MediaError::ServerMismatch,
            MediaError::MissingTag("t"),
        ] {
            let label = format!("{error:?}");
            let resp = error.into_response();
            assert_eq!(
                resp.status(),
                StatusCode::UNAUTHORIZED,
                "Permissive path: expected 401 for {label}"
            );
            let ct = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert!(
                ct.contains("application/json"),
                "expected JSON CT for {label}, got: {ct}"
            );
            // No WWW-Authenticate in the legacy JSON path.
            assert!(
                resp.headers().get("www-authenticate").is_none(),
                "Permissive path must not include WWW-Authenticate for {label}"
            );
            // Exact body: legacy path emits {"error":"authentication failed"} [FI-INV-15].
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .expect("body collect");
            assert_eq!(
                body.as_ref(),
                br#"{"error":"authentication failed"}"#,
                "Permissive path: wrong body for {label}"
            );
        }
    }

    // ── Existing non-auth response map tests ────────────────────────────────

    #[test]
    fn serving_backend_failures_map_to_5xx_but_fences_remain_403() {
        for error in [
            MediaError::ServiceUnavailable,
            MediaError::Internal,
            MediaError::StorageError("backend".to_string()),
        ] {
            assert!(error.into_response().status().is_server_error());
        }
        assert_eq!(
            MediaError::CommunityWriteFenced.into_response().status(),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn unsupported_media_maps_to_415() {
        for error in [
            MediaError::UnknownContentType,
            MediaError::DisallowedContentType("audio/mpeg".to_string()),
            MediaError::UnsupportedContainer,
            MediaError::WrongCodec,
        ] {
            assert_eq!(
                error.into_response().status(),
                StatusCode::UNSUPPORTED_MEDIA_TYPE
            );
        }
    }

    #[test]
    fn invalid_or_noncanonical_media_maps_to_422() {
        for error in [
            MediaError::InvalidImage,
            MediaError::InvalidVideo,
            MediaError::MetadataForbidden,
            MediaError::MoovNotAtFront,
            MediaError::DurationTooLong,
            MediaError::ResolutionTooHigh,
        ] {
            assert_eq!(
                error.into_response().status(),
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }
    }
}
