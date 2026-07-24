//! Encrypted NIP-CB signing, durable spool, owner view, and republish boundary.

use std::collections::BTreeSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;

use buzz_core_pkg::command_brief::{
    decrypt_command_brief_event, CommandBriefEventPayload, CommandBriefFailure,
    CommandBriefFailureCode, CommandBriefLifecycleState, CommandBriefWire,
    COMMAND_BRIEF_PAYLOAD_VERSION,
};
use chrono::Utc;
use nostr::{Event, JsonUtil, Keys};
use tauri::Manager;
use tokio_util::sync::CancellationToken;

use super::orchestrator::{BriefFuture, BriefPersistence, BriefPersistenceError};
use super::store::{
    insert_spool_event, latest_event_id, list_due_spool_events, mark_publish_failed,
    mark_publish_permanent, mark_published, open_command_brief_store, rearm_queued_spool_events,
    validate_due_spool_event, SpoolInsert,
};
use super::types::{CommandBrief, PublicationState, PublishedCommandBrief};

/// Boxed relay-publication future.
pub type AuditPublishFuture<'a> = Pin<Box<dyn Future<Output = Result<(), ()>> + Send + 'a>>;
/// Boxed deterministic pre-commit gate future.
pub type AuditCommitFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Deterministic boundary immediately before cancellation arbitration and the
/// real SQLite commit.
pub trait BriefAuditCommitGate: Send + Sync {
    /// Wait before entering the commit mutex.
    fn wait<'a>(&'a self) -> AuditCommitFuture<'a>;
}

struct ImmediateAuditCommitGate;

impl BriefAuditCommitGate for ImmediateAuditCommitGate {
    fn wait<'a>(&'a self) -> AuditCommitFuture<'a> {
        Box::pin(async {})
    }
}

/// Exact signed-event relay boundary. Implementations must not re-sign.
pub trait BriefAuditPublisher: Send + Sync {
    /// Publish one already-signed event, treating duplicate ID acceptance as success.
    fn publish<'a>(&'a self, event: Event) -> AuditPublishFuture<'a>;
}

/// Production publisher that submits the exact signed event through Buzz's
/// authenticated relay path.
pub struct RelayBriefAuditPublisher {
    app: tauri::AppHandle,
}

impl RelayBriefAuditPublisher {
    /// Bind publication to the running desktop application.
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl BriefAuditPublisher for RelayBriefAuditPublisher {
    fn publish<'a>(&'a self, event: Event) -> AuditPublishFuture<'a> {
        Box::pin(async move {
            let state = self.app.state::<crate::app_state::AppState>();
            crate::relay::submit_signed_event(&event, &state)
                .await
                .map(|_| ())
                .map_err(|_| ())
        })
    }
}

/// Redacted persistence error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BriefAuditError {
    /// Cooperative cancellation won before durable commit.
    Cancelled,
    /// Validation, signing, or durable local persistence failed.
    Failed,
}

/// Strict owned terminal input for every lifecycle outcome.
#[derive(Clone)]
pub struct TerminalAuditInput {
    run_id: String,
    schedule_id: String,
    occurred_at: String,
    snapshot_id: String,
    lifecycle_state: CommandBriefLifecycleState,
    final_brief: Option<CommandBrief>,
    failure_code: Option<CommandBriefFailureCode>,
}

impl TerminalAuditInput {
    /// Build a completed or visibly degraded terminal from a validated brief.
    pub fn completed(brief: CommandBrief) -> Self {
        let lifecycle_state = if brief.is_degraded() {
            CommandBriefLifecycleState::Degraded
        } else {
            CommandBriefLifecycleState::Completed
        };
        Self {
            run_id: brief.run_id().to_string(),
            schedule_id: brief.schedule_id().to_string(),
            occurred_at: brief.generated_at().to_string(),
            snapshot_id: brief.snapshot_id().to_string(),
            lifecycle_state,
            final_brief: Some(brief),
            failure_code: None,
        }
    }

