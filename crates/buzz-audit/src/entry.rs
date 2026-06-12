use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::action::AuditAction;

/// Materialised audit log entry as stored in the DB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Monotonically increasing sequence number.
    pub seq: i64,
    /// When the entry was recorded.
    pub timestamp: DateTime<Utc>,
    /// Nostr event ID that triggered this action.
    pub event_id: String,
    /// Nostr event kind number.
    pub event_kind: u32,
    /// Hex-encoded Nostr pubkey.
    pub actor_pubkey: String,
    /// Action that was performed.
    pub action: AuditAction,
    /// Channel this action applies to, if any.
    pub channel_id: Option<Uuid>,
    /// Arbitrary JSON context. **Included in hash computation** (serialized with
    /// sorted keys for determinism) so that metadata tampering is detectable.
    pub metadata: serde_json::Value,
    /// SHA-256 hex hash of the previous entry (or [`crate::hash::GENESIS_HASH`] for the first).
    pub prev_hash: String,
    /// SHA-256 hex hash of this entry's fields including `prev_hash`.
    pub hash: String,
}

/// Input for creating a new audit entry. `seq`, `prev_hash`, `hash` are computed by `AuditService::log`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewAuditEntry {
    /// Nostr event ID that triggered this action.
    pub event_id: String,
    /// Must not be 22242 (NIP-42 AUTH).
    pub event_kind: u32,
    /// Hex-encoded Nostr pubkey of the actor.
    pub actor_pubkey: String,
    /// Action that was performed.
    pub action: AuditAction,
    /// Channel this action applies to, if any.
    pub channel_id: Option<Uuid>,
    /// Arbitrary JSON context included in hash computation.
    #[serde(default)]
    pub metadata: serde_json::Value,
}
