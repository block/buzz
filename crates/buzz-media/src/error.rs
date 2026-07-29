//! Media error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Errors from media operations.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// The uploaded bytes did not match any known magic-number signature.
    #[error("unknown content type")]
    UnknownContentType,
    /// The detected content type is not on the allowlist.
    #[error("disallowed content type: {0}")]
    DisallowedContentType(String),
    /// The uploaded file exceeds the configured maximum size.
    #[error("file too large: {size} bytes (max {max})")]
    FileTooLarge {
        /// Actual size of the uploaded file in bytes.
        size: u64,
        /// Maximum allowed size in bytes.
        max: u64,
    },
    /// The decoded image exceeds the configured pixel-dimension cap.
    #[error("image dimensions too large")]
    ImageTooLarge,
    /// The image bytes could not be decoded.
    #[error("invalid image data")]
    InvalidImage,
    /// The media embeds forbidden metadata or a non-canonical metadata channel.
    #[error("media contains metadata or a non-canonical metadata channel")]
    MetadataForbidden,
    /// The provided signature failed verification.
    #[error("invalid signature")]
    InvalidSignature,
    /// The auth event has the wrong Nostr kind.
    #[error("invalid auth event kind")]
    InvalidAuthKind,
    /// The auth event carries an unsupported Blossom verb.
    #[error("invalid auth verb")]
    InvalidAuthVerb,
    /// A required Nostr tag is absent from the auth event.
    #[error("missing required tag: {0}")]
    MissingTag(&'static str),
    /// The computed SHA-256 does not match the claimed hash.
    #[error("hash mismatch")]
    HashMismatch,
    /// The auth event references a different server URL.
    #[error("server mismatch")]
    ServerMismatch,
    /// The auth event's expiration timestamp has passed.
    #[error("token expired")]
    TokenExpired,
    /// The auth event's `created_at` falls outside the acceptance window.
    #[error("timestamp out of window")]
    TimestampOutOfWindow,
    /// An underlying storage backend (S3) operation failed.
    #[error("storage error: {0}")]
    StorageError(String),
    /// A generic internal error not covered by a more specific variant.
    #[error("internal error")]
    Internal,
    /// The requested blob does not exist in storage.
    #[error("not found")]
    NotFound,
    /// The request carried no authorization header.
    #[error("missing authorization header")]
    MissingAuth,
    /// The authorization scheme is not supported.
    #[error("invalid authorization scheme")]
    InvalidAuthScheme,
    /// The authorization header contained invalid base64.
    #[error("invalid base64 encoding")]
    InvalidBase64,
    /// The decoded authorization event is malformed.
    #[error("invalid auth event")]
    InvalidAuthEvent,
    /// The authenticated principal is not permitted to perform this action.
    #[error("unauthorized")]
    Unauthorized,
    /// The auth event lacks the required Blossom scope for this operation.
    #[error("insufficient scope")]
    InsufficientScope,
    /// The pubkey is not a member of the required relay.
    #[error("relay membership required")]
    RelayMembershipRequired,
    /// The auth token has been revoked.
    #[error("token revoked")]
    TokenRevoked,
    /// The auth event's pubkey does not match the requested resource's owner.
    #[error("pubkey mismatch")]
    PubkeyMismatch,
    /// The uploader has exceeded the per-window upload rate limit.
    #[error("upload rate limit exceeded")]
    UploadRateLimitExceeded,
    /// The uploader has reached the concurrent-upload limit.
    #[error("upload concurrency limit reached")]
    UploadConcurrencyLimitReached,
    /// A video/audio track does not use the canonical H.264/AAC codecs.
    #[error("unsupported media codec: only H.264 video and AAC audio are accepted")]
    WrongCodec,
    /// Video duration exceeds the 600-second limit.
    #[error("video too long: duration exceeds 600 seconds")]
    DurationTooLong,
    /// Video resolution exceeds 3840×2160.
    #[error("video resolution too high: maximum is 3840x2160")]
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
            // All authentication failures return the same generic 401 to prevent oracle enumeration.
            // InsufficientScope is intentionally 403 — it's an authorization (not authentication)
            // failure and is safe to distinguish since it requires a valid identity first.
            Self::MissingAuth
            | Self::InvalidAuthScheme
            | Self::InvalidBase64
            | Self::InvalidAuthEvent
            | Self::InvalidSignature
            | Self::InvalidAuthKind
            | Self::InvalidAuthVerb
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
            Self::RelayMembershipRequired => (StatusCode::FORBIDDEN, self.to_string()),
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
