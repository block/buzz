#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-sdk` — typed Nostr event builders for Buzz operations.
//!
//! # Mental Model
//!
//! ```text
//! caller params → builder fn → validates → EventBuilder → caller signs → Event
//! ```
//!
//! Each builder function validates its inputs and returns an [`nostr::EventBuilder`].
//! The caller signs with their own keys: `builder.sign_with_keys(&keys)?`.
//! No keys are held here. No network calls are made.

pub mod builders;
pub mod mentions;
pub mod nip_oa;

pub use builders::*;

/// Re-export kind constants so consumers don't need buzz-core directly.
pub use buzz_core::kind;

/// Thread reference for reply builders (NIP-10 markers).
///
/// - Direct reply (root == parent): emits `["e", root, "", "reply"]`
/// - Nested reply (root ≠ parent): emits `["e", root, "", "root"]` + `["e", parent, "", "reply"]`
pub struct ThreadRef {
    /// The root event of the thread.
    pub root_event_id: nostr::EventId,
    /// The immediate parent being replied to.
    pub parent_event_id: nostr::EventId,
}

/// Metadata for diff/patch messages (kind 40008).
pub struct DiffMeta {
    /// Repository URL — required, must start with `http://` or `https://`.
    pub repo_url: String,
    /// Commit SHA — required, minimum 7 hex characters.
    pub commit_sha: String,
    /// Optional file path within the repository.
    pub file_path: Option<String>,
    /// Optional parent commit SHA — minimum 7 hex chars if present.
    pub parent_commit: Option<String>,
    /// Optional branch pair `(source, target)` — both or neither.
    pub branch: Option<(String, String)>,
    /// Optional pull request number — must be positive.
    pub pr_number: Option<u32>,
    /// Optional programming language identifier.
    pub language: Option<String>,
    /// Optional human-readable description.
    pub description: Option<String>,
    /// Whether the diff was truncated due to size.
    pub truncated: bool,
    /// Optional alt text for accessibility.
    pub alt_text: Option<String>,
}

/// Vote direction for `build_vote`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoteDirection {
    /// Upvote — content `"+"`.
    Up,
    /// Downvote — content `"-"`.
    Down,
}

/// A NIP-30 custom emoji tag payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomEmoji {
    /// The shortcode without surrounding colons.
    pub shortcode: String,
    /// Image URL for this custom emoji.
    pub url: String,
}

/// Channel type.
pub use buzz_core::channel::ChannelType as ChannelKind;
/// Channel visibility.
pub use buzz_core::channel::ChannelVisibility as Visibility;
/// Member role.
pub use buzz_core::channel::MemberRole;

/// Errors returned by SDK builder functions.
#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    /// Content exceeds the maximum allowed size.
    #[error("content exceeds maximum size of {max} bytes (got {got})")]
    ContentTooLarge {
        /// Maximum allowed bytes.
        max: usize,
        /// Actual byte count.
        got: usize,
    },
    /// A tag could not be constructed.
    #[error("invalid tag: {0}")]
    InvalidTag(String),
    /// Emoji string exceeds 64 characters.
    #[error("emoji exceeds maximum length of 64 characters")]
    EmojiTooLong,
    /// More than 50 mentions were supplied.
    #[error("too many mentions (max 50)")]
    TooManyMentions,
    /// Diff metadata failed validation.
    #[error("invalid diff metadata: {0}")]
    InvalidDiffMeta(String),
    /// Input failed validation (e.g. malformed pubkey).
    #[error("invalid input: {0}")]
    InvalidInput(String),
}