    /// Build a redacted failed or cancelled terminal with no final brief.
    pub fn closed(
        run_id: String,
        schedule_id: String,
        occurred_at: String,
        snapshot_id: String,
        lifecycle_state: CommandBriefLifecycleState,
        failure_code: CommandBriefFailureCode,
    ) -> Result<Self, BriefAuditError> {
        if !matches!(
            lifecycle_state,
            CommandBriefLifecycleState::Cancelled | CommandBriefLifecycleState::Failed
        ) {
            return Err(BriefAuditError::Failed);
        }
        Ok(Self {
            run_id,
            schedule_id,
            occurred_at,
            snapshot_id,
            lifecycle_state,
            final_brief: None,
            failure_code: Some(failure_code),
        })
    }

    fn cancelled(mut self) -> Self {
        self.lifecycle_state = CommandBriefLifecycleState::Cancelled;
        self.final_brief = None;
        self.failure_code = Some(CommandBriefFailureCode::CancellationRequested);
        self
    }

    #[cfg(test)]
    pub(crate) fn run_id(&self) -> &str {
        &self.run_id
    }

    #[cfg(test)]
    pub(crate) const fn lifecycle_state(&self) -> CommandBriefLifecycleState {
        self.lifecycle_state
    }

    #[cfg(test)]
    pub(crate) fn into_cancelled(self) -> Self {
        self.cancelled()
    }

    #[cfg(test)]
    pub(crate) fn final_brief(&self) -> Option<&CommandBrief> {
        self.final_brief.as_ref()
    }

    #[cfg(test)]
    pub(crate) const fn failure_code(&self) -> Option<CommandBriefFailureCode> {
        self.failure_code
    }
}

/// Durable terminal result fixed by one signed event ID.
#[derive(Clone)]
pub struct PersistedTerminal {
    lifecycle_state: CommandBriefLifecycleState,
    event_id: String,
    publication_state: PublicationState,
    published_brief: Option<PublishedCommandBrief>,
}

impl PersistedTerminal {
    pub(crate) fn new(
        lifecycle_state: CommandBriefLifecycleState,
        event_id: String,
        publication_state: PublicationState,
        published_brief: Option<PublishedCommandBrief>,
    ) -> Self {
        Self {
            lifecycle_state,
            event_id,
            publication_state,
            published_brief,
        }
    }
    /// Return the closed durable lifecycle state.
    pub const fn lifecycle_state(&self) -> CommandBriefLifecycleState {
        self.lifecycle_state
    }

    /// Return the exact signed lifecycle event ID.
    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    /// Return relay publication state for this exact local event.
    pub const fn publication_state(&self) -> PublicationState {
        self.publication_state
    }

    /// Return the final brief only for completed/degraded terminals.
    pub fn published_brief(&self) -> Option<&PublishedCommandBrief> {
        self.published_brief.as_ref()
    }
}

/// Owner-bound encrypted audit service.
pub struct EncryptedBriefAudit {
    path: PathBuf,
    owner_keys: Keys,
    publisher: Arc<dyn BriefAuditPublisher>,
    commit_gate: Arc<dyn BriefAuditCommitGate>,
    committed_runs: Mutex<BTreeSet<String>>,
}

impl std::fmt::Debug for EncryptedBriefAudit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EncryptedBriefAudit")
            .finish_non_exhaustive()
    }
}

impl EncryptedBriefAudit {
    /// Construct an audit service bound to the currently unlocked owner.
    pub fn new(path: PathBuf, owner_keys: Keys, publisher: Arc<dyn BriefAuditPublisher>) -> Self {
        Self {
            path,
            owner_keys,
            publisher,
            commit_gate: Arc::new(ImmediateAuditCommitGate),
            committed_runs: Mutex::new(BTreeSet::new()),
        }
    }

    /// Construct with a deterministic test barrier immediately before commit.
    pub fn new_with_commit_gate(
        path: PathBuf,
        owner_keys: Keys,
        publisher: Arc<dyn BriefAuditPublisher>,
        commit_gate: Arc<dyn BriefAuditCommitGate>,
    ) -> Self {
        Self {
            path,
            owner_keys,
            publisher,
            commit_gate,
            committed_runs: Mutex::new(BTreeSet::new()),
        }
    }

