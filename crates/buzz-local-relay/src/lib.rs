//! A small, durable Buzz relay for local experimentation.
//!
//! The local relay implements a deliberately narrow NIP-01 and Buzz HTTP
//! bridge subset. It verifies real Nostr signatures and persists durable events
//! to an append-only NDJSON log. Its optional identity adapter provides a
//! portable authorization boundary without emulating production membership,
//! media, search indexing, workflows, or multi-node fan-out.

/// Journal-derived operator configuration from sync declaration heads.
pub mod declarations;
/// Laptop NIP-42/NIP-98 authentication and authorization adapter.
pub mod identity;

use std::collections::HashMap;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{
    DefaultBodyLimit, FromRequestParts, Path as UrlPath, Request, State, WebSocketUpgrade,
};
use axum::http::{header::HOST, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use buzz_core::event::StoredEvent;
use buzz_core::filter::filters_match;
use buzz_core::identity::{
    AuthenticatedPrincipal, AuthorizationDecision, IdentityAuthenticator, IdentityDenialCode,
    Principal, ReadOperation, ReplicationPeerAuthenticator,
};
use buzz_core::ingest::{apply_effective_event, decide_event, is_ephemeral_kind, EventDecision};
use buzz_core::kind::{
    event_kind_u32, KIND_NIP29_CREATE_GROUP, KIND_NIP29_GROUP_ADMINS, KIND_NIP29_GROUP_MEMBERS,
    KIND_NIP29_GROUP_METADATA,
};
use buzz_core::replication::{
    ReplicationBatch, ReplicationCursor, ReplicationIngestOutcome, ReplicationReceipt,
    ReplicationRecord, ReplicationSinkPort, ReplicationSourceId, ReplicationSourcePort,
};
use buzz_core::verification::verify_event;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use identity::{
    LocalAuthenticationEvidence, LocalIdentityAdapter, LocalIdentityError, LocalPeerEvidence,
};
use nostr::hashes::Hash as _;
use nostr::{
    Alphabet, Event, EventBuilder, Filter, Keys, Kind, PublicKey, SingleLetterTag, Tag, TagKind,
};
use serde::Serialize;
use serde_json::{json, Value};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

const DEFAULT_QUERY_LIMIT: usize = 500;
const MAX_QUERY_LIMIT: usize = 5_000;
const EVENT_CHANNEL_CAPACITY: usize = 1_024;
const MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 128;
const MAX_SUBSCRIPTION_ID_LENGTH: usize = 128;
const MAX_REPLICATION_BATCH_SIZE: usize = 1_000;
/// Upper bound for one stored artifact blob (bytes).
const MAX_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;
const LOCAL_REPLICATION_CURSOR_PREFIX: &str = "local-ndjson-v1:";

/// Persistent or in-memory storage selection for a local relay.
#[derive(Debug, Clone)]
pub enum StorageMode {
    /// Append durable events to this newline-delimited JSON file.
    Durable(PathBuf),
    /// Keep events only until the process exits.
    Ephemeral,
}

/// The result returned after an event submission.
#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    /// Hex-encoded Nostr event ID.
    pub event_id: String,
    /// Whether the relay accepted the event.
    pub accepted: bool,
    /// Human-readable outcome.
    pub message: String,
    #[serde(skip)]
    publish_live: bool,
}

impl WriteResult {
    fn accepted(event: &Event, message: impl Into<String>, publish_live: bool) -> Self {
        Self {
            event_id: event.id.to_hex(),
            accepted: true,
            message: message.into(),
            publish_live,
        }
    }

    fn rejected(event: &Event, message: impl Into<String>) -> Self {
        Self {
            event_id: event.id.to_hex(),
            accepted: false,
            message: message.into(),
            publish_live: false,
        }
    }
}

/// Errors that prevent the local event store from operating safely.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The event-log file could not be read or written.
    #[error("event log I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// A stored line is not a valid Nostr event.
    #[error("event log line {line} is malformed: {source}")]
    MalformedRecord {
        /// One-based line number.
        line: usize,
        /// JSON decoding failure.
        source: serde_json::Error,
    },
    /// A stored line contains an invalid event ID or signature.
    #[error("event log line {line} failed verification: {reason}")]
    InvalidRecord {
        /// One-based line number.
        line: usize,
        /// Verification failure.
        reason: String,
    },
    /// Signature verification could not be scheduled.
    #[error("event verification task failed: {0}")]
    VerificationTask(String),
    /// A verified event could not be serialized for the log.
    #[error("event serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    /// Relay-authored state could not be materialized from an accepted command.
    #[error("relay state materialization failed: {0}")]
    Materialization(String),
}

#[derive(Debug, Error)]
enum Nip29ProjectionError {
    #[error("invalid kind:9007 create command: {0}")]
    InvalidCommand(String),
    #[error("failed to sign discovery state: {0}")]
    Signing(String),
}

/// Query features intentionally not implemented by the local relay.
#[derive(Debug, Error)]
pub enum QueryError {
    /// NIP-50 requires a search engine rather than core NIP-01 matching.
    #[error("NIP-50 search filters require the production relay")]
    SearchUnsupported,
    /// The filter used a field outside the supported NIP-01 subset.
    #[error("unsupported filter field: {0}")]
    UnsupportedFilterField(String),
}

/// Errors returned while reading the laptop relay's replication stream.
#[derive(Debug, Error)]
pub enum ReplicationSourceError {
    /// A zero-sized page cannot make progress.
    #[error("replication batch limit must be greater than zero")]
    ZeroBatchLimit,
    /// The cursor was not issued by this source adapter.
    #[error("invalid local replication cursor: {0}")]
    InvalidCursor(String),
    /// The cursor points beyond the current durable journal.
    #[error("replication cursor position {position} exceeds journal length {journal_len}")]
    CursorOutOfRange {
        /// Parsed source position.
        position: usize,
        /// Current number of durable source records.
        journal_len: usize,
    },
}

/// Errors that prevent a replication destination from completing ingest.
#[derive(Debug, Error)]
pub enum ReplicationSinkError {
    /// Cryptographic peer identity could not be established or admitted.
    #[error(transparent)]
    Identity(#[from] LocalIdentityError),
    /// Normal relay ingest failed operationally.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The relay returned an accepted result unknown to the portable mapping.
    #[error("unexpected accepted relay outcome: {0}")]
    UnexpectedAcceptedOutcome(String),
}

struct StoreInner {
    events: Vec<StoredEvent>,
    journal: Vec<Event>,
    seen_ids: HashSet<nostr::EventId>,
    writer: Option<tokio::fs::File>,
}

/// A verified effective event set backed by an optional append-only log.
pub struct EventStore {
    inner: Mutex<StoreInner>,
}

impl EventStore {
    /// Opens a store, verifies every durable record, and rebuilds effective state.
    pub async fn open(mode: StorageMode) -> Result<Self, StoreError> {
        let (replayed, writer) = match mode {
            StorageMode::Durable(path) => {
                let replay_path = path.clone();
                let replayed = tokio::task::spawn_blocking(move || replay_log(&replay_path))
                    .await
                    .map_err(|error| StoreError::VerificationTask(error.to_string()))??;

                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    tokio::fs::create_dir_all(parent).await?;
                }
                let writer = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .await?;
                (replayed, Some(writer))
            }
            StorageMode::Ephemeral => (ReplayedLog::default(), None),
        };

        Ok(Self {
            inner: Mutex::new(StoreInner {
                events: replayed.events,
                journal: replayed.journal,
                seen_ids: replayed.seen_ids,
                writer,
            }),
        })
    }

    /// Verifies and accepts an event, durably appending it when applicable.
    pub async fn accept(&self, event: Event) -> Result<WriteResult, StoreError> {
        let verification_event = event.clone();
        let verification = tokio::task::spawn_blocking(move || {
            verify_event(&verification_event).map_err(|e| e.to_string())
        })
        .await
        .map_err(|error| StoreError::VerificationTask(error.to_string()))?;

        if let Err(reason) = verification {
            return Ok(WriteResult::rejected(&event, format!("invalid: {reason}")));
        }

        let mut inner = self.inner.lock().await;
        match decide_event(&inner.events, inner.seen_ids.contains(&event.id), &event) {
            EventDecision::Duplicate => {
                return Ok(WriteResult::accepted(&event, "duplicate", false));
            }
            EventDecision::Ephemeral => {
                inner.seen_ids.insert(event.id);
                return Ok(WriteResult::accepted(&event, "ephemeral", true));
            }
            EventDecision::Superseded => {
                inner.seen_ids.insert(event.id);
                return Ok(WriteResult::accepted(&event, "superseded", false));
            }
            EventDecision::Stored => {}
        }

        if let Some(writer) = inner.writer.as_mut() {
            let mut record = serde_json::to_vec(&event)?;
            record.push(b'\n');
            writer.write_all(&record).await?;
            writer.flush().await?;
            writer.sync_data().await?;
        }

        let stored = stored_event(event.clone());
        apply_effective_event(&mut inner.events, stored);
        inner.journal.push(event.clone());
        inner.seen_ids.insert(event.id);

        Ok(WriteResult::accepted(&event, "stored", true))
    }

