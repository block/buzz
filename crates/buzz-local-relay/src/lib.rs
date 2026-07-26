//! A small, durable Buzz relay for local experimentation.
//!
//! The local relay implements a deliberately narrow NIP-01 and Buzz HTTP
//! bridge subset. It verifies real Nostr signatures and persists durable events
//! to an append-only NDJSON log. Its optional identity adapter provides a
//! portable authorization boundary without emulating production membership,
//! media, search indexing, workflows, or multi-node fan-out.

/// Laptop NIP-42/NIP-98 authentication and authorization adapter.
pub mod identity;

use std::collections::HashMap;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
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
    ReadOperation, ReplicationPeerAuthenticator,
};
use buzz_core::relay::{apply_effective_event, decide_event, is_ephemeral_kind, EventDecision};
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
use nostr::{Alphabet, Event, Filter, SingleLetterTag, TagKind};
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
}

impl LocalReplicationSource {
    /// Binds an operator-assigned source identity to a local event store.
    pub fn new(source: ReplicationSourceId, store: Arc<EventStore>) -> Self {
        Self { source, store }
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

        let page_size = limit.min(MAX_REPLICATION_BATCH_SIZE);
        let end = start.saturating_add(page_size).min(inner.journal.len());
        let records = inner.journal[start..end]
            .iter()
            .enumerate()
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

struct ReplicationDisabled;

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
        let store = Arc::new(EventStore::open(mode).await?);
        let (live_events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Ok(Arc::new(Self {
            store,
            live_events,
            replication_policy,
            identity,
        }))
    }

    /// Returns the relay's event store.
    pub fn store(&self) -> Arc<EventStore> {
        Arc::clone(&self.store)
    }

    async fn submit(&self, event: Event) -> Result<WriteResult, StoreError> {
        let result = self.store.accept(event.clone()).await?;
        if result.publish_live {
            let _ = self.live_events.send(event);
        }
        Ok(result)
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
        .route("/", get(websocket_upgrade))
        .route("/health", get(health))
        .route("/events", post(submit_event))
        .route("/query", post(query_events))
        .route("/count", post(count_events))
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
                method: "POST".to_string(),
                body: body.to_vec(),
            },
            &audience,
        )
        .await?;
    Ok(Some(principal))
}

async fn websocket_upgrade(
    ws: WebSocketUpgrade,
    State(relay): State<Arc<LocalRelay>>,
    headers: HeaderMap,
) -> Response {
    let host = headers
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
    ws.on_upgrade(move |socket| websocket_session(socket, relay, audience))
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
            Self::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Query(_) => StatusCode::BAD_REQUEST,
        };
        let code = match &self {
            Self::Identity(error) => error.denial_code().map(IdentityDenialCode::as_str),
            Self::BadRequest(_) | Self::Store(_) | Self::Query(_) => None,
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
