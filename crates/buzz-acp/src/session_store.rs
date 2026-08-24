//! Durable session-binding and processed-event store.
//!
//! This module is the public seam for restart-safe ACP session reuse and
//! at-least-once event dedupe. The store holds **IDs and timestamps only** —
//! never prompts, keys, or room history.
//!
//! # Semantics
//!
//! - Bindings **survive** agent-subprocess exit, panic recovery, and harness
//!   shutdown. They are the restart payload. In-memory
//!   [`SessionState::invalidate_*`](crate::pool::SessionState) does **not**
//!   imply store deletion.
//! - Bindings are **deleted** only when a session is deliberately retired or
//!   proven invalid: rotation (`MaxTokens` / `MaxTurnRequests` /
//!   `max_turns_per_session`), `ControlSignal::Rotate` / `SwitchModel`,
//!   channel membership removal, `session/load` failure, a malformed row, or
//!   a for-cause [`SessionState::invalidate`](crate::pool::SessionState)
//!   (prompt / idle-timeout / cancel-cleanup errors).
//! - Processed-event marking happens only after a successful turn, so
//!   duplicate relay event IDs produce one reply. A crash mid-turn
//!   re-processes (at-least-once).
//!
//! # Multi-worker invariant
//!
//! The store is process-global. In-memory session maps are per-worker.
//! Bindings are keyed by worker slot (`{index}:{context}`) so two adapter
//! subprocesses never attach the same session id. A fresh channel-session
//! create retires every binding for that channel (including sibling-worker
//! keys) before saving, so a later restart cannot restore a superseded
//! history. `EventQueue`'s `in_flight_channels` gate still ensures one
//! channel is processed by one worker at a time.
//!
//! # Adapters
//!
//! [`InMemorySessionStore`] is for tests and ephemeral runs.
//! [`sqlite::SqliteSessionStore`] is the opt-in production adapter.

pub mod sqlite;

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use uuid::Uuid;

/// Scope pinning a store handle to one agent identity. Set once at open;
/// rows are keyed by it so one DB file is safe to share across agents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreScope {
    /// 64-hex agent pubkey, lowercased.
    pub agent_pubkey: String,
    /// Relay URL, trimmed, with a trailing `/` stripped.
    pub relay_url: String,
    /// Adapter identity from `normalize_agent_command_identity`.
    pub adapter: String,
}

impl StoreScope {
    /// Normalize identity fields used as SQLite composite-key members.
    pub fn new(
        agent_pubkey: impl Into<String>,
        relay_url: impl Into<String>,
        adapter: impl Into<String>,
    ) -> Self {
        Self {
            agent_pubkey: agent_pubkey.into().to_ascii_lowercase(),
            relay_url: relay_url.into().trim().trim_end_matches('/').to_string(),
            adapter: adapter.into(),
        }
    }
}

/// What a session is bound to. Wire format: channel UUID string or `"heartbeat"`.
/// Worker-scoped keys (from [`ContextKey::for_worker`]) use `{index}:{inner}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ContextKey {
    /// A NIP-29 channel.
    Channel(Uuid),
    /// The harness heartbeat session.
    Heartbeat,
    /// Opaque pre-formatted wire key, including worker-scoped forms.
    Wire(String),
}

impl ContextKey {
    /// Wire form stored in SQLite (`context_key`).
    pub fn as_wire(&self) -> String {
        match self {
            Self::Channel(id) => id.to_string(),
            Self::Heartbeat => "heartbeat".to_string(),
            Self::Wire(raw) => raw.clone(),
        }
    }

    /// Scope this binding to one pool worker slot.
    ///
    /// Two workers must never `session/load` the same adapter session into
    /// two subprocesses. Wire form: `{worker_index}:{inner}`.
    pub fn for_worker(&self, worker_index: usize) -> Self {
        Self::Wire(format!("{worker_index}:{}", self.as_wire()))
    }
}

pub(crate) fn context_key_matches_channel(wire: &str, channel_id: Uuid) -> bool {
    let id = channel_id.to_string();
    wire == id || wire.ends_with(&format!(":{id}"))
}

/// A durable binding: IDs and timestamps only. No prompts, keys, or history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBinding {
    /// Adapter session identifier.
    pub session_id: String,
    /// Unix seconds when the binding was first written.
    pub created_at: i64,
    /// Unix seconds when the binding was last saved or touched.
    pub last_used_at: i64,
}

/// Failures from a session store. Callers log and continue — a broken store
/// must never take the harness down.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SessionStoreError {
    /// Filesystem, lock, or spawn failure.
    #[error("session store I/O: {0}")]
    Io(String),
    /// Schema or row that cannot be interpreted safely.
    #[error("session store corrupt: {0}")]
    Corrupt(String),
}