    /// Atomically accept cancellation only while no terminal event has committed.
    pub fn request_cancel(&self, run_id: &str, cancellation: &CancellationToken) -> bool {
        let Ok(committed) = self.committed_runs.lock() else {
            return false;
        };
        if committed.contains(run_id) {
            return false;
        }
        cancellation.cancel();
        true
    }

    /// Persist one strict terminal input. Cancellation accepted before the
    /// commit mutex converts a success input into one cancelled terminal.
    pub async fn persist_terminal_input(
        &self,
        input: TerminalAuditInput,
        cancellation: CancellationToken,
    ) -> Result<PersistedTerminal, BriefAuditError> {
        self.commit_gate.wait().await;
        let (event, event_id, owner, lifecycle_state, final_brief) = {
            let mut committed = self
                .committed_runs
                .lock()
                .map_err(|_| BriefAuditError::Failed)?;
            if committed.contains(&input.run_id) {
                return Err(BriefAuditError::Failed);
            }
            let input = if cancellation.is_cancelled() {
                input.cancelled()
            } else {
                input
            };
            let owner = self.owner_keys.public_key().to_hex();
            let conn = open_command_brief_store(&self.path).map_err(|_| BriefAuditError::Failed)?;
            let previous = latest_event_id(&conn, &owner, &input.run_id)
                .map_err(|_| BriefAuditError::Failed)?;
            let final_brief_wire = input
                .final_brief
                .as_ref()
                .map(|brief| {
                    serde_json::to_value(brief)
                        .map_err(|_| BriefAuditError::Failed)
                        .and_then(|value| {
                            CommandBriefWire::try_from(value).map_err(|_| BriefAuditError::Failed)
                        })
                })
                .transpose()?;
            let payload = CommandBriefEventPayload {
                version: COMMAND_BRIEF_PAYLOAD_VERSION,
                classification: "OFFICIAL".into(),
                run_id: input.run_id.clone(),
                schedule_id: input.schedule_id,
                lifecycle_state: input.lifecycle_state,
                occurred_at: input.occurred_at,
                frozen_snapshot_id: input.snapshot_id,
                final_brief: final_brief_wire,
                failure: input.failure_code.map(|code| CommandBriefFailure { code }),
                previous_lifecycle_event_id: previous.clone(),
            };
            let event =
                crate::events::build_command_brief_lifecycle_event(&self.owner_keys, &payload)
                    .map_err(|_| BriefAuditError::Failed)?;
            let event_id = event.id.to_hex();
            insert_spool_event(
                &conn,
                SpoolInsert {
                    owner_pubkey: owner.clone(),
                    run_id: input.run_id.clone(),
                    event_id: event_id.clone(),
                    status: lifecycle_label(input.lifecycle_state).to_string(),
                    previous_event_id: previous,
                    encrypted_payload: event.content.clone(),
                    raw_event: event.as_json(),
                    created_at: event.created_at.as_secs() as i64,
                },
            )
            .map_err(|_| BriefAuditError::Failed)?;
            committed.insert(input.run_id);
            (
                event,
                event_id,
                owner,
                input.lifecycle_state,
                input.final_brief,
            )
        };

        let publication_state = if self.publisher.publish(event).await.is_ok() {
            let marked = open_command_brief_store(&self.path)
                .and_then(|conn| mark_published(&conn, &owner, &event_id, Utc::now().timestamp()));
            if marked.is_ok() {
                PublicationState::Published
            } else {
                // The signed terminal commit is authoritative. A publication
                // bookkeeping failure must not turn that durable terminal into
                // a false local persistence failure; the queued row is safe to
                // retry idempotently by exact event ID.
                PublicationState::Queued
            }
        } else {
            if let Ok(conn) = open_command_brief_store(&self.path) {
                let _ = mark_publish_failed(&conn, &owner, &event_id, Utc::now().timestamp());
            }
            PublicationState::Queued
        };
        let published_brief = final_brief
            .map(|brief| PublishedCommandBrief::new(brief, event_id.clone(), publication_state));
        Ok(PersistedTerminal::new(
            lifecycle_state,
            event_id,
            publication_state,
            published_brief,
        ))
    }

