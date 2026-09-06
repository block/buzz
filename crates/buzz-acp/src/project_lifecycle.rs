//! Event-driven project lifecycle wakeups for ACP agents.
//!
//! A merged NIP-34 status event is global, while ACP sessions are channel
//! scoped. This module resolves the status event's root pull request, validates
//! that the current agent authored it, and recovers the originating channel
//! from the pull request's signed `h` tag. Only then does it emit a routed
//! [`BuzzEvent`](crate::relay::BuzzEvent) for normal ACP admission and queueing.
//!
//! Delivery state is stored as encrypted NIP-78 application data. A merge is
//! added to `pending` before it is handed to the queue and moved to `completed`
//! only after the ACP turn succeeds (or normal admission deliberately ignores
//! it). This makes reconnect/restart recovery at-least-once without polling.

use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use buzz_core::kind::{KIND_GIT_PULL_REQUEST, KIND_GIT_STATUS_MERGED, KIND_READ_STATE};
use nostr::nips::nip44::{self, Version};
use nostr::{Event, EventBuilder, EventId, JsonUtil, Keys, Kind, PublicKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::relay::{BuzzEvent, RelayError, RestClient};

/// Prompt type attached only after a merge event passes lifecycle validation.
pub const PROMPT_TAG: &str = "pull-request-merged";

const LEDGER_D_TAG: &str = "buzz-acp:project-lifecycle:v1";
const LEDGER_VERSION: u8 = 1;
const MAX_PENDING: usize = 32;
const MAX_COMPLETED: usize = 128;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(300);
const OUTPUT_CAPACITY: usize = 64;
const COMPLETION_CAPACITY: usize = 64;

/// A successfully validated merge routed to the pull request's source channel.
#[derive(Debug)]
pub struct MergeWakeup {
    /// Routed status event. Its `channel_id` comes from the signed root PR.
    pub buzz_event: BuzzEvent,
}

/// Events emitted by the addressed-event processor.
#[derive(Debug)]
pub enum AddressedEvent {
    /// An observer frame, unchanged; the existing observer authorization path
    /// still decides whether it is actionable.
    ObserverControl(Event),
    /// A validated and durably-pending pull-request merge.
    Merge(MergeWakeup),
}

/// Final disposition reported by normal ACP admission/processing.
#[derive(Debug, Clone, Copy)]
pub enum Disposition {
    /// The ACP turn completed successfully.
    Completed,
    /// Normal policy deliberately ignored the wakeup (for example, the agent
    /// is no longer subscribed to the originating channel).
    Ignored,
}

#[derive(Debug)]
struct Completion {
    event_ids: Vec<String>,
    disposition: Disposition,
}

/// Cloneable completion handle retained by the ACP main loop.
#[derive(Clone)]
pub struct LifecycleHandle {
    completion_tx: mpsc::Sender<Completion>,
}

impl LifecycleHandle {
    /// Mark lifecycle event IDs complete after their ACP turn succeeds.
    ///
    /// A full/closed channel is returned as an error instead of silently losing
    /// the completion. The encrypted `pending` record remains durable, so a
    /// process restart will retry the wakeup.
    pub fn complete(&self, event_ids: Vec<String>) -> Result<(), LifecycleError> {
        self.send_disposition(event_ids, Disposition::Completed)
    }

    /// Mark a wakeup deliberately ignored by normal admission policy.
    pub fn ignore(&self, event_id: String) -> Result<(), LifecycleError> {
        self.ignore_many(vec![event_id])
    }

    /// Mark event IDs deliberately removed by normal queue/channel policy.
    /// Ordinary event IDs are harmless: the manager only completes IDs present
    /// in its lifecycle `pending` set.
    pub fn ignore_many(&self, event_ids: Vec<String>) -> Result<(), LifecycleError> {
        self.send_disposition(event_ids, Disposition::Ignored)
    }

    fn send_disposition(
        &self,
        event_ids: Vec<String>,
        disposition: Disposition,
    ) -> Result<(), LifecycleError> {
        if event_ids.is_empty() {
            return Ok(());
        }
        self.completion_tx
            .try_send(Completion {
                event_ids,
                disposition,
            })
            .map_err(|error| LifecycleError::CompletionChannel(error.to_string()))
    }
}

/// Lifecycle validation or persistence failure.
#[derive(Debug, Error)]
pub enum LifecycleError {
    /// Relay HTTP bridge failure.
    #[error(transparent)]
    Relay(#[from] RelayError),
    /// Encrypted lifecycle state could not be decoded.
    #[error("invalid project lifecycle ledger: {0}")]
    Ledger(String),
    /// A completion could not be handed to the lifecycle manager.
    #[error("project lifecycle completion channel unavailable: {0}")]
    CompletionChannel(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PendingEntry {
    event_id: String,
    semantic_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CompletedEntry {
    event_id: String,
    semantic_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Ledger {
    version: u8,
    pending: Vec<PendingEntry>,
    completed: Vec<CompletedEntry>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            version: LEDGER_VERSION,
            pending: Vec::new(),
            completed: Vec::new(),
        }
    }
}

impl Ledger {
    fn validate(&self) -> Result<(), LifecycleError> {
        if self.version != LEDGER_VERSION {
            return Err(LifecycleError::Ledger(format!(
                "unsupported version {}",
                self.version
            )));
        }
        if self.pending.len() > MAX_PENDING || self.completed.len() > MAX_COMPLETED {
            return Err(LifecycleError::Ledger("entry bound exceeded".into()));
        }
        let mut event_ids = HashSet::new();
        let mut semantic_keys = HashSet::new();
        for entry in self
            .pending
            .iter()
            .map(|entry| (&entry.event_id, &entry.semantic_key))
            .chain(
                self.completed
                    .iter()
                    .map(|entry| (&entry.event_id, &entry.semantic_key)),
            )
        {
            validate_hex(entry.0, 64, "ledger event id")?;
            validate_hex(entry.1, 64, "ledger semantic key")?;
            if !event_ids.insert(entry.0) {
                return Err(LifecycleError::Ledger("duplicate event id".into()));
            }
            if !semantic_keys.insert(entry.1) {
                return Err(LifecycleError::Ledger("duplicate semantic key".into()));
            }
        }
        Ok(())
    }

    fn contains_event(&self, event_id: &str) -> bool {
        self.pending.iter().any(|entry| entry.event_id == event_id)
            || self
                .completed
                .iter()
                .any(|entry| entry.event_id == event_id)
    }

    fn contains_semantic(&self, semantic_key: &str) -> bool {
        self.pending
            .iter()
            .any(|entry| entry.semantic_key == semantic_key)
            || self
                .completed
                .iter()
                .any(|entry| entry.semantic_key == semantic_key)
    }

    fn add_pending(&mut self, event_id: String, semantic_key: String) -> bool {
        if self.contains_event(&event_id) || self.contains_semantic(&semantic_key) {
            return false;
        }
        if self.pending.len() >= MAX_PENDING {
            return false;
        }
        self.pending.push(PendingEntry {
            event_id,
            semantic_key,
        });
        true
    }

    fn complete(&mut self, event_id: &str) -> bool {
        let Some(index) = self
            .pending
            .iter()
            .position(|entry| entry.event_id == event_id)
        else {
            return false;
        };
        let entry = self.pending.remove(index);
        self.completed.push(CompletedEntry {
            event_id: entry.event_id,
            semantic_key: entry.semantic_key,
        });
        if self.completed.len() > MAX_COMPLETED {
            let overflow = self.completed.len() - MAX_COMPLETED;
            self.completed.drain(..overflow);
        }
        true
    }
}

#[derive(Debug)]
struct RetryWork {
    event_id: String,
    event: Option<Event>,
    expected_semantic_key: Option<String>,
    attempts: u32,
    retry_at: tokio::time::Instant,
}

impl RetryWork {
    fn live(event: Event) -> Self {
        Self {
            event_id: event.id.to_hex(),
            event: Some(event),
            expected_semantic_key: None,
            attempts: 0,
            retry_at: tokio::time::Instant::now(),
        }
    }

    fn recovered(entry: &PendingEntry) -> Self {
        Self {
            event_id: entry.event_id.clone(),
            event: None,
            expected_semantic_key: Some(entry.semantic_key.clone()),
            attempts: 0,
            retry_at: tokio::time::Instant::now(),
        }
    }

    fn defer(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
        self.retry_at = tokio::time::Instant::now() + retry_delay(self.attempts);
    }
}

struct Manager {
    keys: Keys,
    rest: RestClient,
    ledger: Ledger,
    ledger_created_at: u64,
    dispatched: HashSet<String>,
    work: VecDeque<RetryWork>,
    pending_completions: VecDeque<Completion>,
    completion_retry_at: tokio::time::Instant,
    completion_retry_attempts: u32,
    addressed_rx: mpsc::Receiver<Event>,
    output_tx: mpsc::Sender<AddressedEvent>,
    completion_rx: mpsc::Receiver<Completion>,
}

/// Start the lifecycle manager over the relay's global addressed-event stream.
///
/// Loading the encrypted ledger is a startup boundary: corrupt or unavailable
/// state fails closed rather than risking duplicate model turns. Once started,
/// transient resolution/persistence failures remain in a bounded retry queue
/// with capped exponential backoff.
pub async fn start(
    keys: Keys,
    rest: RestClient,
    addressed_rx: mpsc::Receiver<Event>,
) -> Result<
    (
        LifecycleHandle,
        mpsc::Receiver<AddressedEvent>,
        tokio::task::JoinHandle<()>,
    ),
    LifecycleError,
> {
    let (ledger, ledger_created_at) = load_ledger(&rest, &keys).await?;
    let mut work = VecDeque::with_capacity(ledger.pending.len());
    for pending in &ledger.pending {
        work.push_back(RetryWork::recovered(pending));
    }

    let (output_tx, output_rx) = mpsc::channel(OUTPUT_CAPACITY);
    let (completion_tx, completion_rx) = mpsc::channel(COMPLETION_CAPACITY);
    let handle = LifecycleHandle { completion_tx };
    let mut manager = Manager {
        keys,
        rest,
        ledger,
        ledger_created_at,
        dispatched: HashSet::new(),
        work,
        pending_completions: VecDeque::new(),
        completion_retry_at: tokio::time::Instant::now(),
        completion_retry_attempts: 0,
        addressed_rx,
        output_tx,
        completion_rx,
    };
    let task = tokio::spawn(async move { manager.run().await });
    Ok((handle, output_rx, task))
}

impl Manager {
    async fn run(&mut self) {
        loop {
            let retry_at = self.next_retry_at();
            tokio::select! {
                biased;
                completion = self.completion_rx.recv() => {
                    match completion {
                        Some(completion) => self.pending_completions.push_back(completion),
                        None => {
                            warn!("project lifecycle completion channel closed");
                            return;
                        }
                    }
                }
                _ = async move {
                    match retry_at {
                        Some(retry_at) => tokio::time::sleep_until(retry_at).await,
                        None => std::future::pending().await,
                    }
                } => {}
                event = self.addressed_rx.recv(), if self.ledger.pending.len() + self.work.len() < MAX_PENDING => {
                    match event {
                        Some(event) if is_merge_status(&event) => self.accept_raw_merge(event),
                        Some(event) => {
                            if self.output_tx.send(AddressedEvent::ObserverControl(event)).await.is_err() {
                                warn!("project lifecycle output channel closed");
                                return;
                            }
                        }
                        None => {
                            warn!("addressed relay event channel closed");
                            return;
                        }
                    }
                }
            }

            self.flush_one_completion().await;
            if !self.process_one_work_item().await {
                return;
            }
        }
    }

    fn next_retry_at(&self) -> Option<tokio::time::Instant> {
        let work_retry = self.work.iter().map(|work| work.retry_at).min();
        let completion_retry =
            (!self.pending_completions.is_empty()).then_some(self.completion_retry_at);
        match (work_retry, completion_retry) {
            (Some(work), Some(completion)) => Some(work.min(completion)),
            (Some(work), None) => Some(work),
            (None, Some(completion)) => Some(completion),
            (None, None) => None,
        }
    }

    fn accept_raw_merge(&mut self, event: Event) {
        let event_id = event.id.to_hex();
        if self.ledger.contains_event(&event_id)
            || self.dispatched.contains(&event_id)
            || self.work.iter().any(|work| work.event_id == event_id)
        {
            debug!(event_id, "duplicate project lifecycle event suppressed");
            return;
        }
        self.work.push_back(RetryWork::live(event));
    }

    async fn flush_one_completion(&mut self) {
        if self.completion_retry_at > tokio::time::Instant::now() {
            return;
        }
        let Some(completion) = self.pending_completions.pop_front() else {
            return;
        };
        let mut next = self.ledger.clone();
        let mut changed = false;
        for event_id in &completion.event_ids {
            changed |= next.complete(event_id);
        }
        if !changed {
            return;
        }
        match persist_ledger(&self.rest, &self.keys, &next, self.ledger_created_at).await {
            Ok(created_at) => {
                for event_id in &completion.event_ids {
                    self.dispatched.remove(event_id);
                }
                self.ledger = next;
                self.ledger_created_at = created_at;
                self.completion_retry_attempts = 0;
                self.completion_retry_at = tokio::time::Instant::now();
                info!(
                    disposition = ?completion.disposition,
                    events = completion.event_ids.len(),
                    "persisted project lifecycle completion"
                );
            }
            Err(error) => {
                self.completion_retry_attempts = self.completion_retry_attempts.saturating_add(1);
                self.completion_retry_at =
                    tokio::time::Instant::now() + retry_delay(self.completion_retry_attempts);
                warn!(
                    %error,
                    attempts = self.completion_retry_attempts,
                    "failed to persist project lifecycle completion — will retry"
                );
                self.pending_completions.push_front(completion);
            }
        }
    }

    async fn process_one_work_item(&mut self) -> bool {
        let Some(index) = self
            .work
            .iter()
            .position(|work| work.retry_at <= tokio::time::Instant::now())
        else {
            return true;
        };
        let Some(mut work) = self.work.remove(index) else {
            return true;
        };

        let event = match work.event.take() {
            Some(event) => event,
            None => {
                match fetch_event_by_id(&self.rest, &work.event_id, KIND_GIT_STATUS_MERGED).await {
                    Ok(event) => event,
                    Err(error) => {
                        warn!(event_id = %work.event_id, %error, "failed to recover pending merge status — will retry");
                        work.defer();
                        self.work.push_back(work);
                        return true;
                    }
                }
            }
        };

        let resolved = match resolve_merge(&event, &self.rest, &self.keys.public_key()).await {
            Ok(resolved) => resolved,
            Err(ResolveFailure::Invalid(reason)) if work.expected_semantic_key.is_none() => {
                warn!(event_id = %work.event_id, %reason, "rejected project lifecycle event");
                return true;
            }
            Err(error) => {
                warn!(event_id = %work.event_id, %error, "project lifecycle resolution failed — will retry");
                work.event = Some(event);
                work.defer();
                self.work.push_back(work);
                return true;
            }
        };

        if let Some(expected) = &work.expected_semantic_key {
            if expected != &resolved.semantic_key {
                error!(
                    event_id = %work.event_id,
                    "recovered project lifecycle event changed semantic identity — retaining pending record"
                );
                work.event = Some(event);
                work.defer();
                self.work.push_back(work);
                return true;
            }
        } else {
            if self.ledger.contains_semantic(&resolved.semantic_key) {
                debug!(event_id = %work.event_id, "semantic duplicate merge suppressed");
                return true;
            }
            let mut next = self.ledger.clone();
            if !next.add_pending(work.event_id.clone(), resolved.semantic_key.clone()) {
                warn!(event_id = %work.event_id, "project lifecycle pending ledger full — will retry");
                work.event = Some(event);
                work.defer();
                self.work.push_back(work);
                return true;
            }
            match persist_ledger(&self.rest, &self.keys, &next, self.ledger_created_at).await {
                Ok(created_at) => {
                    self.ledger = next;
                    self.ledger_created_at = created_at;
                }
                Err(error) => {
                    warn!(event_id = %work.event_id, %error, "failed to persist pending merge — will retry");
                    work.event = Some(event);
                    work.defer();
                    self.work.push_back(work);
                    return true;
                }
            }
        }

        let event_id = work.event_id;
        self.dispatched.insert(event_id.clone());
        if self
            .output_tx
            .send(AddressedEvent::Merge(MergeWakeup {
                buzz_event: BuzzEvent {
                    connection_generation: 0,
                    channel_id: resolved.channel_id,
                    event,
                },
            }))
            .await
            .is_err()
        {
            warn!(
                event_id,
                "project lifecycle output channel closed; pending record retained"
            );
            return false;
        }
        true
    }
}

#[derive(Debug)]
struct ResolvedMerge {
    channel_id: Uuid,
    semantic_key: String,
}

#[derive(Debug, Error)]
enum ResolveFailure {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Relay(#[from] RelayError),
}

async fn resolve_merge(
    status: &Event,
    rest: &RestClient,
    agent_pubkey: &PublicKey,
) -> Result<ResolvedMerge, ResolveFailure> {
    let status_meta = validate_status(status, agent_pubkey).map_err(ResolveFailure::Invalid)?;
    let pull_request = fetch_event_by_id(rest, &status_meta.pull_request_id, KIND_GIT_PULL_REQUEST)
        .await
        .map_err(ResolveFailure::Relay)?;
    validate_pull_request(&pull_request, status, &status_meta, agent_pubkey)
        .map_err(ResolveFailure::Invalid)
}

#[derive(Debug)]
struct StatusMeta {
    pull_request_id: String,
    repository: String,
    repository_owner: String,
    merge_commit: String,
}

fn validate_status(status: &Event, agent_pubkey: &PublicKey) -> Result<StatusMeta, String> {
    if !is_merge_status(status) {
        return Err("wrong event kind".into());
    }
    if !status.content.is_empty() {
        return Err("merged status content must be empty".into());
    }
    let pull_request_id = single_root_event_id(status)?;
    let repository = single_tag_value(status, "a")?;
    let repository_owner = parse_repo_owner(&repository)?;
    if status.pubkey.to_hex() != repository_owner {
        return Err("merged status is not signed by the repository owner".into());
    }
    let merge_commit = single_tag_value(status, "merge-commit")?;
    validate_commit(&merge_commit)?;
    if !tag_values(status, "r").any(|value| value == merge_commit) {
        return Err("merged status lacks matching r tag".into());
    }
    let agent_hex = agent_pubkey.to_hex();
    let recipients: HashSet<&str> = tag_values(status, "p").collect();
    if !recipients.contains(agent_hex.as_str()) {
        return Err("merged status is not addressed to this agent".into());
    }
    Ok(StatusMeta {
        pull_request_id,
        repository,
        repository_owner,
        merge_commit,
    })
}

fn validate_pull_request(
    pull_request: &Event,
    status: &Event,
    status_meta: &StatusMeta,
    agent_pubkey: &PublicKey,
) -> Result<ResolvedMerge, String> {
    if pull_request.id.to_hex() != status_meta.pull_request_id {
        return Err("root pull request ID mismatch".into());
    }
    if pull_request.kind.as_u16() as u32 != KIND_GIT_PULL_REQUEST {
        return Err("root event is not a pull request".into());
    }
    if &pull_request.pubkey != agent_pubkey {
        return Err("root pull request was not authored by this agent".into());
    }
    if pull_request.created_at > status.created_at {
        return Err("merged status predates its pull request".into());
    }
    if single_tag_value(pull_request, "a")? != status_meta.repository {
        return Err("pull request repository does not match merged status".into());
    }
    if pull_request.pubkey.to_hex() != status_meta.repository_owner
        && !tag_values(pull_request, "p").any(|value| value == status_meta.repository_owner)
    {
        return Err("pull request does not address the repository owner".into());
    }
    let channel = single_tag_value(pull_request, "h")?;
    let channel_id = Uuid::parse_str(&channel)
        .map_err(|_| "pull request h tag is not a canonical channel UUID".to_string())?;
    if channel_id.to_string() != channel {
        return Err("pull request h tag is not canonical".into());
    }
    let commit = single_tag_value(pull_request, "c")?;
    validate_commit(&commit)?;

    Ok(ResolvedMerge {
        channel_id,
        semantic_key: semantic_key(
            &status_meta.repository,
            &status_meta.pull_request_id,
            &status_meta.merge_commit,
        ),
    })
}

fn is_merge_status(event: &Event) -> bool {
    event.kind.as_u16() as u32 == KIND_GIT_STATUS_MERGED
}

fn single_root_event_id(event: &Event) -> Result<String, String> {
    let roots: Vec<&str> = event
        .tags
        .iter()
        .filter_map(|tag| {
            let fields = tag.as_slice();
            (fields.first().map(String::as_str) == Some("e")
                && fields.get(3).map(String::as_str) == Some("root"))
            .then(|| fields.get(1).map(String::as_str))
            .flatten()
        })
        .collect();
    if roots.len() != 1 {
        return Err("merged status must have exactly one root e tag".into());
    }
    validate_lower_hex(roots[0], 64, "pull request event id")?;
    Ok(roots[0].to_string())
}

fn single_tag_value(event: &Event, name: &str) -> Result<String, String> {
    let values: Vec<&str> = tag_values(event, name).collect();
    if values.len() != 1 {
        return Err(format!("expected exactly one {name} tag"));
    }
    Ok(values[0].to_string())
}

fn tag_values<'a>(event: &'a Event, name: &'a str) -> impl Iterator<Item = &'a str> {
    event.tags.iter().filter_map(move |tag| {
        let fields = tag.as_slice();
        (fields.first().map(String::as_str) == Some(name))
            .then(|| fields.get(1).map(String::as_str))
            .flatten()
    })
}

fn parse_repo_owner(repository: &str) -> Result<String, String> {
    let mut parts = repository.splitn(3, ':');
    if parts.next() != Some("30617") {
        return Err("repository coordinate must start with 30617".into());
    }
    let owner = parts
        .next()
        .ok_or_else(|| "repository coordinate lacks owner".to_string())?;
    validate_lower_hex(owner, 64, "repository owner")?;
    PublicKey::from_hex(owner).map_err(|_| "repository owner is not a valid pubkey".to_string())?;
    let identifier = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "repository coordinate lacks identifier".to_string())?;
    if identifier.chars().any(char::is_control) {
        return Err("repository identifier contains control characters".into());
    }
    Ok(owner.to_string())
}

fn validate_commit(value: &str) -> Result<(), String> {
    if value.len() != 40 && value.len() != 64 {
        return Err("merge commit must be a full SHA-1 or SHA-256 object ID".into());
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("merge commit must be lowercase hexadecimal".into());
    }
    Ok(())
}

fn validate_lower_hex(value: &str, length: usize, label: &str) -> Result<(), String> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "{label} must be {length} lowercase hexadecimal characters"
        ));
    }
    Ok(())
}