    /// Returns every durable journal event carrying an `x` tag equal to
    /// `sha256`. Scans the full journal (not just effective heads): a
    /// superseded event's artifact reference still authorizes custody, and
    /// replication serves the journal, not the effective view.
    pub async fn journal_events_referencing(&self, sha256: &str) -> Vec<Event> {
        let inner = self.inner.lock().await;
        inner
            .journal
            .iter()
            .filter(|event| {
                event.tags.iter().any(|tag| {
                    let values = tag.as_slice();
                    values.first().map(String::as_str) == Some("x")
                        && values.get(1).map(String::as_str) == Some(sha256)
                })
            })
            .cloned()
            .collect()
    }

    /// Returns matching effective events in newest-first order.
    pub async fn query(&self, filters: &[Filter]) -> Result<Vec<Event>, QueryError> {
        validate_filters(filters)?;
        if filters.is_empty() {
            return Ok(Vec::new());
        }

        let inner = self.inner.lock().await;
        let mut ordered: Vec<&StoredEvent> = inner.events.iter().collect();
        ordered.sort_by(|left, right| {
            right
                .event
                .created_at
                .cmp(&left.event.created_at)
                .then_with(|| left.event.id.to_hex().cmp(&right.event.id.to_hex()))
        });

        let mut selected_ids = HashSet::new();
        let mut matches = Vec::new();
        for filter in filters {
            let limit = filter
                .limit
                .unwrap_or(DEFAULT_QUERY_LIMIT)
                .min(MAX_QUERY_LIMIT);
            for stored in ordered
                .iter()
                .copied()
                .filter(|stored| filters_match(std::slice::from_ref(filter), stored))
                .take(limit)
            {
                if selected_ids.insert(stored.event.id) {
                    matches.push(stored.event.clone());
                }
            }
        }
        matches.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.id.to_hex().cmp(&right.id.to_hex()))
        });
        Ok(matches)
    }

    /// Counts matching effective events.
    pub async fn count(&self, filters: &[Filter]) -> Result<usize, QueryError> {
        validate_filters(filters)?;
        if filters.is_empty() {
            return Ok(0);
        }
        let inner = self.inner.lock().await;
        Ok(inner
            .events
            .iter()
            .filter(|stored| filters_match(filters, stored))
            .count())
    }
}

/// Laptop reference implementation of the ordered replication source port.
pub struct LocalReplicationSource {
    source: ReplicationSourceId,
    store: Arc<EventStore>,
    filter: Option<Vec<Filter>>,
}

impl LocalReplicationSource {
    /// Binds an operator-assigned source identity to a local event store.
    pub fn new(source: ReplicationSourceId, store: Arc<EventStore>) -> Self {
        Self {
            source,
            store,
            filter: None,
        }
    }

    /// Binds a selective stream: only journal events matching `filters`
    /// (NIP-01 OR semantics) are exported under this source identity.
    ///
    /// A stream's predicate is part of its identity. Cursors advance past
    /// non-matching records permanently, so redefining a stream's filter
    /// requires a new [`ReplicationSourceId`] and a fresh cursor; destination
    /// idempotence by event ID makes the resulting re-scan safe.
    pub fn with_filter(
        source: ReplicationSourceId,
        store: Arc<EventStore>,
        filters: Vec<Filter>,
    ) -> Self {
        Self {
            source,
            store,
            filter: Some(filters),
        }
    }

    fn exports(&self, event: &Event) -> bool {
        match self.filter.as_ref() {
            None => true,
            Some(filters) => filters_match(filters, &stored_event(event.clone())),
        }
    }
}

impl ReplicationSourcePort for LocalReplicationSource {
    type Error = ReplicationSourceError;

    async fn read_batch(
        &self,
        cursor: Option<ReplicationCursor>,
        limit: usize,
    ) -> Result<ReplicationBatch, Self::Error> {
        if limit == 0 {
            return Err(ReplicationSourceError::ZeroBatchLimit);
        }

        let start = match cursor.as_ref() {
            Some(cursor) => parse_local_replication_cursor(cursor)?,
            None => 0,
        };
        let inner = self.store.inner.lock().await;
        if start > inner.journal.len() {
            return Err(ReplicationSourceError::CursorOutOfRange {
                position: start,
                journal_len: inner.journal.len(),
            });
        }

        // The page bounds *scanned* records, not matched ones, so a filtered
        // stream makes progress through long non-matching runs; batches may
        // therefore be empty without being caught up.
        let page_size = limit.min(MAX_REPLICATION_BATCH_SIZE);
        let end = start.saturating_add(page_size).min(inner.journal.len());
        let records = inner.journal[start..end]
            .iter()
            .enumerate()
            .filter(|(_, event)| self.exports(event))
            .map(|(offset, event)| ReplicationRecord {
                source: self.source.clone(),
                cursor: local_replication_cursor(start + offset + 1),
                event: event.clone(),
            })
            .collect();
        Ok(ReplicationBatch {
            records,
            next_cursor: local_replication_cursor(end),
            caught_up: end == inner.journal.len(),
        })
    }
}

fn local_replication_cursor(position: usize) -> ReplicationCursor {
    ReplicationCursor::new(format!("{LOCAL_REPLICATION_CURSOR_PREFIX}{position}"))
}

fn parse_local_replication_cursor(
    cursor: &ReplicationCursor,
) -> Result<usize, ReplicationSourceError> {
    cursor
        .as_str()
        .strip_prefix(LOCAL_REPLICATION_CURSOR_PREFIX)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ReplicationSourceError::InvalidCursor(cursor.as_str().to_string()))
}

/// Destination admission policy for replicated events.
///
/// This source-level gate is independent of signature validity and event
/// authorship. Hosted adapters should additionally apply their normal
/// community, membership, and event-kind authorization. A future network
/// transport must authenticate its peer and bind the configured source ID
/// before invoking this application port.
pub trait ReplicationPolicy: Send + Sync {
    /// Admits or denies one source/event pair before destination mutation.
    fn admit(&self, source: &ReplicationSourceId, event: &Event) -> Result<(), String>;
}

/// Explicit replication policy that admits only configured source streams.
#[derive(Debug, Clone, Default)]
pub struct ReplicationSourceAllowlist {
    sources: HashSet<ReplicationSourceId>,
}

impl ReplicationSourceAllowlist {
    /// Builds an allowlist from operator-assigned source identities.
    pub fn new(sources: impl IntoIterator<Item = ReplicationSourceId>) -> Self {
        Self {
            sources: sources.into_iter().collect(),
        }
    }
}

impl ReplicationPolicy for ReplicationSourceAllowlist {
    fn admit(&self, source: &ReplicationSourceId, _event: &Event) -> Result<(), String> {
        if self.sources.contains(source) {
            Ok(())
        } else {
            Err(format!("replication source denied: {}", source.as_str()))
        }
    }
}

/// Default replication policy: no source stream is admitted.
pub struct ReplicationDisabled;

impl ReplicationPolicy for ReplicationDisabled {
    fn admit(&self, _source: &ReplicationSourceId, _event: &Event) -> Result<(), String> {
        Err("replication is disabled".to_string())
    }
}

/// Shared state for the HTTP and WebSocket relay surfaces.
pub struct LocalRelay {
    store: Arc<EventStore>,
    live_events: broadcast::Sender<Event>,
    replication_policy: Arc<dyn ReplicationPolicy>,
    identity: Option<Arc<LocalIdentityAdapter>>,
    artifacts_dir: Option<PathBuf>,
    /// Owner anchor + node label. When both are present (with an identity
    /// adapter), the artifact store enforces upload admission (owner or
    /// admitted peers) and the reference rule on fetch/head. Absent anchors
    /// preserve the ungoverned legacy behavior.
    governance: Option<(PublicKey, String)>,
    /// Dedicated key used only for relay-authored discovery projections.
    relay_keys: Option<Keys>,
}

impl LocalRelay {
    /// Opens a relay using the selected storage mode with replication disabled.
    pub async fn open(mode: StorageMode) -> Result<Arc<Self>, StoreError> {
        Self::open_with_adapters(mode, Arc::new(ReplicationDisabled), None).await
    }

