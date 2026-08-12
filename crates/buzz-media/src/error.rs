//! Media error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

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
    // Mirrors `buzz_image::ImageError::MetadataForbidden`; kept here because
    // this enum is the HTTP wire contract that relay code matches on. The two
    // are pinned equal by `metadata_message_matches_across_crates` below.
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

/// Image pipeline failures map onto the existing media variants rather than a
/// new wrapper variant.
///
/// Deliberate: `MediaError`'s variants carry the HTTP status mapping, and the
/// image errors are not all the same status as each other's neighbours —
/// `ImageTooLarge` is 413 while these are 422. Mapping variant-to-variant
/// keeps every status code exactly where it was; a single collapsed
/// `Image(_) => 422` arm would silently downgrade the megapixel-bomb check.
impl From<buzz_image::ImageError> for MediaError {
    fn from(e: buzz_image::ImageError) -> Self {
        match e {
            buzz_image::ImageError::InvalidImage => Self::InvalidImage,
            buzz_image::ImageError::MetadataForbidden => Self::MetadataForbidden,
        }
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

    /// Pin every status code that crosses the `buzz-image` boundary.
    ///
    /// The image pipeline moved into a leaf crate; this asserts the move did
    /// not renumber anything on the wire. `ImageTooLarge` is included
    /// deliberately even though it does not cross the boundary — it is 413
    /// while its neighbours are 422, so it is exactly what a future
    /// "just collapse these into one arm" refactor would break.
    #[test]
    fn image_errors_keep_their_status_codes_across_the_crate_boundary() {
        assert_eq!(
            MediaError::from(buzz_image::ImageError::MetadataForbidden)
                .into_response()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
        assert_eq!(
            MediaError::from(buzz_image::ImageError::InvalidImage)
                .into_response()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
        assert_eq!(
            MediaError::ImageTooLarge.into_response().status(),
            StatusCode::PAYLOAD_TOO_LARGE,
            "the megapixel ceiling is 413, not 422",
        );
    }

    /// The 422 body text is user-visible. Both crates spell it, so pin them
    /// equal rather than trusting two string literals to stay in step.
    #[test]
    fn metadata_message_matches_across_crates() {
        assert_eq!(
            MediaError::from(buzz_image::ImageError::MetadataForbidden).to_string(),
            buzz_image::ImageError::MetadataForbidden.to_string(),
        );
        assert_eq!(
            MediaError::from(buzz_image::ImageError::InvalidImage).to_string(),
            buzz_image::ImageError::InvalidImage.to_string(),
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
