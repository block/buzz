//! Portable relay replication records and port contracts.
//!
//! Replication is durable synchronization between relay journals. It is
//! intentionally separate from live subscriptions: source adapters expose
//! ordered durable records, destination adapters apply their own policy and
//! normal ingest pipeline, and an orchestrator persists checkpoints only after
//! a checkpoint-safe receipt.

use std::future::Future;

use nostr::Event;
use serde::{Deserialize, Serialize};

/// Stable, operator-assigned identity of a replication source stream.
///
/// A source ID names both the relay and its exported scope. Community mapping
/// remains deployment policy and must not be inferred from event tags. The ID
/// is a routing label, not a credential; a transport adapter must bind it to an
/// authenticated peer rather than trusting a self-asserted network field.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReplicationSourceId(String);

impl ReplicationSourceId {
    /// Creates a source identity from an adapter-defined token.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the source identity token.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque, source-owned position in a durable replication stream.
///
/// Consumers must persist and return the token unchanged. They must not parse,
/// compare, increment, or reuse it with a different [`ReplicationSourceId`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReplicationCursor(String);

impl ReplicationCursor {
    /// Creates a cursor from an adapter-defined opaque token.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the opaque cursor token for persistence or transport.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One exact signed event exported from durable source history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicationRecord {
    /// Source stream whose policy and checkpoint namespace apply.
    pub source: ReplicationSourceId,
    /// Checkpoint that is safe to persist after this record is acknowledged.
    pub cursor: ReplicationCursor,
    /// Unmodified signed Nostr event envelope.
    pub event: Event,
}

/// A bounded, ordered page from a replication source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicationBatch {
    /// Records in source journal order.
    pub records: Vec<ReplicationRecord>,
    /// Cursor immediately after the final returned record, or the requested
    /// position when the batch is empty.
    pub next_cursor: ReplicationCursor,
    /// Whether the source had no additional durable records when it read the
    /// batch.
    pub caught_up: bool,
}

/// Destination result for one replicated event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReplicationIngestOutcome {
    /// The event entered durable destination history.
    Stored,
    /// The destination had already accepted the event ID.
    Duplicate,
    /// The event was valid but lost destination replacement ordering.
    Superseded,
    /// Destination policy or verification rejected the event.
    Rejected {
        /// Stable human-readable rejection reason.
        reason: String,
    },
}

/// A destination acknowledgement bound to the source checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicationReceipt {
    /// Source stream copied from the replication record.
    pub source: ReplicationSourceId,
    /// Source checkpoint copied from the replication record.
    pub cursor: ReplicationCursor,
    /// Hex-encoded event ID observed by the destination.
    pub event_id: String,
    /// Destination ingest result.
    pub outcome: ReplicationIngestOutcome,
}

impl ReplicationReceipt {
    /// Returns whether an orchestrator may durably advance to this receipt's
    /// cursor without silently dropping an event.
    ///
    /// Stored, duplicate, and superseded outcomes are terminal and safe.
    /// Rejections require operator policy, retry, or a durable dead-letter
    /// decision before checkpoint advancement.
    pub fn checkpoint_safe(&self) -> bool {
        !matches!(self.outcome, ReplicationIngestOutcome::Rejected { .. })
    }
}

/// Ordered durable-history source used by a replication orchestrator.
///
/// Implementations own cursor syntax and must return the same signed envelope
/// bytes semantically represented by [`Event`]. Ephemeral events must never be
/// exported.
pub trait ReplicationSourcePort {
    /// Operational error returned while reading source history.
    type Error;

    /// Reads at most `limit` records strictly after `cursor`.
    fn read_batch(
        &self,
        cursor: Option<ReplicationCursor>,
        limit: usize,
    ) -> impl Future<Output = Result<ReplicationBatch, Self::Error>>;
}

/// Policy-gated destination used by a replication orchestrator.
///
/// Implementations must not trust source acceptance. They independently apply
/// source/community policy, verify the signed envelope, and use the same
/// duplicate, replacement, durability, projection, and publication path as a
/// local submission. Network adapters must authenticate the peer and bind its
/// configured source ID before invoking this port.
pub trait ReplicationSinkPort {
    /// Operational error returned when destination ingest cannot complete.
    type Error;

    /// Attempts to ingest one source record and returns a bound receipt.
    fn ingest_replication(
        &self,
        record: ReplicationRecord,
    ) -> impl Future<Output = Result<ReplicationReceipt, Self::Error>>;
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind};

    use super::*;

    #[test]
    fn record_json_round_trip_preserves_the_signed_envelope() {
        let event = EventBuilder::new(Kind::TextNote, "portable")
            .sign_with_keys(&Keys::generate())
            .expect("test event signs");
        let record = ReplicationRecord {
            source: ReplicationSourceId::new("relay-a/community-main"),
            cursor: ReplicationCursor::new("adapter:42"),
            event,
        };

        let encoded = serde_json::to_vec(&record).expect("record serializes");
        let decoded: ReplicationRecord =
            serde_json::from_slice(&encoded).expect("record deserializes");
        assert_eq!(decoded, record);
    }

    #[test]
    fn only_terminal_destination_outcomes_are_checkpoint_safe() {
        let receipt = |outcome| ReplicationReceipt {
            source: ReplicationSourceId::new("relay-a/community-main"),
            cursor: ReplicationCursor::new("adapter:1"),
            event_id: "00".repeat(32),
            outcome,
        };

        assert!(receipt(ReplicationIngestOutcome::Stored).checkpoint_safe());
        assert!(receipt(ReplicationIngestOutcome::Duplicate).checkpoint_safe());
        assert!(receipt(ReplicationIngestOutcome::Superseded).checkpoint_safe());
        assert!(!receipt(ReplicationIngestOutcome::Rejected {
            reason: "source denied".to_string(),
        })
        .checkpoint_safe());
    }
}