    /// Opens a relay with an explicit source admission policy for replication.
    pub async fn open_with_replication_policy(
        mode: StorageMode,
        replication_policy: Arc<dyn ReplicationPolicy>,
    ) -> Result<Arc<Self>, StoreError> {
        Self::open_with_adapters(mode, replication_policy, None).await
    }

    /// Opens a relay requiring portable NIP-42/NIP-98 identity.
    pub async fn open_with_identity(
        mode: StorageMode,
        identity: Arc<LocalIdentityAdapter>,
    ) -> Result<Arc<Self>, StoreError> {
        Self::open_with_adapters(mode, Arc::new(ReplicationDisabled), Some(identity)).await
    }

    /// Opens a relay with explicit replication and identity adapters.
    pub async fn open_with_adapters(
        mode: StorageMode,
        replication_policy: Arc<dyn ReplicationPolicy>,
        identity: Option<Arc<LocalIdentityAdapter>>,
    ) -> Result<Arc<Self>, StoreError> {
        Self::open_full(mode, replication_policy, identity, None).await
    }

    /// Opens a relay with adapters and an optional content-addressed
    /// artifact store rooted at `artifacts_dir`.
    pub async fn open_full(
        mode: StorageMode,
        replication_policy: Arc<dyn ReplicationPolicy>,
        identity: Option<Arc<LocalIdentityAdapter>>,
        artifacts_dir: Option<PathBuf>,
    ) -> Result<Arc<Self>, StoreError> {
        let store = Arc::new(EventStore::open(mode).await?);
        Ok(Self::open_full_with_store(
            store,
            replication_policy,
            identity,
            artifacts_dir,
        ))
    }

    /// Builds a relay around an already-opened store.
    ///
    /// Lets callers evaluate journal state (for example, declaration-derived
    /// trust) before constructing the adapters that depend on it.
    pub fn open_full_with_store(
        store: Arc<EventStore>,
        replication_policy: Arc<dyn ReplicationPolicy>,
        identity: Option<Arc<LocalIdentityAdapter>>,
        artifacts_dir: Option<PathBuf>,
    ) -> Arc<Self> {
        Self::open_governed(store, replication_policy, identity, artifacts_dir, None)
    }

    /// Builds a relay with an owner anchor and node label, activating
    /// declaration-governed artifact access (upload admission and the
    /// reference rule) alongside journal-derived trust.
    pub fn open_governed(
        store: Arc<EventStore>,
        replication_policy: Arc<dyn ReplicationPolicy>,
        identity: Option<Arc<LocalIdentityAdapter>>,
        artifacts_dir: Option<PathBuf>,
        governance: Option<(PublicKey, String)>,
    ) -> Arc<Self> {
        Self::open_governed_with_keys(
            store,
            replication_policy,
            identity,
            artifacts_dir,
            governance,
            None,
        )
    }

    /// Builds a relay around an opened store and an optional dedicated key
    /// used only for relay-authored state projections.
    pub fn open_full_with_store_and_keys(
        store: Arc<EventStore>,
        replication_policy: Arc<dyn ReplicationPolicy>,
        identity: Option<Arc<LocalIdentityAdapter>>,
        artifacts_dir: Option<PathBuf>,
        relay_keys: Option<Keys>,
    ) -> Arc<Self> {
        Self::open_governed_with_keys(
            store,
            replication_policy,
            identity,
            artifacts_dir,
            None,
            relay_keys,
        )
    }

    /// Builds a relay with both declaration-governed artifact access and an
    /// optional dedicated key for relay-authored state projections.
    pub fn open_governed_with_keys(
        store: Arc<EventStore>,
        replication_policy: Arc<dyn ReplicationPolicy>,
        identity: Option<Arc<LocalIdentityAdapter>>,
        artifacts_dir: Option<PathBuf>,
        governance: Option<(PublicKey, String)>,
        relay_keys: Option<Keys>,
    ) -> Arc<Self> {
        let (live_events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Arc::new(Self {
            store,
            live_events,
            replication_policy,
            identity,
            artifacts_dir,
            governance,
            relay_keys,
        })
    }

    /// Returns the relay's event store.
    pub fn store(&self) -> Arc<EventStore> {
        Arc::clone(&self.store)
    }

    async fn submit(&self, event: Event) -> Result<WriteResult, StoreError> {
        let projected = if event_kind_u32(&event) == KIND_NIP29_CREATE_GROUP {
            match self.project_group_create(&event) {
                Ok(projected) => projected,
                Err(Nip29ProjectionError::InvalidCommand(reason)) => {
                    return Ok(WriteResult::rejected(&event, reason));
                }
                Err(Nip29ProjectionError::Signing(reason)) => {
                    return Err(StoreError::Materialization(reason));
                }
            }
        } else {
            Vec::new()
        };
        let result = self.store.accept(event.clone()).await?;
        if result.publish_live {
            let _ = self.live_events.send(event);
            self.publish_projected_events(projected).await?;
        }
        Ok(result)
    }

    /// Backfills relay-authored discovery state for accepted create commands
    /// already present in a durable journal.
    pub async fn materialize_existing_nip29_state(&self) -> Result<(), StoreError> {
        if self.relay_keys.is_none() {
            return Ok(());
        }
        let creates = self
            .store
            .query(&[Filter::new().kind(Kind::Custom(KIND_NIP29_CREATE_GROUP as u16))])
            .await
            .map_err(|error| StoreError::Materialization(error.to_string()))?;
        for create in creates {
            let projected = match self.project_group_create(&create) {
                Ok(projected) => projected,
                Err(Nip29ProjectionError::InvalidCommand(reason)) => {
                    tracing::warn!(event_id = %create.id, %reason, "skipping malformed historical group create");
                    continue;
                }
                Err(Nip29ProjectionError::Signing(reason)) => {
                    return Err(StoreError::Materialization(reason));
                }
            };
            self.publish_projected_events(projected).await?;
        }
        Ok(())
    }

    fn project_group_create(&self, event: &Event) -> Result<Vec<Event>, Nip29ProjectionError> {
        let Some(keys) = self.relay_keys.as_ref() else {
            return Ok(Vec::new());
        };
        let group_id = required_tag(event, "h")?;
        Uuid::parse_str(group_id).map_err(|error| {
            Nip29ProjectionError::InvalidCommand(format!("invalid h tag UUID: {error}"))
        })?;
        let name = required_tag(event, "name")?;
        if name.trim().is_empty() {
            return Err(Nip29ProjectionError::InvalidCommand(
                "name tag cannot be empty".to_string(),
            ));
        }
        let visibility = event_tag_value(event, "visibility").unwrap_or("open");
        if !matches!(visibility, "open" | "private") {
            return Err(Nip29ProjectionError::InvalidCommand(format!(
                "unsupported visibility {visibility:?}"
            )));
        }
        let channel_type = event_tag_value(event, "channel_type").unwrap_or("stream");
        if !matches!(channel_type, "stream" | "forum" | "dm") {
            return Err(Nip29ProjectionError::InvalidCommand(format!(
                "unsupported channel_type {channel_type:?}"
            )));
        }

        let mut metadata = vec![parse_tag(["d", group_id])?, parse_tag(["name", name])?];
        if let Some(about) = event_tag_value(event, "about").filter(|value| !value.is_empty()) {
            metadata.push(parse_tag(["about", about])?);
        }
        metadata.push(parse_tag([if visibility == "private" {
            "private"
        } else {
            "public"
        }])?);
        metadata.push(parse_tag(["closed"])?);
        metadata.push(parse_tag(["t", channel_type])?);
        if let Some(ttl) = event_tag_value(event, "ttl").filter(|value| !value.is_empty()) {
            metadata.push(parse_tag(["ttl", ttl])?);
        }

        let creator = event.pubkey.to_hex();
        let admins = vec![
            parse_tag(["d", group_id])?,
            parse_tag(["p", creator.as_str(), "owner"])?,
        ];
        let members = vec![
            parse_tag(["d", group_id])?,
            parse_tag(["p", creator.as_str(), "", "owner"])?,
        ];

        [
            (KIND_NIP29_GROUP_METADATA, metadata),
            (KIND_NIP29_GROUP_ADMINS, admins),
            (KIND_NIP29_GROUP_MEMBERS, members),
        ]
        .into_iter()
        .map(|(kind, tags)| {
            EventBuilder::new(Kind::Custom(kind as u16), "")
                .tags(tags)
                .custom_created_at(event.created_at)
                .sign_with_keys(keys)
                .map_err(|error| Nip29ProjectionError::Signing(error.to_string()))
        })
        .collect()
    }

    async fn publish_projected_events(&self, projected: Vec<Event>) -> Result<(), StoreError> {
        for event in projected {
            let result = self.store.accept(event.clone()).await?;
            if result.publish_live {
                let _ = self.live_events.send(event);
            }
        }
        Ok(())
    }

    fn authorize_direct(
        &self,
        principal: Option<&AuthenticatedPrincipal>,
        event: &Event,
    ) -> Result<(), LocalIdentityError> {
        let Some(identity) = self.identity.as_ref() else {
            return Ok(());
        };
        let principal = principal.ok_or_else(|| {
            LocalIdentityError::denied(IdentityDenialCode::AuthenticationRequired)
        })?;
        decision_result(identity.authorize_direct(principal, event))
    }

    fn authorize_query(
        &self,
        principal: Option<&AuthenticatedPrincipal>,
        operation: ReadOperation,
        filters: &[Filter],
    ) -> Result<(), LocalIdentityError> {
        let Some(identity) = self.identity.as_ref() else {
            return Ok(());
        };
        let principal = principal.ok_or_else(|| {
            LocalIdentityError::denied(IdentityDenialCode::AuthenticationRequired)
        })?;
        decision_result(identity.authorize_local_query(principal, operation, filters))
    }

    fn event_is_visible(
        &self,
        principal: Option<&AuthenticatedPrincipal>,
        operation: ReadOperation,
        event: &Event,
    ) -> bool {
        match (&self.identity, principal) {
            (None, _) => true,
            (Some(identity), Some(principal)) => identity
                .authorize_local_event(principal, operation, event)
                .is_allowed(),
            (Some(_), None) => false,
        }
    }

    async fn query_for(
        &self,
        principal: Option<&AuthenticatedPrincipal>,
        operation: ReadOperation,
        filters: &[Filter],
    ) -> Result<Vec<Event>, ApiError> {
        self.authorize_query(principal, operation, filters)?;
        let mut events = self.store.query(filters).await?;
        events.retain(|event| self.event_is_visible(principal, operation, event));
        Ok(events)
    }

    async fn count_for(
        &self,
        principal: Option<&AuthenticatedPrincipal>,
        filters: &[Filter],
    ) -> Result<usize, ApiError> {
        self.authorize_query(principal, ReadOperation::Count, filters)?;
        validate_filters(filters)?;
        if filters.is_empty() {
            return Ok(0);
        }
        let inner = self.store.inner.lock().await;
        Ok(inner
            .events
            .iter()
            .filter(|stored| filters_match(filters, stored))
            .filter(|stored| self.event_is_visible(principal, ReadOperation::Count, &stored.event))
            .count())
    }

    /// Authenticates a configured peer before invoking the replication sink.
    pub async fn ingest_replication_from_peer(
        &self,
        evidence: LocalPeerEvidence,
        audience: &str,
        record: ReplicationRecord,
    ) -> Result<ReplicationReceipt, ReplicationSinkError> {
        let identity = self.identity.as_ref().ok_or_else(|| {
            LocalIdentityError::denied(IdentityDenialCode::AuthenticationRequired)
        })?;
        let binding = identity
            .authenticate_peer(evidence, audience, &record.source)
            .await?;
        if binding.source != record.source {
            return Err(LocalIdentityError::denied(IdentityDenialCode::SourceMismatch).into());
        }
        self.ingest_replication(record).await
    }
}

fn decision_result(decision: AuthorizationDecision) -> Result<(), LocalIdentityError> {
    match decision {
        AuthorizationDecision::Allowed => Ok(()),
        AuthorizationDecision::Denied { code } => Err(LocalIdentityError::denied(code)),
    }
}

impl ReplicationSinkPort for LocalRelay {
    type Error = ReplicationSinkError;