fn validate_hex(value: &str, length: usize, label: &str) -> Result<(), LifecycleError> {
    validate_lower_hex(value, length, label).map_err(LifecycleError::Ledger)
}

fn retry_delay(attempts: u32) -> Duration {
    let exponent = attempts.saturating_sub(1).min(6);
    let seconds = 5_u64.saturating_mul(1_u64 << exponent);
    Duration::from_secs(seconds).min(MAX_RETRY_DELAY)
}

fn semantic_key(repository: &str, pull_request_id: &str, merge_commit: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(repository.as_bytes());
    hash.update([0]);
    hash.update(pull_request_id.as_bytes());
    hash.update([0]);
    hash.update(merge_commit.as_bytes());
    hex::encode(hash.finalize())
}

async fn fetch_event_by_id(
    rest: &RestClient,
    event_id: &str,
    kind: u32,
) -> Result<Event, RelayError> {
    let event_id = EventId::from_hex(event_id)
        .map_err(|error| RelayError::Http(format!("invalid event ID: {error}")))?;
    let filter = nostr::Filter::new()
        .kind(Kind::Custom(kind as u16))
        .id(event_id)
        .limit(2);
    let response = rest.query(&[filter]).await?;
    let events = response
        .as_array()
        .ok_or_else(|| RelayError::Http("event lookup response is not an array".into()))?;
    if events.len() != 1 {
        return Err(RelayError::Http(format!(
            "event lookup returned {} events, expected exactly one",
            events.len()
        )));
    }
    let event = Event::from_json(events[0].to_string()).map_err(|error| {
        RelayError::Http(format!("event lookup returned invalid event: {error}"))
    })?;
    buzz_core::verify_event(&event)
        .map_err(|error| RelayError::Http(format!("event lookup signature invalid: {error}")))?;
    Ok(event)
}

