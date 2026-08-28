//! The engine's view of a raw relay event.

use serde::{Deserialize, Serialize};

/// One signal: a raw relay event the caller fetched for a selection.
///
/// Signals are never rewritten — they are the source of truth every artifact
/// claim must be able to point back to. The caller maps relay JSON into this
/// shape; the engine only reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signal {
    /// Nostr event id (64 lowercase hex chars).
    pub id: String,
    /// Author pubkey (hex).
    pub pubkey: String,
    /// Event kind.
    pub kind: u32,
    /// Unix seconds.
    pub created_at: i64,
    /// Plain content.
    pub content: String,
    /// Channel id (`h` tag) when the event is channel-scoped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}