    async fn ingest_replication(
        &self,
        record: ReplicationRecord,
    ) -> Result<ReplicationReceipt, Self::Error> {
        let event_id = record.event.id.to_hex();
        let rejected = |reason| ReplicationReceipt {
            source: record.source.clone(),
            cursor: record.cursor.clone(),
            event_id: event_id.clone(),
            outcome: ReplicationIngestOutcome::Rejected { reason },
        };

        if let Err(reason) = self.replication_policy.admit(&record.source, &record.event) {
            return Ok(rejected(reason));
        }
        if is_ephemeral_kind(record.event.kind.as_u16()) {
            return Ok(rejected(
                "ephemeral events are not part of durable replication".to_string(),
            ));
        }
        let kind = u32::from(record.event.kind.as_u16());
        if kind == buzz_core::kind::KIND_AUTH || kind == buzz_core::kind::KIND_HTTP_AUTH {
            return Ok(rejected(
                "authentication events are never journaled".to_string(),
            ));
        }

        let result = self.submit(record.event).await?;
        let outcome = if !result.accepted {
            ReplicationIngestOutcome::Rejected {
                reason: result.message,
            }
        } else {
            match result.message.as_str() {
                "stored" => ReplicationIngestOutcome::Stored,
                "duplicate" => ReplicationIngestOutcome::Duplicate,
                "superseded" => ReplicationIngestOutcome::Superseded,
                other => {
                    return Err(ReplicationSinkError::UnexpectedAcceptedOutcome(
                        other.to_string(),
                    ));
                }
            }
        };
        Ok(ReplicationReceipt {
            source: record.source,
            cursor: record.cursor,
            event_id,
            outcome,
        })
    }
}

/// Builds the local relay HTTP and WebSocket router.
pub fn router(relay: Arc<LocalRelay>) -> Router {
    Router::new()
        .route("/", get(relay_root))
        .route("/health", get(health))
        .route("/events", post(submit_event))
        .route("/query", post(query_events))
        .route("/count", post(count_events))
        .route("/replication", post(replication_ingest))
        .route(
            "/artifacts",
            post(artifact_upload).layer(DefaultBodyLimit::max(MAX_ARTIFACT_BYTES)),
        )
        .route(
            "/artifacts/{sha256}",
            get(artifact_fetch).head(artifact_head),
        )
        .with_state(relay)
}

/// Serves a local relay until the returned future is cancelled.
pub async fn serve(listener: TcpListener, relay: Arc<LocalRelay>) -> std::io::Result<()> {
    axum::serve(listener, router(relay)).await
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn submit_event(
    State(relay): State<Arc<LocalRelay>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<WriteResult>, ApiError> {
    let principal = authenticate_http(&relay, &headers, "/events", &body).await?;
    let event: Event = serde_json::from_slice(&body)
        .map_err(|error| ApiError::BadRequest(format!("invalid event JSON: {error}")))?;
    relay.authorize_direct(principal.as_ref(), &event)?;
    Ok(Json(relay.submit(event).await?))
}

async fn query_events(
    State(relay): State<Arc<LocalRelay>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Vec<Event>>, ApiError> {
    let principal = authenticate_http(&relay, &headers, "/query", &body).await?;
    let filters = parse_filter_body(&body)?;
    Ok(Json(
        relay
            .query_for(principal.as_ref(), ReadOperation::Query, &filters)
            .await?,
    ))
}

async fn count_events(
    State(relay): State<Arc<LocalRelay>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let principal = authenticate_http(&relay, &headers, "/count", &body).await?;
    let filters = parse_filter_body(&body)?;
    let count = relay.count_for(principal.as_ref(), &filters).await?;
    Ok(Json(json!({ "count": count })))
}

async fn authenticate_http(
    relay: &LocalRelay,
    headers: &HeaderMap,
    path: &str,
    body: &[u8],
) -> Result<Option<AuthenticatedPrincipal>, ApiError> {
    authenticate_http_method(relay, headers, "POST", path, body).await
}

async fn authenticate_http_method(
    relay: &LocalRelay,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<Option<AuthenticatedPrincipal>, ApiError> {
    let Some(identity) = relay.identity.as_ref() else {
        return Ok(None);
    };
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| LocalIdentityError::denied(IdentityDenialCode::AudienceMismatch))?;
    let audience = format!("http://{host}{path}");
    let encoded = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Nostr "))
        .ok_or_else(|| LocalIdentityError::denied(IdentityDenialCode::AuthenticationRequired))?;
    let decoded = BASE64
        .decode(encoded)
        .map_err(|_| LocalIdentityError::denied(IdentityDenialCode::InvalidEvidence))?;
    let event_json = String::from_utf8(decoded)
        .map_err(|_| LocalIdentityError::denied(IdentityDenialCode::InvalidEvidence))?;
    let principal = identity
        .authenticate(
            LocalAuthenticationEvidence::Nip98 {
                event_json,
                method: method.to_string(),
                body: body.to_vec(),
            },
            &audience,
        )
        .await?;
    Ok(Some(principal))
}

/// Peer-bound replication sink over HTTP, mirroring the Cloudflare adapter:
/// mandatory payload-bound NIP-98 evidence whose signing key must be a
/// destination-configured verification key for the batch's source stream.
async fn replication_ingest(
    State(relay): State<Arc<LocalRelay>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Vec<ReplicationReceipt>>, ApiError> {
    if relay.identity.is_none() {
        return Err(LocalIdentityError::denied(IdentityDenialCode::AuthenticationRequired).into());
    }
    let principal = authenticate_http(&relay, &headers, "/replication", &body)
        .await?
        .ok_or_else(|| LocalIdentityError::denied(IdentityDenialCode::AuthenticationRequired))?;
    let records: Vec<ReplicationRecord> = serde_json::from_slice(&body)
        .map_err(|error| ApiError::BadRequest(format!("invalid replication JSON: {error}")))?;
    let Some(first) = records.first() else {
        return Ok(Json(Vec::new()));
    };
    let source = first.source.clone();
    let identity = relay.identity.as_ref().expect("checked above");
    let pubkey = principal
        .principal
        .nostr_pubkey()
        .ok_or_else(|| LocalIdentityError::denied(IdentityDenialCode::PeerUnbound))?;
    identity.bind_peer_key(&source, pubkey)?;

    let mut receipts = Vec::with_capacity(records.len());
    for record in records {
        if record.source != source {
            receipts.push(ReplicationReceipt {
                source: record.source.clone(),
                cursor: record.cursor.clone(),
                event_id: record.event.id.to_hex(),
                outcome: ReplicationIngestOutcome::Rejected {
                    reason: "record source does not match the authenticated batch".to_string(),
                },
            });
            break;
        }
        let receipt = relay
            .ingest_replication(record)
            .await
            .map_err(|error| match error {
                ReplicationSinkError::Identity(identity_error) => {
                    ApiError::Identity(identity_error)
                }
                ReplicationSinkError::Store(store_error) => ApiError::Store(store_error),
                other => ApiError::BadRequest(other.to_string()),
            })?;
        let rejected = !receipt.checkpoint_safe();
        receipts.push(receipt);
        if rejected {
            break;
        }
    }
    Ok(Json(receipts))
}

/// Extracts the Nostr pubkey from an authenticated principal, when it is one.
fn principal_pubkey(authenticated: &AuthenticatedPrincipal) -> Option<PublicKey> {
    match &authenticated.principal {
        Principal::Nostr { pubkey } => Some(*pubkey),
        _ => None,
    }
}

/// Upload admission (evaluation rule 4): with governance anchors set, blobs
/// are stored only for the owner or an admitted replication peer.
fn authorize_artifact_upload(
    relay: &LocalRelay,
    principal: Option<&AuthenticatedPrincipal>,
) -> Result<(), ApiError> {
    let Some((owner, _)) = relay.governance.as_ref() else {
        return Ok(());
    };
    let allowed = principal.and_then(principal_pubkey).is_some_and(|pubkey| {
        pubkey == *owner
            || relay
                .identity
                .as_ref()
                .is_some_and(|identity| identity.is_admitted_verification_key(&pubkey))
    });
    if allowed {
        Ok(())
    } else {
        Err(ApiError::Identity(LocalIdentityError::denied(
            IdentityDenialCode::ScopeDenied,
        )))
    }
}

/// Fetch/head authorization (evaluation rule 4): with governance anchors
/// set, a blob is disclosed only to the owner or a principal whose active
/// read grant covers a stream that selects a referencing journal event.
async fn authorize_artifact_fetch(
    relay: &LocalRelay,
    principal: Option<&AuthenticatedPrincipal>,
    sha256: &str,
) -> Result<(), ApiError> {
    let Some((owner, node_label)) = relay.governance.as_ref() else {
        return Ok(());
    };
    let allowed = match principal.and_then(principal_pubkey) {
        Some(pubkey) if pubkey == *owner => true,
        Some(pubkey) => {
            declarations::artifact_fetch_allowed(&relay.store, owner, node_label, &pubkey, sha256)
                .await
                .map_err(|error| ApiError::BadRequest(error.to_string()))?
        }
        None => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(ApiError::Identity(LocalIdentityError::denied(
            IdentityDenialCode::ScopeDenied,
        )))
    }
}

/// Stores one immutable blob under its SHA-256. The NIP-98 payload tag binds
/// the uploader's proof to the exact content, so authentication doubles as an
/// integrity commitment over the artifact.
async fn artifact_upload(
    State(relay): State<Arc<LocalRelay>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let Some(dir) = relay.artifacts_dir.clone() else {
        return Err(ApiError::NotFound("artifact store is not enabled".into()));
    };
    let principal = authenticate_http(&relay, &headers, "/artifacts", &body).await?;
    authorize_artifact_upload(&relay, principal.as_ref())?;
    if body.is_empty() {
        return Err(ApiError::BadRequest("artifact body is empty".into()));
    }
    let hash = nostr::hashes::sha256::Hash::hash(&body).to_string();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(StoreError::Io)?;
    let final_path = dir.join(&hash);
    if !tokio::fs::try_exists(&final_path)
        .await
        .map_err(StoreError::Io)?
    {
        let tmp_path = dir.join(format!(".{hash}.{}", Uuid::new_v4()));
        let mut file = tokio::fs::File::create(&tmp_path)
            .await
            .map_err(StoreError::Io)?;
        file.write_all(&body).await.map_err(StoreError::Io)?;
        file.sync_data().await.map_err(StoreError::Io)?;
        tokio::fs::rename(&tmp_path, &final_path)
            .await
            .map_err(StoreError::Io)?;
    }
    Ok(Json(json!({
        "sha256": hash,
        "size": body.len(),
        "url": format!("/artifacts/{hash}"),
    })))
}

/// Cheap existence probe: reports whether a blob is present without reading or
/// transferring it, so a sync walker can skip content it already holds.
async fn artifact_head(
    State(relay): State<Arc<LocalRelay>>,
    UrlPath(sha256): UrlPath<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let Some(dir) = relay.artifacts_dir.clone() else {
        return Err(ApiError::NotFound("artifact store is not enabled".into()));
    };
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest(
            "artifact ID must be 64 hex characters".into(),
        ));
    }
    let sha256 = sha256.to_ascii_lowercase();
    // A HEAD is a metadata-only GET; the same GET-signed NIP-98 proof (URL +
    // empty-payload binding) authorizes it, so callers need no HEAD variant.
    let principal = authenticate_http_method(
        &relay,
        &headers,
        "GET",
        &format!("/artifacts/{sha256}"),
        b"",
    )
    .await?;
    authorize_artifact_fetch(&relay, principal.as_ref(), &sha256).await?;
    if tokio::fs::try_exists(dir.join(&sha256))
        .await
        .map_err(StoreError::Io)?
    {
        Ok(StatusCode::OK.into_response())
    } else {
        Err(ApiError::NotFound(format!("unknown artifact {sha256}")))
    }
}

/// Serves one blob by content hash, re-verifying the bytes before disclosure
/// so silent disk corruption fails closed instead of shipping.
async fn artifact_fetch(
    State(relay): State<Arc<LocalRelay>>,
    UrlPath(sha256): UrlPath<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let Some(dir) = relay.artifacts_dir.clone() else {
        return Err(ApiError::NotFound("artifact store is not enabled".into()));
    };
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest(
            "artifact ID must be 64 hex characters".into(),
        ));
    }
    let sha256 = sha256.to_ascii_lowercase();
    let principal = authenticate_http_method(
        &relay,
        &headers,
        "GET",
        &format!("/artifacts/{sha256}"),
        b"",
    )
    .await?;
    authorize_artifact_fetch(&relay, principal.as_ref(), &sha256).await?;
    let path = dir.join(&sha256);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::NotFound(format!("unknown artifact {sha256}")));
        }
        Err(error) => return Err(StoreError::Io(error).into()),
    };
    if nostr::hashes::sha256::Hash::hash(&bytes).to_string() != sha256 {
        return Err(ApiError::BadRequest(format!(
            "artifact {sha256} failed content verification"
        )));
    }
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
        bytes,
    )
        .into_response())
}