async fn load_ledger(rest: &RestClient, keys: &Keys) -> Result<(Ledger, u64), LifecycleError> {
    use nostr::{Alphabet, SingleLetterTag};

    let filter = nostr::Filter::new()
        .kind(Kind::Custom(KIND_READ_STATE as u16))
        .author(keys.public_key())
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [LEDGER_D_TAG])
        .limit(2);
    let response = rest.query(&[filter]).await?;
    let values = response
        .as_array()
        .ok_or_else(|| LifecycleError::Ledger("query response is not an array".into()))?;
    if values.is_empty() {
        return Ok((Ledger::default(), 0));
    }

    let mut events = Vec::with_capacity(values.len());
    for value in values {
        let event = Event::from_json(value.to_string())
            .map_err(|error| LifecycleError::Ledger(format!("invalid event JSON: {error}")))?;
        buzz_core::verify_event(&event)
            .map_err(|error| LifecycleError::Ledger(format!("invalid event signature: {error}")))?;
        if event.kind.as_u16() as u32 != KIND_READ_STATE
            || event.pubkey != keys.public_key()
            || single_tag_value(&event, "d").map_err(LifecycleError::Ledger)? != LEDGER_D_TAG
        {
            return Err(LifecycleError::Ledger(
                "query returned an event outside the lifecycle ledger address".into(),
            ));
        }
        events.push(event);
    }
    events.sort_by_key(|event| (event.created_at.as_secs(), event.id.to_hex()));
    let event = events
        .pop()
        .ok_or_else(|| LifecycleError::Ledger("ledger event disappeared".into()))?;
    let ledger = decode_ledger_event(&event, keys)?;
    Ok((ledger, event.created_at.as_secs()))
}