    /// Encrypt, sign, durably spool, and best-effort publish a terminal brief.
    ///
    /// Relay failure returns a valid queued envelope: local completion remains
    /// durable and reconnect republishes the exact same signed event ID.
    pub async fn persist_terminal(
        &self,
        brief: &CommandBrief,
        cancellation: CancellationToken,
    ) -> Result<PublishedCommandBrief, BriefAuditError> {
        let terminal = self
            .persist_terminal_input(TerminalAuditInput::completed(brief.clone()), cancellation)
            .await?;
        terminal.published_brief.ok_or(BriefAuditError::Cancelled)
    }

    /// Republish a bounded due batch for only this unlocked owner.
    pub async fn republish_due(&self, now: i64) -> Result<usize, BriefAuditError> {
        let owner = self.owner_keys.public_key().to_hex();
        let rows = {
            let conn = open_command_brief_store(&self.path).map_err(|_| BriefAuditError::Failed)?;
            list_due_spool_events(&conn, &owner, now, 64).map_err(|_| BriefAuditError::Failed)?
        };
        let mut published = 0;
        for row in rows {
            let event = match validate_due_spool_event(&row) {
                Ok(event) if event.pubkey.to_hex() == owner => event,
                _ => {
                    let conn = open_command_brief_store(&self.path)
                        .map_err(|_| BriefAuditError::Failed)?;
                    mark_publish_permanent(&conn, &owner, &row.event_id)
                        .map_err(|_| BriefAuditError::Failed)?;
                    continue;
                }
            };
            let accepted = self.publisher.publish(event).await.is_ok();
            let conn = open_command_brief_store(&self.path).map_err(|_| BriefAuditError::Failed)?;
            if accepted {
                mark_published(&conn, &owner, &row.event_id, Utc::now().timestamp())
                    .map_err(|_| BriefAuditError::Failed)?;
                published += 1;
            } else {
                mark_publish_failed(&conn, &owner, &row.event_id, now)
                    .map_err(|_| BriefAuditError::Failed)?;
            }
        }
        Ok(published)
    }

    /// Rearm one bounded owner batch on relay readiness, then republish exact IDs.
    pub async fn recover_on_relay_ready(&self, now: i64) -> Result<usize, BriefAuditError> {
        let owner = self.owner_keys.public_key().to_hex();
        {
            let conn = open_command_brief_store(&self.path).map_err(|_| BriefAuditError::Failed)?;
            rearm_queued_spool_events(&conn, &owner, now, 64)
                .map_err(|_| BriefAuditError::Failed)?;
        }
        self.republish_due(now).await
    }

    /// Decrypt and return only the authoritative validated brief view model.
    pub fn decrypt_for_current_owner(
        &self,
        event: &Event,
    ) -> Result<CommandBrief, BriefAuditError> {
        let payload = decrypt_command_brief_event(&self.owner_keys, event)
            .map_err(|_| BriefAuditError::Failed)?;
        let value = payload.final_brief.ok_or(BriefAuditError::Failed)?;
        CommandBrief::try_from(value.into_value()).map_err(|_| BriefAuditError::Failed)
    }
}

fn lifecycle_label(state: CommandBriefLifecycleState) -> &'static str {
    match state {
        CommandBriefLifecycleState::Completed => "completed",
        CommandBriefLifecycleState::Degraded => "degraded",
        CommandBriefLifecycleState::Cancelled => "cancelled",
        CommandBriefLifecycleState::Failed => "failed",
    }
}

impl BriefPersistence for EncryptedBriefAudit {
    fn persist_terminal<'a>(
        &'a self,
        input: TerminalAuditInput,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<PersistedTerminal, BriefPersistenceError>> {
        Box::pin(async move {
            self.persist_terminal_input(input, cancellation)
                .await
                .map_err(|error| match error {
                    BriefAuditError::Cancelled => BriefPersistenceError::Cancelled,
                    BriefAuditError::Failed => BriefPersistenceError::Failed,
                })
        })
    }

    fn request_cancel(&self, run_id: &str, cancellation: &CancellationToken) -> bool {
        EncryptedBriefAudit::request_cancel(self, run_id, cancellation)
    }
}