async fn relay_root(State(relay): State<Arc<LocalRelay>>, request: Request) -> Response {
    let (mut parts, _) = request.into_parts();
    if parts.headers.contains_key(axum::http::header::UPGRADE) {
        let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &relay).await {
            Ok(ws) => ws,
            Err(rejection) => return rejection.into_response(),
        };
        let host = parts
            .headers
            .get(HOST)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty());
        if relay.identity.is_some() && host.is_none() {
            return ApiError::Identity(LocalIdentityError::denied(
                IdentityDenialCode::AudienceMismatch,
            ))
            .into_response();
        }
        let audience = host.map(|host| format!("ws://{host}/"));
        return ws
            .on_upgrade(move |socket| websocket_session(socket, relay, audience))
            .into_response();
    }

    let supported_nips = if relay.identity.is_some() {
        if relay.relay_keys.is_some() {
            vec![1, 11, 29, 42, 98]
        } else {
            vec![1, 11, 42, 98]
        }
    } else if relay.relay_keys.is_some() {
        vec![1, 11, 29]
    } else {
        vec![1, 11]
    };
    let mut document = json!({
        "name": "Buzz local relay",
        "description": "Durable single-process Buzz relay",
        "software": "https://github.com/block/buzz",
        "version": env!("CARGO_PKG_VERSION"),
        "supported_nips": supported_nips,
    });
    if let Some(keys) = relay.relay_keys.as_ref() {
        document["pubkey"] = Value::String(keys.public_key().to_hex());
    }
    (
        [(axum::http::header::CONTENT_TYPE, "application/nostr+json")],
        Json(document),
    )
        .into_response()
}

