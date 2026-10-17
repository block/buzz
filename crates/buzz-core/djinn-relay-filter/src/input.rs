//! Data-only projection of the structural relay matcher inputs.
//! Missing and present-empty constraints remain distinct. No sorting required.

/// A generic tag selector, with exact byte spelling and OR-ed values.
#[derive(Clone, Debug)]
pub struct TagQuery {
    /// Exact first-string bytes; case is preserved.
    pub key: Vec<u8>,
    /// Alternative second-string values, without normalization.
    pub values: Vec<Vec<u8>>,
}

/// One filter. All its fields and generic tag selectors are AND-ed.
#[derive(Clone, Debug)]
pub struct FilterInput {
    /// Exact event IDs; None is unrestricted and Some(empty) rejects.
    pub ids: Option<Vec<[u8; 32]>>,
    /// Exact author keys, preserving absent versus present-empty.
    pub authors: Option<Vec<[u8; 32]>>,
    /// Allowed unsigned 16-bit kinds.
    pub kinds: Option<Vec<u16>>,
    /// Inclusive lower timestamp bound.
    pub since: Option<u64>,
    /// Inclusive upper timestamp bound.
    pub until: Option<u64>,
    /// All generic tag constraints must hold.
    pub tags: Vec<TagQuery>,
}

/// Only the first and optional second strings of a Nostr tag matter here.
#[derive(Clone, Debug)]
pub struct EventTag {
    /// Exact first-string bytes; case is preserved.
    pub key: Vec<u8>,
    /// Optional second string; missing differs from an empty string.
    pub value: Option<Vec<u8>>,
}

/// Event data used by matching, excluding signature/authentication metadata.
#[derive(Clone, Debug)]
pub struct EventInput {
    /// Exact event ID bytes.
    pub id: [u8; 32],
    /// Exact author bytes.
    pub author: [u8; 32],
    /// Unsigned event kind.
    pub kind: u16,
    /// Unsigned event timestamp in seconds.
    pub created_at: u64,
    /// Raw tag keys and optional second values; later strings are irrelevant.
    pub tags: Vec<EventTag>,
    /// Canonical stored UUID bytes, supplied by the data-only adapter.
    pub channel: Option<Vec<u8>>,
}