fn decode_ledger_event(event: &Event, keys: &Keys) -> Result<Ledger, LifecycleError> {
    buzz_core::verify_event(event)
        .map_err(|error| LifecycleError::Ledger(format!("invalid event signature: {error}")))?;
    if event.kind.as_u16() as u32 != KIND_READ_STATE
        || event.pubkey != keys.public_key()
        || single_tag_value(event, "d").map_err(LifecycleError::Ledger)? != LEDGER_D_TAG
    {
        return Err(LifecycleError::Ledger(
            "event is outside the lifecycle ledger address".into(),
        ));
    }
    let plaintext = nip44::decrypt(keys.secret_key(), &keys.public_key(), &event.content)
        .map_err(|error| LifecycleError::Ledger(format!("decrypt failed: {error}")))?;
    let ledger: Ledger = serde_json::from_str(&plaintext)
        .map_err(|error| LifecycleError::Ledger(format!("JSON decode failed: {error}")))?;
    ledger.validate()?;
    Ok(ledger)
}

fn build_ledger_event(
    keys: &Keys,
    ledger: &Ledger,
    previous_created_at: u64,
) -> Result<(Event, u64), LifecycleError> {
    ledger.validate()?;
    let plaintext = serde_json::to_string(ledger)
        .map_err(|error| LifecycleError::Ledger(format!("JSON encode failed: {error}")))?;
    let ciphertext = nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        plaintext,
        Version::V2,
    )
    .map_err(|error| LifecycleError::Ledger(format!("encrypt failed: {error}")))?;
    let created_at = Timestamp::now()
        .as_secs()
        .max(previous_created_at.saturating_add(1));
    let d_tag = Tag::parse(["d", LEDGER_D_TAG])
        .map_err(|error| LifecycleError::Ledger(format!("d tag failed: {error}")))?;
    let event = EventBuilder::new(Kind::Custom(KIND_READ_STATE as u16), ciphertext)
        .tag(d_tag)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .map_err(|error| LifecycleError::Ledger(format!("sign failed: {error}")))?;
    Ok((event, created_at))
}