async fn websocket_session(socket: WebSocket, relay: Arc<LocalRelay>, audience: Option<String>) {
    let (mut sender, mut receiver) = socket.split();
    let mut live_events = relay.live_events.subscribe();
    let mut subscriptions: HashMap<String, Vec<Filter>> = HashMap::new();
    let challenge = relay
        .identity
        .as_ref()
        .map(|_| buzz_auth::generate_challenge());
    let mut principal = None;

    if let Some(challenge) = challenge.as_ref() {
        if send_json(&mut sender, json!(["AUTH", challenge]))
            .await
            .is_err()
        {
            return;
        }
    }

    loop {
        tokio::select! {
            inbound = receiver.next() => {
                let Some(inbound) = inbound else {
                    break;
                };
                let Ok(message) = inbound else {
                    break;
                };
                let should_continue = handle_client_message(
                    message,
                    &relay,
                    &mut subscriptions,
                    &mut principal,
                    challenge.as_deref(),
                    audience.as_deref(),
                    &mut sender,
                ).await;
                if !should_continue {
                    break;
                }
            }
            live = live_events.recv() => {
                match live {
                    Ok(event) => {
                        for (subscription_id, filters) in &subscriptions {
                            let stored = stored_event(event.clone());
                            if filters_match(filters, &stored)
                                && relay.event_is_visible(
                                    principal.as_ref(),
                                    ReadOperation::LiveDelivery,
                                    &event,
                                )
                                && send_json(
                                    &mut sender,
                                    json!(["EVENT", subscription_id, event]),
                                )
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        if send_json(
                            &mut sender,
                            json!(["NOTICE", format!("local relay subscriber lagged by {skipped} events")]),
                        )
                        .await
                        .is_err()
                        {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    }
}

async fn handle_client_message<S>(
    message: Message,
    relay: &LocalRelay,
    subscriptions: &mut HashMap<String, Vec<Filter>>,
    principal: &mut Option<AuthenticatedPrincipal>,
    challenge: Option<&str>,
    audience: Option<&str>,
    sender: &mut S,
) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    match message {
        Message::Text(text) => {
            let parsed = match serde_json::from_str::<Value>(text.as_str()) {
                Ok(Value::Array(parts)) => parts,
                _ => {
                    return send_json(sender, json!(["NOTICE", "invalid message"]))
                        .await
                        .is_ok();
                }
            };
            let Some(verb) = parsed.first().and_then(Value::as_str) else {
                return send_json(sender, json!(["NOTICE", "invalid message"]))
                    .await
                    .is_ok();
            };

            match verb {
                "AUTH" => {
                    handle_ws_auth(&parsed, relay, principal, challenge, audience, sender).await
                }
                "EVENT" => handle_ws_event(&parsed, relay, principal.as_ref(), sender).await,
                "REQ" => {
                    handle_ws_req(&parsed, relay, subscriptions, principal.as_ref(), sender).await
                }
                "CLOSE" => {
                    if let Some(subscription_id) = parsed.get(1).and_then(Value::as_str) {
                        subscriptions.remove(subscription_id);
                    }
                    true
                }
                _ => send_json(sender, json!(["NOTICE", "unsupported message"]))
                    .await
                    .is_ok(),
            }
        }
        Message::Ping(payload) => sender.send(Message::Pong(payload)).await.is_ok(),
        Message::Pong(_) => true,
        Message::Close(_) => false,
        Message::Binary(_) => false,
    }
}

async fn handle_ws_auth<S>(
    parts: &[Value],
    relay: &LocalRelay,
    principal: &mut Option<AuthenticatedPrincipal>,
    challenge: Option<&str>,
    audience: Option<&str>,
    sender: &mut S,
) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    let event = match parts.get(1).cloned().map(serde_json::from_value::<Event>) {
        Some(Ok(event)) => event,
        _ => {
            return send_json(sender, json!(["OK", "", false, "invalid_evidence"]))
                .await
                .is_ok();
        }
    };
    let event_id = event.id.to_hex();
    let Some(identity) = relay.identity.as_ref() else {
        return send_json(
            sender,
            json!(["OK", event_id, false, "authentication_not_enabled"]),
        )
        .await
        .is_ok();
    };
    let (Some(challenge), Some(audience)) = (challenge, audience) else {
        return send_json(sender, json!(["OK", event_id, false, "audience_mismatch"]))
            .await
            .is_ok();
    };
    if principal.is_some() {
        return send_json(sender, json!(["OK", event_id, false, "replay_detected"]))
            .await
            .is_ok();
    }
    match identity
        .authenticate(
            LocalAuthenticationEvidence::Nip42 {
                event,
                challenge: challenge.to_string(),
            },
            audience,
        )
        .await
    {
        Ok(authenticated) => {
            *principal = Some(authenticated);
            send_json(sender, json!(["OK", event_id, true, "authenticated"]))
                .await
                .is_ok()
        }
        Err(error) => {
            let message = identity_error_token(&error);
            send_json(sender, json!(["OK", event_id, false, message]))
                .await
                .is_ok()
        }
    }
}

async fn handle_ws_event<S>(
    parts: &[Value],
    relay: &LocalRelay,
    principal: Option<&AuthenticatedPrincipal>,
    sender: &mut S,
) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    let event = match parts.get(1).cloned().map(serde_json::from_value::<Event>) {
        Some(Ok(event)) => event,
        _ => {
            return send_json(sender, json!(["OK", "", false, "invalid: malformed event"]))
                .await
                .is_ok();
        }
    };
    let event_id = event.id.to_hex();
    if let Err(error) = relay.authorize_direct(principal, &event) {
        return send_json(
            sender,
            json!(["OK", event_id, false, identity_error_token(&error)]),
        )
        .await
        .is_ok();
    }
    let result = match relay.submit(event).await {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(%error, "local event persistence failed");
            return send_json(
                sender,
                json!(["OK", event_id, false, format!("error: {error}")]),
            )
            .await
            .is_ok();
        }
    };
    send_json(
        sender,
        json!(["OK", result.event_id, result.accepted, result.message]),
    )
    .await
    .is_ok()
}

async fn handle_ws_req<S>(
    parts: &[Value],
    relay: &LocalRelay,
    subscriptions: &mut HashMap<String, Vec<Filter>>,
    principal: Option<&AuthenticatedPrincipal>,
    sender: &mut S,
) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    let Some(subscription_id) = parts.get(1).and_then(Value::as_str) else {
        return send_json(sender, json!(["NOTICE", "invalid REQ"]))
            .await
            .is_ok();
    };
    if subscription_id.len() > MAX_SUBSCRIPTION_ID_LENGTH {
        return send_json(
            sender,
            json!(["CLOSED", subscription_id, "subscription ID too long"]),
        )
        .await
        .is_ok();
    }
    if !subscriptions.contains_key(subscription_id)
        && subscriptions.len() >= MAX_SUBSCRIPTIONS_PER_CONNECTION
    {
        return send_json(
            sender,
            json!(["CLOSED", subscription_id, "too many subscriptions"]),
        )
        .await
        .is_ok();
    }

    let filter_values: Vec<Value> = parts.iter().skip(2).cloned().collect();
    if let Err(error) = validate_filter_fields(&filter_values) {
        return send_json(
            sender,
            json!(["CLOSED", subscription_id, error.to_string()]),
        )
        .await
        .is_ok();
    }
    let filters: Result<Vec<Filter>, _> = filter_values
        .into_iter()
        .map(serde_json::from_value)
        .collect();
    let filters = match filters {
        Ok(filters) if !filters.is_empty() => filters,
        _ => {
            return send_json(
                sender,
                json!(["CLOSED", subscription_id, "invalid filters"]),
            )
            .await
            .is_ok();
        }
    };

    if let Err(error) = validate_filters(&filters) {
        return send_json(
            sender,
            json!(["CLOSED", subscription_id, error.to_string()]),
        )
        .await
        .is_ok();
    }

    let historical = match relay
        .query_for(principal, ReadOperation::HistoricalSubscription, &filters)
        .await
    {
        Ok(historical) => historical,
        Err(error) => {
            return send_json(
                sender,
                json!(["CLOSED", subscription_id, error.to_string()]),
            )
            .await
            .is_ok();
        }
    };
    subscriptions.insert(subscription_id.to_string(), filters);
    for event in historical {
        if send_json(sender, json!(["EVENT", subscription_id, event]))
            .await
            .is_err()
        {
            return false;
        }
    }
    send_json(sender, json!(["EOSE", subscription_id]))
        .await
        .is_ok()
}

async fn send_json<S>(sender: &mut S, value: Value) -> Result<(), S::Error>
where
    S: futures_util::Sink<Message> + Unpin,
{
    sender.send(Message::Text(value.to_string().into())).await
}

#[derive(Debug, Error)]
enum ApiError {
    #[error(transparent)]
    Identity(#[from] LocalIdentityError),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotFound(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Query(#[from] QueryError),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Identity(LocalIdentityError::Denied { code }) => match code {
                IdentityDenialCode::AuthenticationRequired
                | IdentityDenialCode::InvalidEvidence
                | IdentityDenialCode::EvidenceExpired
                | IdentityDenialCode::AudienceMismatch
                | IdentityDenialCode::ReplayDetected => StatusCode::UNAUTHORIZED,
                IdentityDenialCode::AuthorMismatch
                | IdentityDenialCode::DelegationInvalid
                | IdentityDenialCode::PeerUnbound
                | IdentityDenialCode::SourceMismatch
                | IdentityDenialCode::ScopeDenied
                | IdentityDenialCode::EventDisclosureDenied => StatusCode::FORBIDDEN,
            },
            Self::Identity(LocalIdentityError::Internal(_)) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Query(_) => StatusCode::BAD_REQUEST,
        };
        let code = match &self {
            Self::Identity(error) => error.denial_code().map(IdentityDenialCode::as_str),
            Self::BadRequest(_) | Self::NotFound(_) | Self::Store(_) | Self::Query(_) => None,
        };
        (
            status,
            Json(json!({ "error": self.to_string(), "code": code })),
        )
            .into_response()
    }
}

