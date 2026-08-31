//! Shared daemon status: connection, subscription, and backfill state.
//!
//! The sync loop writes; the HTTP API reads. This is the surface an external
//! client (the future standalone UI) polls to "see the machinery".

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Connection lifecycle, coarse enough to render as a chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionState {
    /// Dialing / authenticating.
    Connecting,
    /// NIP-42 authenticated; subscriptions being established or live.
    Connected,
    /// Lost the socket; waiting out the backoff before redialing.
    Backoff,
}

/// Per-channel sync progress.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ChannelSync {
    /// Channel display name, when metadata resolved.
    pub name: Option<String>,
    /// `stream` | `private` | `dm` | `unknown`.
    pub channel_type: String,
    /// `pending` | `paging` | `done`.
    pub backfill: String,
    /// Pages fetched so far during backfill.
    pub pages: u64,
    /// Events stored for this channel (mirror count).
    pub events: i64,
    /// Newest stored event timestamp.
    pub newest_ts: Option<i64>,
    /// Whether a live subscription is currently open for this channel.
    pub live: bool,
}

/// Full daemon status snapshot (serialized verbatim as `GET /status`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusSnapshot {
    /// Relay URL in use (pinned unless explicitly overridden).
    pub relay: String,
    /// Identity in use, hex.
    pub pubkey: String,
    /// Unix seconds the daemon started.
    pub started_at: i64,
    /// Current connection state.
    pub connection: ConnectionState,
    /// Unix seconds of the last successful (re)connect, if any.
    pub connected_at: Option<i64>,
    /// Consecutive reconnect attempts since the last healthy connection.
    pub reconnect_attempts: u32,
    /// Human-readable last connection error, if any.
    pub last_error: Option<String>,
    /// Unix seconds of the most recent event received on any subscription.
    pub last_event_at: Option<i64>,
    /// Whether initial backfill of every discovered channel has completed.
    pub backfill_complete: bool,
    /// Per-channel sync progress, keyed by channel UUID.
    pub channels: BTreeMap<String, ChannelSync>,
    /// Total events in the local mirror.
    pub total_events: i64,
    /// Fold specs defined.
    pub folds: i64,
    /// Artifact versions persisted.
    pub artifacts: i64,
    /// Absolute path of the SQLite mirror.
    pub db_path: String,
}

#[derive(Debug)]
struct Inner {
    relay: String,
    pubkey: String,
    started_at: i64,
    connection: ConnectionState,
    connected_at: Option<i64>,
    reconnect_attempts: u32,
    last_error: Option<String>,
    last_event_at: Option<i64>,
    channels: BTreeMap<String, ChannelSync>,
    db_path: String,
}

/// Cheap-to-clone shared registry.
#[derive(Clone)]
pub struct StatusRegistry(Arc<Mutex<Inner>>);

impl StatusRegistry {
    /// Creates the registry with the immutable identity facts.
    pub fn new(relay: &str, pubkey: &str, db_path: &str, started_at: i64) -> Self {
        Self(Arc::new(Mutex::new(Inner {
            relay: relay.to_string(),
            pubkey: pubkey.to_string(),
            started_at,
            connection: ConnectionState::Connecting,
            connected_at: None,
            reconnect_attempts: 0,
            last_error: None,
            last_event_at: None,
            channels: BTreeMap::new(),
            db_path: db_path.to_string(),
        })))
    }

    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        // A poisoned mutex means a panicked writer; status is diagnostic, so
        // keep serving the last coherent view rather than propagating.
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut guard)
    }

    /// Marks the connection as (re)dialing.
    pub fn connecting(&self) {
        self.with(|i| i.connection = ConnectionState::Connecting);
    }

    /// Marks the connection healthy.
    pub fn connected(&self, now: i64) {
        self.with(|i| {
            i.connection = ConnectionState::Connected;
            i.connected_at = Some(now);
            i.reconnect_attempts = 0;
            i.last_error = None;
        });
    }

    /// Marks the connection lost and records the error + attempt count.
    pub fn backoff(&self, error: &str) {
        self.with(|i| {
            i.connection = ConnectionState::Backoff;
            i.reconnect_attempts += 1;
            i.last_error = Some(error.to_string());
            for ch in i.channels.values_mut() {
                ch.live = false;
            }
        });
    }

    /// Records an event arrival timestamp.
    pub fn saw_event(&self, now: i64) {
        self.with(|i| i.last_event_at = Some(now));
    }

    /// Updates (or creates) a channel's sync row.
    pub fn channel(&self, id: &str, f: impl FnOnce(&mut ChannelSync)) {
        self.with(|i| f(i.channels.entry(id.to_string()).or_default()));
    }

    /// Removes live flags / marks a channel dropped after access revocation.
    pub fn channel_revoked(&self, id: &str) {
        self.with(|i| {
            if let Some(ch) = i.channels.get_mut(id) {
                ch.live = false;
                ch.backfill = "revoked".into();
            }
        });
    }

    /// Produces the HTTP-facing snapshot, merging in store counts.
    pub fn snapshot(
        &self,
        total_events: i64,
        per_channel: &BTreeMap<String, i64>,
        folds: i64,
        artifacts: i64,
    ) -> StatusSnapshot {
        self.with(|i| {
            let mut channels = i.channels.clone();
            for (id, n) in per_channel {
                if let Some(ch) = channels.get_mut(id) {
                    ch.events = *n;
                }
            }
            let backfill_complete = !channels.is_empty()
                && channels
                    .values()
                    .all(|c| c.backfill == "done" || c.backfill == "revoked");
            StatusSnapshot {
                relay: i.relay.clone(),
                pubkey: i.pubkey.clone(),
                started_at: i.started_at,
                connection: i.connection,
                connected_at: i.connected_at,
                reconnect_attempts: i.reconnect_attempts,
                last_error: i.last_error.clone(),
                last_event_at: i.last_event_at,
                backfill_complete,
                channels,
                total_events,
                folds,
                artifacts,
                db_path: i.db_path.clone(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_reaches_snapshot() {
        let reg = StatusRegistry::new("wss://relay", "ab", "/tmp/db", 10);
        reg.connecting();
        reg.connected(11);
        reg.channel("ch1", |c| {
            c.backfill = "done".into();
            c.live = true;
        });
        reg.saw_event(12);
        let snap = reg.snapshot(5, &[("ch1".to_string(), 5)].into_iter().collect(), 1, 2);
        assert_eq!(snap.connection, ConnectionState::Connected);
        assert!(snap.backfill_complete);
        assert_eq!(snap.channels["ch1"].events, 5);
        assert_eq!(snap.total_events, 5);
        assert_eq!(snap.last_event_at, Some(12));
    }

    #[test]
    fn backoff_clears_live_and_counts_attempts() {
        let reg = StatusRegistry::new("wss://relay", "ab", "/tmp/db", 10);
        reg.channel("ch1", |c| c.live = true);
        reg.backoff("socket closed");
        reg.backoff("socket closed");
        let snap = reg.snapshot(0, &BTreeMap::new(), 0, 0);
        assert_eq!(snap.connection, ConnectionState::Backoff);
        assert_eq!(snap.reconnect_attempts, 2);
        assert!(!snap.channels["ch1"].live);
        assert!(!snap.backfill_complete, "no channels done yet");
    }
}