async fn persist_ledger(
    rest: &RestClient,
    keys: &Keys,
    ledger: &Ledger,
    previous_created_at: u64,
) -> Result<u64, LifecycleError> {
    let (event, created_at) = build_ledger_event(keys, ledger, previous_created_at)?;
    let response = rest.submit_event(&event).await?;
    if response
        .get("accepted")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return Err(LifecycleError::Ledger(format!(
            "relay rejected ledger event: {}",
            response
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown rejection")
        )));
    }
    Ok(created_at)
}

/// Extract lifecycle event IDs from a dispatched batch.
pub fn batch_event_ids(batch: &crate::queue::FlushBatch) -> Vec<String> {
    batch
        .cancelled_events
        .iter()
        .chain(batch.events.iter())
        .filter(|event| event.prompt_tag == PROMPT_TAG)
        .map(|event| event.event.id.to_hex())
        .collect()
}

/// Render trusted lifecycle guidance for a validated merged-status event.
pub fn prompt_guidance(event: &Event, prompt_tag: &str) -> Option<String> {
    if prompt_tag != PROMPT_TAG || !is_merge_status(event) {
        return None;
    }
    let pull_request_id = single_root_event_id(event).ok()?;
    let repository = single_tag_value(event, "a").ok()?;
    let merge_commit = single_tag_value(event, "merge-commit").ok()?;
    Some(format!(
        "Lifecycle: pull request merged successfully\n\
         Pull request: {pull_request_id}\n\
         Repository: {repository}\n\
         Merge commit: {merge_commit}\n\
         Instruction: This is a verified lifecycle wakeup for a pull request you authored, not a new human request. Resume ownership of the work now that it has merged: inspect the pull request context and perform any useful post-merge follow-up. Do not post a generic acknowledgement. The merged-status event is global and cannot be a channel-message reply parent; post at channel level, or reply to an original channel message only if you can identify one. Send an update only when there is concrete follow-up or information worth sharing."
    ))
}

#[cfg(test)]
mod tests;