fn identity_error_token(error: &LocalIdentityError) -> &'static str {
    error
        .denial_code()
        .map(IdentityDenialCode::as_str)
        .unwrap_or("identity_internal_error")
}

#[derive(Default)]
struct ReplayedLog {
    events: Vec<StoredEvent>,
    journal: Vec<Event>,
    seen_ids: HashSet<nostr::EventId>,
}

fn replay_log(path: &Path) -> Result<ReplayedLog, StoreError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(StoreError::Io(error)),
    };
    let mut replayed = ReplayedLog::default();

    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let line_number = index + 1;
        let event: Event =
            serde_json::from_str(line).map_err(|source| StoreError::MalformedRecord {
                line: line_number,
                source,
            })?;
        verify_event(&event).map_err(|error| StoreError::InvalidRecord {
            line: line_number,
            reason: error.to_string(),
        })?;
        replayed.seen_ids.insert(event.id);
        replayed.journal.push(event.clone());
        apply_effective_event(&mut replayed.events, stored_event(event));
    }

    Ok(replayed)
}

fn stored_event(event: Event) -> StoredEvent {
    let channel_id = event
        .tags
        .filter(TagKind::SingleLetter(SingleLetterTag::lowercase(
            Alphabet::H,
        )))
        .filter_map(|tag| tag.content())
        .find_map(|value| value.parse::<Uuid>().ok());
    StoredEvent::with_received_at(event, Utc::now(), channel_id, true)
}

fn required_tag<'a>(event: &'a Event, name: &str) -> Result<&'a str, Nip29ProjectionError> {
    event_tag_value(event, name)
        .ok_or_else(|| Nip29ProjectionError::InvalidCommand(format!("missing required {name} tag")))
}

fn event_tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some(name))
            .then(|| values.get(1).map(String::as_str))
            .flatten()
    })
}

fn parse_tag<const N: usize>(values: [&str; N]) -> Result<Tag, Nip29ProjectionError> {
    Tag::parse(values).map_err(|error| Nip29ProjectionError::InvalidCommand(error.to_string()))
}

fn validate_filters(filters: &[Filter]) -> Result<(), QueryError> {
    if filters.iter().any(|filter| filter.search.is_some()) {
        return Err(QueryError::SearchUnsupported);
    }
    Ok(())
}

/// Filter fields the portable subset accepts; `search` stays listed so it
/// reaches [`validate_filters`] and fails with its dedicated denial.
const SUPPORTED_FILTER_FIELDS: [&str; 7] = [
    "ids", "authors", "kinds", "since", "until", "limit", "search",
];

/// Rejects filter fields outside the supported NIP-01 subset.
///
/// Serde silently drops unknown fields, which would broaden a query the
/// caller believed was narrower. This check runs on the raw JSON before
/// deserialization so unsupported extensions fail closed instead.
fn validate_filter_fields(filters: &[Value]) -> Result<(), QueryError> {
    for filter in filters {
        let Some(object) = filter.as_object() else {
            continue;
        };
        for field in object.keys() {
            if !field.starts_with('#') && !SUPPORTED_FILTER_FIELDS.contains(&field.as_str()) {
                return Err(QueryError::UnsupportedFilterField(field.clone()));
            }
        }
    }
    Ok(())
}

/// Parses an HTTP filter body, failing closed on unsupported filter fields.
fn parse_filter_body(body: &[u8]) -> Result<Vec<Filter>, ApiError> {
    let values: Vec<Value> = serde_json::from_slice(body)
        .map_err(|error| ApiError::BadRequest(format!("invalid filter JSON: {error}")))?;
    validate_filter_fields(&values)?;
    serde_json::from_value(Value::Array(values))
        .map_err(|error| ApiError::BadRequest(format!("invalid filter JSON: {error}")))
}