/// Durable session-binding + processed-event store.
///
/// All methods are best-effort from the harness's perspective: callers log
/// and continue on error.
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Load the binding for `key`, if any.
    async fn load_binding(
        &self,
        key: &ContextKey,
    ) -> Result<Option<SessionBinding>, SessionStoreError>;

    /// Insert or replace the binding for `key`.
    async fn save_binding(
        &self,
        key: &ContextKey,
        session_id: &str,
    ) -> Result<(), SessionStoreError>;

    /// Refresh `last_used_at` without changing `session_id`.
    async fn touch_binding(&self, key: &ContextKey) -> Result<(), SessionStoreError>;

    /// Delete the binding for `key`. Missing rows are success.
    async fn remove_binding(&self, key: &ContextKey) -> Result<(), SessionStoreError>;

    /// Delete every binding for `channel_id`, including worker-scoped keys.
    ///
    /// Used when a channel is retired for all workers (membership removal,
    /// idle `!rotate`). Missing rows are success.
    async fn remove_bindings_for_channel(&self, channel_id: Uuid) -> Result<(), SessionStoreError>;

    /// Whether `event_id` was marked processed after a successful turn.
    async fn is_event_processed(&self, event_id: &str) -> Result<bool, SessionStoreError>;

    /// Record that `event_ids` were delivered on `channel_id`.
    async fn mark_events_processed(
        &self,
        channel_id: Uuid,
        event_ids: &[String],
    ) -> Result<(), SessionStoreError>;

    /// Event IDs previously marked processed for `channel_id`, so a restored
    /// session can re-seed its in-memory delivered set.
    async fn processed_event_ids_for_channel(
        &self,
        channel_id: Uuid,
    ) -> Result<Vec<String>, SessionStoreError>;
}

/// In-memory adapter for tests and ephemeral runs. Bindings do not survive
/// process restart.
#[derive(Debug, Default)]
pub struct InMemorySessionStore {
    inner: Mutex<InMemoryInner>,
}

#[derive(Debug, Default)]
struct InMemoryInner {
    bindings: HashMap<String, SessionBinding>,
    events: HashMap<String, (Uuid, i64)>,
}

