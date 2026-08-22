//! Signed, in-band accounting for observer events lost before relay archival.

use crate::observer;
use buzz_core::observer::{encrypt_observer_payload, OBSERVER_FRAME_TELEMETRY};
use nostr::{Event, Keys, PublicKey};

pub(crate) const OBSERVER_TELEMETRY_GAP_KIND: &str = "observer_telemetry_gap";

#[derive(Clone, Copy, Debug)]
pub(crate) enum ObserverGapReason {
    ReplayBufferOverflow,
    PublishQueueEviction,
    BroadcastLag,
    PublishFailure,
    RelayQueueEviction,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ObserverGapCounts {
    replay_buffer_overflow: u64,
    publish_queue_eviction: u64,
    broadcast_lag: u64,
    publish_failure: u64,
    relay_queue_eviction: u64,
    first_observed_at: Option<String>,
    last_observed_at: Option<String>,
}

pub(crate) struct PendingObserverFrame {
    pub(crate) event: observer::ObserverEvent,
    pub(crate) source_events: u64,
    pub(crate) reported_gaps: ObserverGapCounts,
}

impl ObserverGapCounts {
    pub(crate) fn record(&mut self, reason: ObserverGapReason, count: u64) {
        if count == 0 {
            return;
        }
        let timestamp = chrono::Utc::now().to_rfc3339();
        self.first_observed_at
            .get_or_insert_with(|| timestamp.clone());
        self.last_observed_at = Some(timestamp);
        let bucket = match reason {
            ObserverGapReason::ReplayBufferOverflow => &mut self.replay_buffer_overflow,
            ObserverGapReason::PublishQueueEviction => &mut self.publish_queue_eviction,
            ObserverGapReason::BroadcastLag => &mut self.broadcast_lag,
            ObserverGapReason::PublishFailure => &mut self.publish_failure,
            ObserverGapReason::RelayQueueEviction => &mut self.relay_queue_eviction,
        };
        *bucket = bucket.saturating_add(count);
    }

    pub(crate) fn merge(&mut self, other: Self) {
        if other.is_empty() {
            return;
        }
        if self.first_observed_at.is_none() {
            self.first_observed_at = other.first_observed_at.clone();
        }
        self.last_observed_at = other.last_observed_at.clone();
        self.replay_buffer_overflow = self
            .replay_buffer_overflow
            .saturating_add(other.replay_buffer_overflow);
        self.publish_queue_eviction = self
            .publish_queue_eviction
            .saturating_add(other.publish_queue_eviction);
        self.broadcast_lag = self.broadcast_lag.saturating_add(other.broadcast_lag);
        self.publish_failure = self.publish_failure.saturating_add(other.publish_failure);
        self.relay_queue_eviction = self
            .relay_queue_eviction
            .saturating_add(other.relay_queue_eviction);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.total() == 0
    }

    pub(crate) fn total(&self) -> u64 {
        self.replay_buffer_overflow
            .saturating_add(self.publish_queue_eviction)
            .saturating_add(self.broadcast_lag)
            .saturating_add(self.publish_failure)
            .saturating_add(self.relay_queue_eviction)
    }

    pub(crate) fn into_event(
        self,
        template: Option<&observer::ObserverEvent>,
    ) -> observer::ObserverEvent {
        let total = self.total();
        observer::ObserverEvent {
            seq: template.map_or(0, |event| event.seq),
            timestamp: chrono::Utc::now().to_rfc3339(),
            kind: OBSERVER_TELEMETRY_GAP_KIND.to_string(),
            agent_index: template.and_then(|event| event.agent_index),
            channel_id: template.and_then(|event| event.channel_id.clone()),
            session_id: None,
            turn_id: None,
            started_at: None,
            payload: serde_json::json!({
                "droppedEvents": total,
                "reasonCounts": {
                    "replayBufferOverflow": self.replay_buffer_overflow,
                    "publishQueueEviction": self.publish_queue_eviction,
                    "broadcastLag": self.broadcast_lag,
                    "publishFailure": self.publish_failure,
                    "relayQueueEviction": self.relay_queue_eviction,
                },
                "firstObservedAt": self.first_observed_at,
                "lastObservedAt": self.last_observed_at,
                "scope": "publisher_global",
            }),
        }
    }
}

fn observer_owner(event: &Event) -> Result<PublicKey, String> {
    let owner = event
        .tags
        .iter()
        .find_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some("p"))
                .then(|| parts.get(1))
                .flatten()
        })
        .ok_or("observer frame is missing owner p tag")?;
    PublicKey::from_hex(owner).map_err(|error| format!("invalid observer owner: {error}"))
}

