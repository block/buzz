//! Error type for image sanitization and metadata validation.

/// Failures from the shared image pipeline.
///
/// Deliberately narrow: every variant here maps to the same HTTP status
/// (422) on the relay side, so callers that translate this into a transport
/// error cannot accidentally change a status code by collapsing variants.
/// Size caps and the megapixel ceiling stay in `buzz-media` precisely because
/// they map to 413 — see `MediaError::ImageTooLarge`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ImageError {
    #[error("invalid image data")]
    InvalidImage,
    // This string is duplicated by `MediaError::MetadataForbidden`, which owns
    // the HTTP wire contract. `buzz-media`'s `metadata_message_matches_across_crates`
    // pins the two equal — change one and that test fails. Do not "tidy" either
    // away; they serve different consumers.
    #[error("media contains metadata or a non-canonical metadata channel")]
    MetadataForbidden,
}