impl InMemorySessionStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }

    fn now_secs() -> i64 {
        unix_now_secs()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, InMemoryInner>, SessionStoreError> {
        self.inner
            .lock()
            .map_err(|e| SessionStoreError::Io(format!("in-memory store poisoned: {e}")))
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn load_binding(
        &self,
        key: &ContextKey,
    ) -> Result<Option<SessionBinding>, SessionStoreError> {
        let inner = self.lock()?;
        Ok(inner.bindings.get(&key.as_wire()).cloned())
    }

    async fn save_binding(
        &self,
        key: &ContextKey,
        session_id: &str,
    ) -> Result<(), SessionStoreError> {
        let now = Self::now_secs();
        let mut inner = self.lock()?;
        let wire = key.as_wire();
        let created_at = inner
            .bindings
            .get(&wire)
            .map(|b| b.created_at)
            .unwrap_or(now);
        inner.bindings.insert(
            wire,
            SessionBinding {
                session_id: session_id.to_string(),
                created_at,
                last_used_at: now,
            },
        );
        Ok(())
    }

    async fn touch_binding(&self, key: &ContextKey) -> Result<(), SessionStoreError> {
        let now = Self::now_secs();
        let mut inner = self.lock()?;
        if let Some(binding) = inner.bindings.get_mut(&key.as_wire()) {
            binding.last_used_at = now;
        }
        Ok(())
    }

    async fn remove_binding(&self, key: &ContextKey) -> Result<(), SessionStoreError> {
        self.lock()?.bindings.remove(&key.as_wire());
        Ok(())
    }

    async fn remove_bindings_for_channel(&self, channel_id: Uuid) -> Result<(), SessionStoreError> {
        let mut inner = self.lock()?;
        inner
            .bindings
            .retain(|wire, _| !context_key_matches_channel(wire, channel_id));
        Ok(())
    }

    async fn is_event_processed(&self, event_id: &str) -> Result<bool, SessionStoreError> {
        Ok(self.lock()?.events.contains_key(event_id))
    }

    async fn mark_events_processed(
        &self,
        channel_id: Uuid,
        event_ids: &[String],
    ) -> Result<(), SessionStoreError> {
        let now = Self::now_secs();
        let mut inner = self.lock()?;
        for event_id in event_ids {
            inner.events.insert(event_id.clone(), (channel_id, now));
        }
        Ok(())
    }

    async fn processed_event_ids_for_channel(
        &self,
        channel_id: Uuid,
    ) -> Result<Vec<String>, SessionStoreError> {
        let inner = self.lock()?;
        Ok(inner
            .events
            .iter()
            .filter(|&(_, (cid, _))| *cid == channel_id)
            .map(|(id, _)| id.clone())
            .collect())
    }
}

pub(crate) fn unix_now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Best-effort processed-event gate. Store errors degrade to "not processed".
pub(crate) async fn skip_if_already_processed(
    store: Option<&dyn SessionStore>,
    event_id: &str,
) -> bool {
    let Some(store) = store else {
        return false;
    };
    match store.is_event_processed(event_id).await {
        Ok(true) => {
            tracing::debug!(event_id, "skipping already-processed event");
            true
        }
        Ok(false) => false,
        Err(error) => {
            tracing::warn!(%error, event_id, "session store processed-event check failed");
            false
        }
    }
}

#[cfg(test)]
mod session_store_in_memory_tests {
    use super::*;

    #[tokio::test]
    async fn session_store_in_memory_round_trip_and_upsert() {
        let store = InMemorySessionStore::new();
        let cid = Uuid::new_v4();
        let key = ContextKey::Channel(cid);

        assert_eq!(store.load_binding(&key).await.unwrap(), None);
        store.save_binding(&key, "ses_one").await.unwrap();
        let first = store.load_binding(&key).await.unwrap().unwrap();
        assert_eq!(first.session_id, "ses_one");

        store.save_binding(&key, "ses_two").await.unwrap();
        let second = store.load_binding(&key).await.unwrap().unwrap();
        assert_eq!(second.session_id, "ses_two");
        assert_eq!(second.created_at, first.created_at);
        assert!(second.last_used_at >= first.last_used_at);
    }

    #[tokio::test]
    async fn session_store_in_memory_processed_events_are_channel_scoped() {
        let store = InMemorySessionStore::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        store
            .mark_events_processed(a, &["evt-a".into(), "evt-shared".into()])
            .await
            .unwrap();
        store
            .mark_events_processed(b, &["evt-b".into()])
            .await
            .unwrap();

        assert!(store.is_event_processed("evt-a").await.unwrap());
        assert!(!store.is_event_processed("evt-missing").await.unwrap());
        let mut for_a = store.processed_event_ids_for_channel(a).await.unwrap();
        for_a.sort();
        assert_eq!(for_a, vec!["evt-a", "evt-shared"]);
    }

    #[tokio::test]
    async fn session_store_skip_gate_honors_marked_ids() {
        let store = InMemorySessionStore::new();
        store
            .mark_events_processed(Uuid::new_v4(), &["seen".into()])
            .await
            .unwrap();
        assert!(skip_if_already_processed(Some(&store), "seen").await);
        assert!(!skip_if_already_processed(Some(&store), "fresh").await);
        assert!(!skip_if_already_processed(None, "seen").await);
    }

    #[tokio::test]
    async fn session_store_worker_keys_and_channel_removal_are_isolated() {
        let store = InMemorySessionStore::new();
        let cid = Uuid::new_v4();
        let other = Uuid::new_v4();
        let worker0 = ContextKey::Channel(cid).for_worker(0);
        let worker1 = ContextKey::Channel(cid).for_worker(1);
        let other_key = ContextKey::Channel(other).for_worker(0);
        store.save_binding(&worker0, "ses_w0").await.unwrap();
        store.save_binding(&worker1, "ses_w1").await.unwrap();
        store.save_binding(&other_key, "ses_other").await.unwrap();

        assert_eq!(worker0.as_wire(), format!("0:{cid}"));
        assert_ne!(worker0.as_wire(), ContextKey::Channel(cid).as_wire());
        assert_eq!(
            store
                .load_binding(&worker0)
                .await
                .unwrap()
                .unwrap()
                .session_id,
            "ses_w0"
        );
        assert_eq!(
            store
                .load_binding(&worker1)
                .await
                .unwrap()
                .unwrap()
                .session_id,
            "ses_w1"
        );

        store.remove_bindings_for_channel(cid).await.unwrap();
        assert!(store.load_binding(&worker0).await.unwrap().is_none());
        assert!(store.load_binding(&worker1).await.unwrap().is_none());
        assert_eq!(
            store
                .load_binding(&other_key)
                .await
                .unwrap()
                .unwrap()
                .session_id,
            "ses_other"
        );
    }
}