/// Count decrypted observer events represented by one signed relay frame.
pub(crate) fn represented_event_count(keys: Option<&Keys>, event: &Event) -> u64 {
    let Some(keys) = keys else { return 1 };
    let Ok(owner) = observer_owner(event) else {
        return 1;
    };
    let Ok(plaintext) =
        nostr::nips::nip44::decrypt(keys.secret_key(), &owner, event.content.as_str())
    else {
        return 1;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&plaintext) else {
        return 1;
    };
    value
        .get("payload")
        .and_then(|payload| payload.get("events"))
        .and_then(serde_json::Value::as_array)
        .map_or(1, |events| events.len().max(1) as u64)
}

/// Build a replacement gap frame using the dropped frame's owner scope.
pub(crate) fn signed_relay_gap(
    keys: &Keys,
    template: &Event,
    dropped_events: u64,
) -> Result<Event, String> {
    let owner = observer_owner(template)?;
    let mut gaps = ObserverGapCounts::default();
    gaps.record(ObserverGapReason::RelayQueueEviction, dropped_events);
    let payload = gaps.into_event(None);
    let encrypted = encrypt_observer_payload(keys, &owner, &payload)
        .map_err(|error| format!("encrypt relay gap: {error}"))?;
    buzz_sdk::build_agent_observer_frame(
        &owner.to_hex(),
        &keys.public_key().to_hex(),
        OBSERVER_FRAME_TELEMETRY,
        &encrypted,
    )
    .map_err(|error| format!("build relay gap: {error}"))?
    .sign_with_keys(keys)
    .map_err(|error| format!("sign relay gap: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gap_counts_merge_and_build_a_frame_payload() {
        let mut gaps = ObserverGapCounts::default();
        gaps.record(ObserverGapReason::BroadcastLag, 2);
        let mut later = ObserverGapCounts::default();
        later.record(ObserverGapReason::PublishFailure, 3);
        gaps.merge(later);

        let event = gaps.into_event(None);
        assert_eq!(event.kind, OBSERVER_TELEMETRY_GAP_KIND);
        assert_eq!(event.payload["droppedEvents"], 5);
        assert_eq!(event.payload["reasonCounts"]["broadcastLag"], 2);
        assert_eq!(event.payload["reasonCounts"]["publishFailure"], 3);
        assert_eq!(event.payload["scope"], "publisher_global");
    }

    #[test]
    fn relay_gap_counts_batch_members_and_remains_owner_decryptable() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let batch = serde_json::json!({
            "seq": 2,
            "timestamp": "2026-08-22T12:00:00Z",
            "kind": "batch",
            "payload": {"events": [{"seq": 1}, {"seq": 2}]},
        });
        let encrypted = encrypt_observer_payload(&agent, &owner.public_key(), &batch).unwrap();
        let template = buzz_sdk::build_agent_observer_frame(
            &owner.public_key().to_hex(),
            &agent.public_key().to_hex(),
            OBSERVER_FRAME_TELEMETRY,
            &encrypted,
        )
        .unwrap()
        .sign_with_keys(&agent)
        .unwrap();

        assert_eq!(represented_event_count(Some(&agent), &template), 2);
        let gap = signed_relay_gap(&agent, &template, 2).unwrap();
        let payload: serde_json::Value =
            buzz_core::observer::decrypt_observer_payload(&owner, &gap).unwrap();
        assert_eq!(payload["kind"], OBSERVER_TELEMETRY_GAP_KIND);
        assert_eq!(payload["payload"]["droppedEvents"], 2);
        assert_eq!(payload["payload"]["reasonCounts"]["relayQueueEviction"], 2);
    }
}