/// Parses the bind address used by the local relay binary.
pub fn parse_bind_address(raw: &str) -> Result<SocketAddr, std::net::AddrParseError> {
    raw.parse()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::{SinkExt, StreamExt};
    use nostr::{EventBuilder, Keys, Kind, Timestamp};
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    use super::*;

    fn test_log_path() -> PathBuf {
        std::env::temp_dir().join(format!("buzz-local-relay-{}.ndjson", Uuid::new_v4()))
    }

    fn signed_event(kind: u16, content: &str) -> Event {
        EventBuilder::new(Kind::Custom(kind), content)
            .sign_with_keys(&Keys::generate())
            .expect("test event signs")
    }

    fn signed_group_create(keys: &Keys, group_id: Uuid, created_at: Timestamp) -> Event {
        EventBuilder::new(Kind::Custom(KIND_NIP29_CREATE_GROUP as u16), "")
            .tags(vec![
                Tag::parse(["h", &group_id.to_string()]).expect("h tag parses"),
                Tag::parse(["name", "general"]).expect("name tag parses"),
                Tag::parse(["visibility", "open"]).expect("visibility tag parses"),
                Tag::parse(["channel_type", "stream"]).expect("type tag parses"),
                Tag::parse(["about", "General discussion"]).expect("about tag parses"),
            ])
            .custom_created_at(created_at)
            .sign_with_keys(keys)
            .expect("group create signs")
    }

    fn group_filter(kind: u32, group_id: Uuid) -> Filter {
        serde_json::from_value(json!({
            "kinds": [kind],
            "#d": [group_id.to_string()],
            "limit": 1,
        }))
        .expect("group filter parses")
    }

    #[test]
    fn specification_fixture_is_a_valid_signed_event() {
        let event: Event = serde_json::from_str(include_str!(
            "../../../specs/fixtures/local-relay/signed-message.json"
        ))
        .expect("fixture parses");
        verify_event(&event).expect("fixture signature verifies");
    }

    #[tokio::test]
    async fn durable_event_survives_reopen_and_duplicate_is_idempotent() {
        let path = test_log_path();
        let event = signed_event(1, "durable");
        let store = EventStore::open(StorageMode::Durable(path.clone()))
            .await
            .expect("store opens");

        let first = store.accept(event.clone()).await.expect("event stores");
        let duplicate = store
            .accept(event.clone())
            .await
            .expect("duplicate accepted");
        assert_eq!(first.message, "stored");
        assert_eq!(duplicate.message, "duplicate");
        drop(store);

        let reopened = EventStore::open(StorageMode::Durable(path.clone()))
            .await
            .expect("store reopens");
        let results = reopened
            .query(&[Filter::new().id(event.id)])
            .await
            .expect("query succeeds");
        assert_eq!(results.len(), 1);

        let records = std::fs::read_to_string(&path).expect("log reads");
        assert_eq!(records.lines().count(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn tampered_event_is_rejected_and_ephemeral_event_is_not_stored() {
        let store = EventStore::open(StorageMode::Ephemeral)
            .await
            .expect("store opens");
        let event = signed_event(1, "original");
        let mut value = serde_json::to_value(&event).expect("serializes");
        value["content"] = Value::String("tampered".to_string());
        let tampered: Event = serde_json::from_value(value).expect("event parses");
        let rejected = store.accept(tampered).await.expect("rejection returns");
        assert!(!rejected.accepted);

        let ephemeral = signed_event(20_001, "typing");
        let accepted = store
            .accept(ephemeral.clone())
            .await
            .expect("ephemeral accepted");
        let duplicate = store
            .accept(ephemeral.clone())
            .await
            .expect("ephemeral duplicate accepted");
        assert_eq!(accepted.message, "ephemeral");
        assert_eq!(duplicate.message, "duplicate");
        assert!(store
            .query(&[Filter::new().id(ephemeral.id)])
            .await
            .expect("query succeeds")
            .is_empty());
    }

    #[tokio::test]
    async fn newer_replaceable_event_becomes_effective() {
        let store = EventStore::open(StorageMode::Ephemeral)
            .await
            .expect("store opens");
        let keys = Keys::generate();
        let now = Timestamp::now();
        let older = EventBuilder::new(Kind::Metadata, "old")
            .custom_created_at(Timestamp::from(now.as_secs().saturating_sub(1)))
            .sign_with_keys(&keys)
            .expect("older signs");
        let newer = EventBuilder::new(Kind::Metadata, "new")
            .custom_created_at(now)
            .sign_with_keys(&keys)
            .expect("newer signs");

        store.accept(older).await.expect("older stores");
        store.accept(newer.clone()).await.expect("newer stores");
        let results = store
            .query(&[Filter::new().kind(Kind::Metadata)])
            .await
            .expect("query succeeds");
        assert_eq!(results, vec![newer]);
    }

    #[tokio::test]
    async fn group_create_materializes_relay_signed_discovery_state_idempotently() {
        let store = Arc::new(
            EventStore::open(StorageMode::Ephemeral)
                .await
                .expect("store opens"),
        );
        let relay_keys = Keys::generate();
        let relay = LocalRelay::open_full_with_store_and_keys(
            Arc::clone(&store),
            Arc::new(ReplicationDisabled),
            None,
            None,
            Some(relay_keys.clone()),
        );
        let creator = Keys::generate();
        let group_id = Uuid::new_v4();
        let create = signed_group_create(&creator, group_id, Timestamp::now());

        let first = relay
            .submit(create.clone())
            .await
            .expect("group create stores");
        let duplicate = relay.submit(create).await.expect("retry is accepted");
        assert_eq!(first.message, "stored");
        assert_eq!(duplicate.message, "duplicate");

        let metadata = store
            .query(&[group_filter(KIND_NIP29_GROUP_METADATA, group_id)])
            .await
            .expect("metadata query succeeds");
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].pubkey, relay_keys.public_key());
        assert_eq!(event_tag_value(&metadata[0], "name"), Some("general"));
        assert_eq!(
            event_tag_value(&metadata[0], "about"),
            Some("General discussion")
        );
        assert_eq!(event_tag_value(&metadata[0], "t"), Some("stream"));

        let admins = store
            .query(&[group_filter(KIND_NIP29_GROUP_ADMINS, group_id)])
            .await
            .expect("admins query succeeds");
        let members = store
            .query(&[group_filter(KIND_NIP29_GROUP_MEMBERS, group_id)])
            .await
            .expect("members query succeeds");
        assert_eq!(admins.len(), 1);
        assert_eq!(members.len(), 1);
        let creator_hex = creator.public_key().to_hex();
        assert!(admins[0].tags.iter().any(|tag| {
            tag.as_slice() == ["p".to_string(), creator_hex.clone(), "owner".to_string()]
        }));
        assert!(members[0].tags.iter().any(|tag| {
            tag.as_slice()
                == [
                    "p".to_string(),
                    creator_hex.clone(),
                    "".to_string(),
                    "owner".to_string(),
                ]
        }));
    }

    #[tokio::test]
    async fn historical_group_create_is_backfilled_once() {
        let store = Arc::new(
            EventStore::open(StorageMode::Ephemeral)
                .await
                .expect("store opens"),
        );
        let group_id = Uuid::new_v4();
        let create = signed_group_create(&Keys::generate(), group_id, Timestamp::now());
        store
            .accept(create)
            .await
            .expect("historical create stores");
        let relay = LocalRelay::open_full_with_store_and_keys(
            Arc::clone(&store),
            Arc::new(ReplicationDisabled),
            None,
            None,
            Some(Keys::generate()),
        );

        relay
            .materialize_existing_nip29_state()
            .await
            .expect("first backfill succeeds");
        relay
            .materialize_existing_nip29_state()
            .await
            .expect("second backfill succeeds");

        let metadata = store
            .query(&[group_filter(KIND_NIP29_GROUP_METADATA, group_id)])
            .await
            .expect("metadata query succeeds");
        assert_eq!(metadata.len(), 1);
    }

    #[tokio::test]
    async fn nip11_document_advertises_relay_state_identity() {
        let store = Arc::new(
            EventStore::open(StorageMode::Ephemeral)
                .await
                .expect("store opens"),
        );
        let relay_keys = Keys::generate();
        let relay = LocalRelay::open_full_with_store_and_keys(
            store,
            Arc::new(ReplicationDisabled),
            None,
            None,
            Some(relay_keys.clone()),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener binds");
        let address = listener.local_addr().expect("address available");
        let server = tokio::spawn(serve(listener, relay));

        let response = reqwest::Client::new()
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect("NIP-11 request succeeds");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/nostr+json")
        );
        let document: Value = response.json().await.expect("NIP-11 JSON parses");
        assert_eq!(
            document["pubkey"],
            Value::String(relay_keys.public_key().to_hex())
        );
        assert!(document["supported_nips"]
            .as_array()
            .expect("supported_nips is an array")
            .contains(&json!(29)));
        server.abort();
    }

    #[tokio::test]
    async fn search_filter_fails_explicitly() {
        let store = EventStore::open(StorageMode::Ephemeral)
            .await
            .expect("store opens");
        let error = store
            .query(&[Filter::new().search("coherence")])
            .await
            .expect_err("search must not silently return unfiltered events");
        assert!(matches!(error, QueryError::SearchUnsupported));
    }

    #[test]
    fn filter_field_validation_fails_closed_on_unknown_fields() {
        assert!(validate_filter_fields(&[
            json!({ "ids": ["a"], "authors": ["b"], "kinds": [1], "#t": ["x"], "limit": 5 })
        ])
        .is_ok());

        let error = validate_filter_fields(&[json!({ "kinds": [1], "unknown_extension": 1 })])
            .expect_err("unknown filter fields must not silently broaden a query");
        assert!(matches!(
            error,
            QueryError::UnsupportedFilterField(field) if field == "unknown_extension"
        ));
    }

    #[tokio::test]
    async fn http_submission_and_websocket_history_share_the_store() {
        let relay = LocalRelay::open(StorageMode::Ephemeral)
            .await
            .expect("relay opens");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener binds");
        let address = listener.local_addr().expect("address available");
        let server = tokio::spawn(serve(listener, relay));
        let event = signed_event(1, "over HTTP");

        let response = reqwest::Client::new()
            .post(format!("http://{address}/events"))
            .json(&event)
            .send()
            .await
            .expect("HTTP submit succeeds");
        assert_eq!(response.status(), StatusCode::OK);

        let (mut websocket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/"))
            .await
            .expect("websocket connects");
        websocket
            .send(ClientMessage::Text(
                json!(["REQ", "history", { "ids": [event.id.to_hex()] }])
                    .to_string()
                    .into(),
            ))
            .await
            .expect("REQ sends");

        let event_frame = tokio::time::timeout(Duration::from_secs(2), websocket.next())
            .await
            .expect("EVENT arrives")
            .expect("stream remains open")
            .expect("EVENT is valid");
        let eose_frame = tokio::time::timeout(Duration::from_secs(2), websocket.next())
            .await
            .expect("EOSE arrives")
            .expect("stream remains open")
            .expect("EOSE is valid");

        let event_text = event_frame.into_text().expect("EVENT is text");
        let eose_text = eose_frame.into_text().expect("EOSE is text");
        assert!(event_text.starts_with("[\"EVENT\",\"history\""));
        assert_eq!(eose_text, "[\"EOSE\",\"history\"]");

        server.abort();
    }
}
