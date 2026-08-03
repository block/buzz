//! SQLite recovery state owned by one dedicated thread.
use crate::protocol::{
    AssignmentRecord, AssignmentSetStateRequest, AssignmentState, AssignmentStatusSnapshot, JobId,
    JobListFilter, JobStartRequest, JobState, JobStatus, PublicationState, RuntimeDiagnostics,
    RuntimeStatusSnapshot, WorkState, MAX_ASSIGNMENT_TEXT_BYTES,
};
use chrono::{DateTime, SecondsFormat, Utc};
use nostr::Event;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;
pub const STORE_COMMAND_CAPACITY: usize = 256;
pub const MAX_EVENT_JSON_BYTES: usize = 512 * 1024;
pub const MAX_ARGV_JSON_BYTES: usize = 64 * 1024;
pub const MAX_PENDING_PER_CHANNEL: usize = 500;
pub const MAX_REMOTE_CANCEL_TOMBSTONES: usize = 4096;
pub const MAX_INBOX_RETRIES: u32 = 10;
pub const BASE_RETRY_DELAY_SECS: u64 = 5;
pub const MAX_RETRY_DELAY_SECS: u64 = 300;
pub const REPLAY_SKEW_SECS: u64 = 5;
/// Current durable runtime-store schema.
pub const STORE_SCHEMA_VERSION: u32 = 4;
/// Store-owned operational diagnostics safe for owner-facing status.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreDiagnostics {
    pub schema_version: u32,
    pub last_relay_progress_published_at: Option<DateTime<Utc>>,
}
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("failed to open runtime store {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("runtime store error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("runtime store IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("runtime store serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("invalid runtime store data: {0}")]
    InvalidData(String),
    #[error("event_json exceeds 512 KiB")]
    EventTooLarge,
    #[error("argv_json exceeds 64 KiB")]
    ArgvTooLarge,
    #[error("invalid job transition from {from:?} to {to:?}")]
    InvalidJobTransition { from: JobState, to: JobState },
    #[error("a privileged job is already active")]
    ActiveJobExists,
    #[error("model job does not match the current active assignment")]
    AssignmentJobMismatch,
    #[error("remote cancel tombstone capacity reached")]
    CancelTombstoneCapacity,
    #[error("runtime store thread is unavailable")]
    Unavailable,
    #[error("assignment {0} was not found")]
    AssignmentNotFound(String),
    #[error("assignment {0} is not the current assignment")]
    AssignmentNotCurrent(String),
    #[error("assignment {assignment_id} is terminal in state {state:?}")]
    TerminalAssignment {
        assignment_id: String,
        state: AssignmentState,
    },
    #[error("invalid assignment transition from {from:?} to {to:?}")]
    InvalidAssignmentTransition {
        from: AssignmentState,
        to: AssignmentState,
    },
    #[error("assignment completion requires a succeeded linked job or delivery evidence")]
    AssignmentCompletionUnverified,
    #[error("invalid assignment: {0}")]
    InvalidAssignment(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Enqueued,
    Duplicate,
    CapacityRejected,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxState {
    Queued,
    InTurn,
    Completed,
    DeadLetter,
}
#[derive(Debug, Clone)]
pub struct InboxEvent {
    pub channel_id: Uuid,
    pub event: Event,
    pub received_at: DateTime<Utc>,
}
#[derive(Debug, Clone)]
pub struct InboxRecord {
    pub event_id: String,
    pub channel_id: Uuid,
    pub sender_pubkey: String,
    pub created_at: u64,
    pub received_at: DateTime<Utc>,
    pub event: Event,
    pub state: InboxState,
    pub attempt: u32,
    pub available_at: Option<DateTime<Utc>>,
    pub turn_id: Option<String>,
    pub last_error: Option<String>,
}
#[derive(Debug, Clone)]
pub struct InboxBatch {
    pub channel_id: Uuid,
    pub turn_id: String,
    pub events: Vec<InboxRecord>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequeueOutcome {
    Requeued {
        attempt: u32,
        available_at: DateTime<Utc>,
    },
    DeadLettered {
        attempt: u32,
    },
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecoveryOutcome {
    pub requeued: u64,
    pub dead_lettered: u64,
}

/// Durable startup components that must all reconcile before ready state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupRecoveryPhase {
    /// Interrupted durable inbox turns.
    Inbox,
    /// Persisted channel-to-ACP session mappings.
    Sessions,
    /// The current nonterminal assignment, if any.
    Assignments,
    /// Detached runner identities and receipts for nonterminal jobs.
    Runners,
}
impl StartupRecoveryPhase {
    fn key(self) -> &'static str {
        match self {
            Self::Inbox => "recovery_pending_inbox",
            Self::Sessions => "recovery_pending_sessions",
            Self::Assignments => "recovery_pending_assignments",
            Self::Runners => "recovery_pending_runners",
        }
    }
}

/// Nonterminal durable state discovered only after the recovery marker commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRecoverySnapshot {
    /// Number of inbox rows interrupted while a turn owned them.
    pub in_turn_inbox: u64,
    /// Current durable assignment that must be projected during recovery.
    pub active_assignment: Option<AssignmentRecord>,
    /// Nonterminal job identities requiring runner reconciliation.
    pub active_jobs: Vec<JobId>,
    /// Persisted channel session mappings requiring validation or later resume.
    pub channel_sessions: Vec<Uuid>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueDepths {
    pub queued: u64,
    pub in_turn: u64,
    pub completed: u64,
    pub dead_letter: u64,
    pub capacity_rejections: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeMode {
    Resume,
    Load,
    Fresh,
}
impl ResumeMode {
    fn text(self) -> &'static str {
        match self {
            Self::Resume => "resume",
            Self::Load => "load",
            Self::Fresh => "fresh",
        }
    }
    fn parse(v: &str) -> Result<Self, StoreError> {
        match v {
            "resume" => Ok(Self::Resume),
            "load" => Ok(Self::Load),
            "fresh" => Ok(Self::Fresh),
            _ => Err(StoreError::InvalidData(format!("invalid resume mode {v}"))),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub channel_id: Uuid,
    pub session_id: String,
    pub adapter_fingerprint: String,
    pub cwd: String,
    pub config_hash: String,
    pub resume_mode: ResumeMode,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignmentSnapshot {
    pub queue_depths: QueueDepths,
    pub active_assignment: Option<AssignmentRecord>,
    pub terminal_assignment: Option<AssignmentRecord>,
    pub active_jobs: Vec<JobId>,
    pub recovering: bool,
    pub recovery_reason: Option<String>,
}
impl AssignmentSnapshot {
    /// Builds the secret-free local status projection for one authenticated generation.
    pub fn runtime_status(
        &self,
        runtime_id: String,
        generation: Uuid,
        handshake_valid: bool,
        permission_gate_outstanding: bool,
        active_turn: bool,
        active_job: Option<&JobRecord>,
    ) -> RuntimeStatusSnapshot {
        let work_state = project_work_state(
            handshake_valid,
            self.recovering,
            permission_gate_outstanding,
            active_turn,
            self.active_assignment.as_ref(),
            active_job,
        );
        let active_job_id = self
            .active_assignment
            .as_ref()
            .and_then(|assignment| assignment.active_job_id)
            .or_else(|| self.active_jobs.first().copied());
        RuntimeStatusSnapshot {
            runtime_id,
            generation,
            work_state,
            recovering: self.recovering,
            recovery_reason: self.recovery_reason.clone(),
            queued_inbox: self.queue_depths.queued,
            in_turn_inbox: self.queue_depths.in_turn,
            dead_letter_inbox: self.queue_depths.dead_letter,
            capacity_rejections: self.queue_depths.capacity_rejections,
            active_assignment: self
                .active_assignment
                .as_ref()
                .map(AssignmentStatusSnapshot::from),
            active_job: active_job_id,
            active_jobs: self.active_jobs.clone(),
            diagnostics: RuntimeDiagnostics::default(),
        }
    }
}

/// Deterministically projects work state from durable runtime facts.
pub fn project_work_state(
    handshake_valid: bool,
    unreconciled: bool,
    permission_gate_outstanding: bool,
    active_turn: bool,
    assignment: Option<&AssignmentRecord>,
    active_job: Option<&JobRecord>,
) -> WorkState {
    if !handshake_valid {
        return WorkState::Offline;
    }
    if unreconciled || assignment.is_some_and(|value| value.state == AssignmentState::Recovering) {
        return WorkState::Recovering;
    }
    if permission_gate_outstanding
        || assignment.is_some_and(|value| value.state == AssignmentState::NeedsApproval)
    {
        return WorkState::NeedsApproval;
    }
    if assignment.is_some_and(|value| value.state == AssignmentState::Blocked) {
        return WorkState::Blocked;
    }
    if active_job.is_some_and(|value| {
        matches!(
            value.state,
            JobState::Accepted | JobState::Running | JobState::Cancelling
        )
    }) || assignment.is_some_and(|value| value.state == AssignmentState::Working)
    {
        return WorkState::Working;
    }
    if assignment.is_some_and(|value| value.state == AssignmentState::Waiting) {
        return WorkState::Waiting;
    }
    if active_turn || assignment.is_some_and(|value| value.state == AssignmentState::Reading) {
        return WorkState::Reading;
    }
    WorkState::Idle
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    pub job_id: JobId,
    pub request_event_id: String,
    pub requester_pubkey: String,
    pub executable: PathBuf,
    pub request: JobStartRequest,
    pub attempt: u32,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCancelTombstone {
    pub job_id: JobId,
    pub request_event_id: String,
    pub channel_id: Uuid,
    pub cancel_event_id: String,
    pub canceller_pubkey: String,
    pub authorized_without_request: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordCancelOutcome {
    Recorded,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelledRemoteJob {
    pub job: NewJob,
    pub cancel_event_id: String,
    pub result_json: String,
    pub terminal_event: OutboxEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerIdentity {
    pub pid: u32,
    pub start_marker: String,
    pub process_group: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRecord {
    pub job_id: JobId,
    pub request_event_id: Option<String>,
    pub source_event_id: Option<String>,
    pub channel_id: Uuid,
    pub requester_pubkey: String,
    pub driver: String,
    pub executable: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub summary: String,
    pub state: JobState,
    pub runner: Option<RunnerIdentity>,
    pub attempt: u32,
    pub progress_seq: u64,
    pub exit_code: Option<i32>,
    pub result_json: Option<String>,
    pub error_code: Option<String>,
    pub terminal_event_id: Option<String>,
    pub publication_state: PublicationState,
    pub publication_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}
impl JobRecord {
    /// Projects the stable same-host control response without executable or path details.
    pub fn to_status(&self) -> JobStatus {
        JobStatus {
            job_id: self.job_id,
            request_event_id: self.request_event_id.clone(),
            source_event_id: self.source_event_id.clone(),
            channel_id: self.channel_id,
            state: self.state,
            attempt: self.attempt,
            progress_seq: self.progress_seq,
            summary: self.summary.clone(),
            started_at: self.started_at.to_owned(),
            finished_at: self.finished_at.to_owned(),
            exit_code: self.exit_code,
            error_code: self.error_code.clone(),
            publication_state: self.publication_state,
            runner_pid: self.runner.as_ref().map(|v| v.pid),
            runner_start_marker: self.runner.as_ref().map(|v| v.start_marker.clone()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobTransition {
    pub job_id: JobId,
    pub attempt: u32,
    pub next_state: JobState,
    pub runner: Option<RunnerIdentity>,
    pub progress_seq: Option<u64>,
    pub exit_code: Option<i32>,
    pub result_json: Option<String>,
    pub error_code: Option<String>,
    pub terminal_event_id: Option<String>,
    pub publication_state: Option<PublicationState>,
    pub publication_error: Option<String>,
    pub occurred_at: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEvent {
    pub event_id: String,
    pub job_id: Option<JobId>,
    pub channel_id: Uuid,
    pub ordering_key: String,
    pub kind: u16,
    pub seq: Option<u64>,
    pub is_terminal: bool,
    pub event_json: String,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxRecord {
    pub id: i64,
    pub event: OutboxEvent,
    pub attempt: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateJobOutcome {
    Created(JobRecord),
    Duplicate(JobRecord),
}
#[derive(Clone)]
pub struct StoreHandle {
    tx: mpsc::Sender<Command>,
}
impl std::fmt::Debug for StoreHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreHandle").finish_non_exhaustive()
    }
}
impl StoreHandle {
    /// Opens and migrates the database before returning.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_owned();
        let (tx, rx) = mpsc::channel(STORE_COMMAND_CAPACITY);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let thread_path = path.clone();
        std::thread::Builder::new()
            .name("buzz-runtime-store".into())
            .spawn(move || match open_connection(&thread_path) {
                Ok(conn) => {
                    let _ = ready_tx.send(Ok(()));
                    run(conn, rx)
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|_| StoreError::Unavailable)?;
        ready_rx.recv().map_err(|_| StoreError::Unavailable)??;
        Ok(Self { tx })
    }
    async fn request<T>(
        &self,
        f: impl FnOnce(oneshot::Sender<Result<T, StoreError>>) -> Command,
    ) -> Result<T, StoreError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(f(tx))
            .await
            .map_err(|_| StoreError::Unavailable)?;
        rx.await.map_err(|_| StoreError::Unavailable)?
    }
    pub async fn enqueue_inbox(&self, v: InboxEvent) -> Result<EnqueueOutcome, StoreError> {
        self.request(|r| Command::Enqueue(v, r)).await
    }
    pub async fn claim_inbox_batch(
        &self,
        max: usize,
        turn_id: String,
        now: DateTime<Utc>,
    ) -> Result<Option<InboxBatch>, StoreError> {
        self.request(|r| Command::Claim(max, turn_id, now, r)).await
    }
    pub async fn complete_inbox(&self, turn_id: String) -> Result<usize, StoreError> {
        self.request(|r| Command::Complete(turn_id, r)).await
    }
    pub async fn requeue_inbox(
        &self,
        turn_id: String,
        error: String,
        now: DateTime<Utc>,
    ) -> Result<RequeueOutcome, StoreError> {
        self.request(|r| Command::Requeue(turn_id, error, now, r))
            .await
    }
    pub async fn dead_letter_inbox(
        &self,
        turn_id: String,
        error: String,
    ) -> Result<usize, StoreError> {
        self.request(|r| Command::Dead(turn_id, error, r)).await
    }
    pub async fn recover_in_turn(&self, now: DateTime<Utc>) -> Result<RecoveryOutcome, StoreError> {
        self.request(|r| Command::Recover(now, r)).await
    }
    pub async fn channel_watermark(&self, id: Uuid) -> Result<Option<u64>, StoreError> {
        self.request(|r| Command::Watermark(Some(id), r)).await
    }
    pub async fn replay_watermark(&self) -> Result<Option<u64>, StoreError> {
        self.request(|r| Command::Watermark(None, r)).await
    }
    pub async fn queue_depths(&self) -> Result<QueueDepths, StoreError> {
        self.request(Command::Depths).await
    }
    pub async fn get_channel_session(&self, id: Uuid) -> Result<Option<SessionRecord>, StoreError> {
        self.request(|r| Command::GetSession(id, r)).await
    }
    /// Lists every persisted channel session for startup validation.
    pub async fn channel_sessions(&self) -> Result<Vec<SessionRecord>, StoreError> {
        self.request(Command::ListSessions).await
    }
    pub async fn upsert_channel_session(&self, v: SessionRecord) -> Result<(), StoreError> {
        self.request(|r| Command::UpsertSession(v, r)).await
    }
    pub async fn delete_channel_session(&self, id: Uuid) -> Result<bool, StoreError> {
        self.request(|r| Command::DeleteSession(id, r)).await
    }
    pub async fn release_inbox(&self, turn_id: String) -> Result<usize, StoreError> {
        self.request(|r| Command::Release(turn_id, r)).await
    }
    pub async fn complete_inbox_event(&self, event_id: String) -> Result<bool, StoreError> {
        self.request(|r| Command::CompleteEvent(event_id, r)).await
    }
    pub async fn dead_letter_channel(
        &self,
        channel_id: Uuid,
        error: String,
    ) -> Result<Vec<String>, StoreError> {
        self.request(|r| Command::DeadChannel(channel_id, error, r))
            .await
    }
    pub async fn create_local_job(
        &self,
        job: NewJob,
        request_event: OutboxEvent,
    ) -> Result<CreateJobOutcome, StoreError> {
        self.request(|r| Command::CreateJob(job, request_event, r))
            .await
    }
    /// Atomically admits a model-origin job and links it to the exact current assignment.
    pub async fn create_local_job_for_assignment(
        &self,
        assignment_id: &str,
        job: NewJob,
        request_event: OutboxEvent,
    ) -> Result<CreateJobOutcome, StoreError> {
        self.request(|reply| {
            Command::CreateAssignmentJob(assignment_id.to_owned(), job, request_event, reply)
        })
        .await
    }
    pub async fn create_remote_job(&self, job: NewJob) -> Result<CreateJobOutcome, StoreError> {
        self.request(|r| Command::CreateRemoteJob(job, r)).await
    }
    pub async fn record_remote_cancel(
        &self,
        tombstone: RemoteCancelTombstone,
    ) -> Result<RecordCancelOutcome, StoreError> {
        self.request(|r| Command::RecordRemoteCancel(tombstone, r))
            .await
    }
    pub async fn remote_cancels(
        &self,
        job_id: JobId,
        request_event_id: String,
    ) -> Result<Vec<RemoteCancelTombstone>, StoreError> {
        self.request(|r| Command::RemoteCancels(job_id, request_event_id, r))
            .await
    }
    pub async fn discard_remote_cancels(
        &self,
        job_id: JobId,
        request_event_id: String,
    ) -> Result<usize, StoreError> {
        self.request(|r| Command::DiscardRemoteCancels(job_id, request_event_id, r))
            .await
    }
    pub async fn create_cancelled_remote_job(
        &self,
        cancelled: CancelledRemoteJob,
    ) -> Result<CreateJobOutcome, StoreError> {
        self.request(|r| Command::CreateCancelledRemoteJob(cancelled, r))
            .await
    }
    pub async fn transition_job(
        &self,
        transition: JobTransition,
        outbox: Option<OutboxEvent>,
    ) -> Result<JobRecord, StoreError> {
        self.request(|r| Command::TransitionJob(transition, outbox, r))
            .await
    }
    pub async fn get_job(&self, id: JobId) -> Result<Option<JobRecord>, StoreError> {
        self.request(|r| Command::GetJob(id, r)).await
    }
    pub async fn list_jobs(&self, filter: JobListFilter) -> Result<Vec<JobRecord>, StoreError> {
        self.request(|r| Command::ListJobs(filter, r)).await
    }
    pub async fn pending_outbox(
        &self,
        limit: usize,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutboxRecord>, StoreError> {
        self.request(|r| Command::PendingOutbox(limit, now, r))
            .await
    }
    /// Marks one pending outbox row published.
    pub async fn mark_outbox_published(
        &self,
        id: i64,
        at: DateTime<Utc>,
    ) -> Result<bool, StoreError> {
        self.request(|reply| Command::MarkOutboxPublished { id, at, reply })
            .await
    }
    /// Schedules one pending outbox row for a later retry.
    pub async fn mark_outbox_retry(
        &self,
        id: i64,
        error: String,
        available_at: DateTime<Utc>,
    ) -> Result<bool, StoreError> {
        self.request(|reply| Command::MarkOutboxRetry {
            id,
            error,
            available_at,
            reply,
        })
        .await
    }
    /// Permanently rejects one pending outbox row without rolling back job state.
    pub async fn mark_outbox_rejected(
        &self,
        id: i64,
        reason: String,
        at: DateTime<Utc>,
    ) -> Result<bool, StoreError> {
        self.request(|reply| Command::MarkOutboxRejected {
            id,
            reason,
            at,
            reply,
        })
        .await
    }
    /// Inserts a caller-defined assignment after validating its stable identity.
    pub async fn create_assignment(
        &self,
        assignment: AssignmentRecord,
    ) -> Result<AssignmentRecord, StoreError> {
        self.request(|reply| Command::CreateAssignment(assignment, reply))
            .await
    }
    /// Claims a new source as reading, or returns the existing active assignment unchanged.
    pub async fn claim_assignment(
        &self,
        channel_id: Uuid,
        source_event_id: Option<String>,
        summary: String,
        session_id: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<AssignmentRecord, StoreError> {
        self.request(|reply| Command::ClaimAssignment {
            channel_id,
            source_event_id,
            summary,
            session_id,
            now,
            reply,
        })
        .await
    }
    pub async fn active_assignment(&self) -> Result<Option<AssignmentRecord>, StoreError> {
        self.request(Command::ActiveAssignment).await
    }
    /// Changes only the exact current nonterminal assignment.
    pub async fn set_assignment_state(
        &self,
        assignment_id: &str,
        request: AssignmentSetStateRequest,
        now: DateTime<Utc>,
    ) -> Result<AssignmentRecord, StoreError> {
        let assignment_id = assignment_id.to_owned();
        self.request(|reply| Command::SetAssignmentState {
            assignment_id,
            request,
            now,
            reply,
        })
        .await
    }
    /// Links one durable job to the exact current assignment.
    pub async fn link_assignment_job(
        &self,
        assignment_id: &str,
        job_id: JobId,
        now: DateTime<Utc>,
    ) -> Result<AssignmentRecord, StoreError> {
        let assignment_id = assignment_id.to_owned();
        self.request(|reply| Command::LinkAssignmentJob {
            assignment_id,
            job_id,
            now,
            reply,
        })
        .await
    }
    /// Completes an assignment only after the store can verify delivery.
    pub async fn complete_assignment(
        &self,
        assignment_id: &str,
        evidence: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<AssignmentRecord, StoreError> {
        self.set_assignment_state(
            assignment_id,
            AssignmentSetStateRequest {
                state: AssignmentState::Completed,
                summary: None,
                reason: None,
                blocker: None,
                approval_gate_id: None,
                delivery_evidence: evidence,
                reply_event_id: None,
            },
            now,
        )
        .await
    }
    /// Removes the latest terminal assignment from the hot snapshot without deleting history.
    pub async fn clear_terminal_assignment(&self) -> Result<bool, StoreError> {
        self.request(Command::ClearTerminalAssignment).await
    }
    /// Returns one transactional durable-state snapshot.
    pub async fn assignment_snapshot(&self) -> Result<AssignmentSnapshot, StoreError> {
        self.request(Command::AssignmentSnapshot).await
    }
    /// Returns schema and relay-progress publication diagnostics without secrets or paths.
    pub async fn operational_diagnostics(&self) -> Result<StoreDiagnostics, StoreError> {
        self.request(Command::OperationalDiagnostics).await
    }
    /// Records whether startup reconciliation is outstanding.
    pub async fn set_recovery_state(
        &self,
        recovering: bool,
        reason: Option<String>,
    ) -> Result<(), StoreError> {
        self.request(|reply| Command::SetRecoveryState {
            recovering,
            reason,
            reply,
        })
        .await
    }
    /// Durably marks every startup recovery component pending before returning
    /// any nonterminal state to the caller.
    pub async fn begin_startup_recovery(
        &self,
        reason: &str,
    ) -> Result<StartupRecoverySnapshot, StoreError> {
        self.request(|reply| Command::BeginStartupRecovery {
            reason: reason.to_owned(),
            reply,
        })
        .await
    }
    /// Completes one startup component. Recovery clears only when all four
    /// components have been completed durably.
    pub async fn complete_startup_recovery_phase(
        &self,
        phase: StartupRecoveryPhase,
    ) -> Result<bool, StoreError> {
        self.request(|reply| Command::CompleteStartupRecoveryPhase { phase, reply })
            .await
    }
}
type Reply<T> = oneshot::Sender<Result<T, StoreError>>;
enum Command {
    Enqueue(InboxEvent, Reply<EnqueueOutcome>),
    Claim(usize, String, DateTime<Utc>, Reply<Option<InboxBatch>>),
    Complete(String, Reply<usize>),
    Requeue(String, String, DateTime<Utc>, Reply<RequeueOutcome>),
    Dead(String, String, Reply<usize>),
    Recover(DateTime<Utc>, Reply<RecoveryOutcome>),
    Watermark(Option<Uuid>, Reply<Option<u64>>),
    Depths(Reply<QueueDepths>),
    GetSession(Uuid, Reply<Option<SessionRecord>>),
    ListSessions(Reply<Vec<SessionRecord>>),
    UpsertSession(SessionRecord, Reply<()>),
    DeleteSession(Uuid, Reply<bool>),
    Release(String, Reply<usize>),
    CompleteEvent(String, Reply<bool>),
    DeadChannel(Uuid, String, Reply<Vec<String>>),
    CreateJob(NewJob, OutboxEvent, Reply<CreateJobOutcome>),
    CreateAssignmentJob(String, NewJob, OutboxEvent, Reply<CreateJobOutcome>),
    CreateRemoteJob(NewJob, Reply<CreateJobOutcome>),
    RecordRemoteCancel(RemoteCancelTombstone, Reply<RecordCancelOutcome>),
    RemoteCancels(JobId, String, Reply<Vec<RemoteCancelTombstone>>),
    DiscardRemoteCancels(JobId, String, Reply<usize>),
    CreateCancelledRemoteJob(CancelledRemoteJob, Reply<CreateJobOutcome>),
    TransitionJob(JobTransition, Option<OutboxEvent>, Reply<JobRecord>),
    GetJob(JobId, Reply<Option<JobRecord>>),
    ListJobs(JobListFilter, Reply<Vec<JobRecord>>),
    PendingOutbox(usize, DateTime<Utc>, Reply<Vec<OutboxRecord>>),
    MarkOutboxPublished {
        id: i64,
        at: DateTime<Utc>,
        reply: Reply<bool>,
    },
    MarkOutboxRetry {
        id: i64,
        error: String,
        available_at: DateTime<Utc>,
        reply: Reply<bool>,
    },
    MarkOutboxRejected {
        id: i64,
        reason: String,
        at: DateTime<Utc>,
        reply: Reply<bool>,
    },
    CreateAssignment(AssignmentRecord, Reply<AssignmentRecord>),
    ClaimAssignment {
        channel_id: Uuid,
        source_event_id: Option<String>,
        summary: String,
        session_id: Option<String>,
        now: DateTime<Utc>,
        reply: Reply<AssignmentRecord>,
    },
    ActiveAssignment(Reply<Option<AssignmentRecord>>),
    SetAssignmentState {
        assignment_id: String,
        request: AssignmentSetStateRequest,
        now: DateTime<Utc>,
        reply: Reply<AssignmentRecord>,
    },
    LinkAssignmentJob {
        assignment_id: String,
        job_id: JobId,
        now: DateTime<Utc>,
        reply: Reply<AssignmentRecord>,
    },
    ClearTerminalAssignment(Reply<bool>),
    AssignmentSnapshot(Reply<AssignmentSnapshot>),
    OperationalDiagnostics(Reply<StoreDiagnostics>),
    SetRecoveryState {
        recovering: bool,
        reason: Option<String>,
        reply: Reply<()>,
    },
    BeginStartupRecovery {
        reason: String,
        reply: Reply<StartupRecoverySnapshot>,
    },
    CompleteStartupRecoveryPhase {
        phase: StartupRecoveryPhase,
        reply: Reply<bool>,
    },
}
fn run(mut c: Connection, mut rx: mpsc::Receiver<Command>) {
    while let Some(v) = rx.blocking_recv() {
        match v {
            Command::Enqueue(v, r) => {
                let _ = r.send(enqueue(&mut c, v));
            }
            Command::Claim(m, t, n, r) => {
                let _ = r.send(claim(&mut c, m, &t, n));
            }
            Command::Complete(t, r) => {
                let _ = r.send(complete(&c, &t));
            }
            Command::Requeue(t, e, n, r) => {
                let _ = r.send(requeue(&mut c, &t, &e, n));
            }
            Command::Dead(t, e, r) => {
                let _ = r.send(dead(&c, &t, &e));
            }
            Command::Recover(n, r) => {
                let _ = r.send(recover(&mut c, n));
            }
            Command::Watermark(id, r) => {
                let _ = r.send(watermark(&c, id));
            }
            Command::Depths(r) => {
                let _ = r.send(depths(&c));
            }
            Command::GetSession(id, r) => {
                let _ = r.send(get_session(&c, id));
            }
            Command::ListSessions(r) => {
                let _ = r.send(channel_sessions(&c));
            }
            Command::UpsertSession(v, r) => {
                let _ = r.send(upsert_session(&c, &v));
            }
            Command::DeleteSession(id, r) => {
                let _ = r.send(delete_session(&c, id));
            }
            Command::Release(t, r) => {
                let _ = r.send(release(&c, &t));
            }
            Command::CompleteEvent(id, r) => {
                let _ = r.send(complete_event(&c, &id));
            }
            Command::DeadChannel(id, e, r) => {
                let _ = r.send(dead_channel(&mut c, id, &e));
            }
            Command::CreateJob(j, o, r) => {
                let _ = r.send(create_job(&mut c, j, o));
            }
            Command::CreateAssignmentJob(assignment_id, job, outbox, reply) => {
                let _ = reply.send(create_assignment_job(&mut c, &assignment_id, job, outbox));
            }
            Command::CreateRemoteJob(j, r) => {
                let _ = r.send(create_remote_job(&mut c, j));
            }
            Command::RecordRemoteCancel(tombstone, reply) => {
                let _ = reply.send(record_remote_cancel(&mut c, tombstone));
            }
            Command::RemoteCancels(job_id, request_event_id, reply) => {
                let _ = reply.send(remote_cancels(&c, job_id, &request_event_id));
            }
            Command::DiscardRemoteCancels(job_id, request_event_id, reply) => {
                let _ = reply.send(discard_remote_cancels(&c, job_id, &request_event_id));
            }
            Command::CreateCancelledRemoteJob(cancelled, reply) => {
                let _ = reply.send(create_cancelled_remote_job(&mut c, cancelled));
            }
            Command::TransitionJob(t, o, r) => {
                let _ = r.send(transition_job(&mut c, t, o));
            }
            Command::GetJob(id, r) => {
                let _ = r.send(get_job(&c, id));
            }
            Command::ListJobs(f, r) => {
                let _ = r.send(list_jobs(&c, f));
            }
            Command::PendingOutbox(l, n, r) => {
                let _ = r.send(pending_outbox(&c, l, n));
            }
            Command::MarkOutboxPublished { id, at, reply } => {
                let _ = reply.send(mark_outbox_published(&mut c, id, at));
            }
            Command::MarkOutboxRetry {
                id,
                error,
                available_at,
                reply,
            } => {
                let _ = reply.send(mark_outbox_retry(&c, id, &error, available_at));
            }
            Command::MarkOutboxRejected {
                id,
                reason,
                at,
                reply,
            } => {
                let _ = reply.send(mark_outbox_rejected(&mut c, id, &reason, at));
            }
            Command::CreateAssignment(a, r) => {
                let _ = r.send(create_assignment(&mut c, a));
            }
            Command::ClaimAssignment {
                channel_id,
                source_event_id,
                summary,
                session_id,
                now,
                reply,
            } => {
                let _ = reply.send(claim_assignment(
                    &mut c,
                    channel_id,
                    source_event_id,
                    summary,
                    session_id,
                    now,
                ));
            }
            Command::ActiveAssignment(r) => {
                let _ = r.send(active_assignment(&c));
            }
            Command::SetAssignmentState {
                assignment_id,
                request,
                now,
                reply,
            } => {
                let _ = reply.send(set_assignment_state(&mut c, &assignment_id, request, now));
            }
            Command::LinkAssignmentJob {
                assignment_id,
                job_id,
                now,
                reply,
            } => {
                let _ = reply.send(link_assignment_job(&mut c, &assignment_id, job_id, now));
            }
            Command::ClearTerminalAssignment(r) => {
                let _ = r.send(clear_terminal_assignment(&c));
            }
            Command::AssignmentSnapshot(r) => {
                let _ = r.send(assignment_snapshot(&mut c));
            }
            Command::OperationalDiagnostics(r) => {
                let _ = r.send(operational_diagnostics(&c));
            }
            Command::SetRecoveryState {
                recovering,
                reason,
                reply,
            } => {
                let _ = reply.send(set_recovery_state(&mut c, recovering, reason));
            }
            Command::BeginStartupRecovery { reason, reply } => {
                let _ = reply.send(begin_startup_recovery(&mut c, &reason));
            }
            Command::CompleteStartupRecoveryPhase { phase, reply } => {
                let _ = reply.send(complete_startup_recovery_phase(&mut c, phase));
            }
        }
    }
}
fn open_connection(path: &Path) -> Result<Connection, StoreError> {
    prepare_store_path(path)?;
    let mut c = Connection::open(path).map_err(|source| StoreError::Open {
        path: path.to_owned(),
        source,
    })?;
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.pragma_update(None, "foreign_keys", "ON")?;
    c.busy_timeout(Duration::from_millis(5000))?;
    let tx = c.transaction_with_behavior(TransactionBehavior::Exclusive)?;
    tx.execute_batch(SCHEMA)?;
    ensure_assignment_column(&tx, "reason", "TEXT")?;
    ensure_assignment_column(&tx, "approval_gate_id", "TEXT")?;
    ensure_assignment_column(&tx, "delivery_evidence", "TEXT")?;
    tx.execute("INSERT INTO runtime_meta(key,value)VALUES('schema_version',?1)ON CONFLICT(key)DO UPDATE SET value=excluded.value",[STORE_SCHEMA_VERSION.to_string()])?;
    tx.commit()?;
    Ok(c)
}
fn prepare_store_path(path: &Path) -> Result<(), StoreError> {
    if path == Path::new(":memory:") {
        return Ok(());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| StoreError::InvalidData("store path has no parent".into()))?;
    crate::artifacts::ensure_owner_dir(parent)
        .map_err(|e| StoreError::InvalidData(e.to_string()))?;
    if path.exists() {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(StoreError::InvalidData(
                "store path is not a regular file".into(),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(StoreError::InvalidData(
                    "store file is not owner-only".into(),
                ));
            }
        }
    } else {
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        #[cfg(windows)]
        crate::artifacts::harden_windows_acl(path, false)
            .map_err(|e| StoreError::InvalidData(e.to_string()))?;
        file.sync_all()?;
    }
    #[cfg(windows)]
    crate::artifacts::harden_windows_acl(path, false)
        .map_err(|e| StoreError::InvalidData(e.to_string()))?;
    Ok(())
}
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS runtime_meta(
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS inbox_events(
    event_id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL,
    sender_pubkey TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    received_at TEXT NOT NULL,
    event_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN('queued','in_turn','completed','dead_letter')),
    attempt INTEGER NOT NULL DEFAULT 0,
    available_at TEXT,
    turn_id TEXT,
    last_error TEXT
);
CREATE INDEX IF NOT EXISTS inbox_dispatch_idx
    ON inbox_events(state,available_at,created_at,event_id);
CREATE TABLE IF NOT EXISTS channel_sessions(
    channel_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    adapter_fingerprint TEXT NOT NULL,
    cwd TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    resume_mode TEXT NOT NULL CHECK(resume_mode IN('resume','load','fresh')),
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS assignments(
    assignment_id TEXT PRIMARY KEY,
    source_event_id TEXT,
    channel_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN(
        'reading','working','waiting','needs_approval','blocked','recovering',
        'completed','failed','cancelled'
    )),
    summary TEXT NOT NULL,
    active_job_id TEXT,
    session_id TEXT,
    reply_event_id TEXT,
    last_progress_at TEXT NOT NULL,
    reason TEXT,
    blocker TEXT,
    approval_gate_id TEXT,
    delivery_evidence TEXT,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS assignments_single_active
    ON assignments((1))
    WHERE state IN('reading','working','waiting','needs_approval','blocked','recovering');
CREATE TABLE IF NOT EXISTS jobs(
    job_id TEXT PRIMARY KEY,
    request_event_id TEXT,
    source_event_id TEXT,
    channel_id TEXT NOT NULL,
    requester_pubkey TEXT NOT NULL,
    driver TEXT NOT NULL CHECK(driver='lh'),
    executable TEXT NOT NULL,
    argv_json TEXT NOT NULL,
    cwd TEXT NOT NULL,
    summary TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN(
        'requested','accepted','running','cancelling','succeeded','failed','cancelled','lost'
    )),
    runner_pid INTEGER,
    runner_start_marker TEXT,
    process_group TEXT,
    attempt INTEGER NOT NULL DEFAULT 1,
    progress_seq INTEGER NOT NULL DEFAULT 0,
    exit_code INTEGER,
    result_json TEXT,
    error_code TEXT,
    terminal_event_id TEXT,
    publication_state TEXT NOT NULL DEFAULT 'not_started' CHECK(
        publication_state IN('not_started','pending','published','failed')
    ),
    publication_error TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS jobs_single_active
    ON jobs((1))
    WHERE state IN('requested','accepted','running','cancelling');
CREATE TABLE IF NOT EXISTS job_cancel_tombstones(
    cancel_event_id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    request_event_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    canceller_pubkey TEXT NOT NULL,
    authorized_without_request INTEGER NOT NULL CHECK(authorized_without_request IN(0,1)),
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS job_cancel_tombstones_identity
    ON job_cancel_tombstones(job_id,request_event_id);
CREATE TABLE IF NOT EXISTS relay_outbox(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    job_id TEXT,
    channel_id TEXT NOT NULL,
    ordering_key TEXT NOT NULL,
    kind INTEGER NOT NULL,
    seq INTEGER,
    is_terminal INTEGER NOT NULL CHECK(is_terminal IN(0,1)),
    event_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN('pending','published','rejected','superseded')),
    attempt INTEGER NOT NULL DEFAULT 0,
    available_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL,
    published_at TEXT,
    rejected_at TEXT
);
CREATE INDEX IF NOT EXISTS relay_outbox_publish_idx
    ON relay_outbox(state,ordering_key,available_at,id);
";

fn ensure_assignment_column(
    transaction: &rusqlite::Transaction<'_>,
    name: &str,
    definition: &str,
) -> Result<(), StoreError> {
    let mut statement = transaction.prepare("PRAGMA table_info(assignments)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    if !columns.iter().any(|column| column == name) {
        transaction.execute(
            &format!("ALTER TABLE assignments ADD COLUMN {name} {definition}"),
            [],
        )?;
    }
    Ok(())
}
fn stamp(v: DateTime<Utc>) -> String {
    v.to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn parse_stamp(v: &str) -> Result<DateTime<Utc>, StoreError> {
    DateTime::parse_from_rfc3339(v)
        .map(|v| v.with_timezone(&Utc))
        .map_err(|e| StoreError::InvalidData(e.to_string()))
}
fn delay(a: u32) -> Duration {
    let b = BASE_RETRY_DELAY_SECS
        .saturating_mul(1u64 << a.saturating_sub(1).min(6))
        .min(MAX_RETRY_DELAY_SECS);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    Duration::from_secs_f64(b as f64 * (0.8 + (n as f64 / u32::MAX as f64) * 0.4))
}
fn enqueue(c: &mut Connection, v: InboxEvent) -> Result<EnqueueOutcome, StoreError> {
    let json = serde_json::to_string(&v.event)?;
    if json.len() > MAX_EVENT_JSON_BYTES {
        return Err(StoreError::EventTooLarge);
    }
    let id = v.event.id.to_hex();
    let tx = c.transaction()?;
    if tx
        .query_row(
            "SELECT 1 FROM inbox_events WHERE event_id=?1",
            [&id],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    {
        tx.commit()?;
        return Ok(EnqueueOutcome::Duplicate);
    }
    let n: i64 = tx.query_row(
        "SELECT COUNT(*)FROM inbox_events WHERE channel_id=?1 AND state IN('queued','in_turn')",
        [v.channel_id.to_string()],
        |r| r.get(0),
    )?;
    let (state, error, outcome) = if n >= 500 {
        (
            "dead_letter",
            Some("queue_capacity"),
            EnqueueOutcome::CapacityRejected,
        )
    } else {
        ("queued", None, EnqueueOutcome::Enqueued)
    };
    tx.execute("INSERT INTO inbox_events(event_id,channel_id,sender_pubkey,created_at,received_at,event_json,state,last_error)VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id,v.channel_id.to_string(),v.event.pubkey.to_hex(),v.event.created_at.as_secs() as i64,stamp(v.received_at),json,state,error])?;
    if outcome == EnqueueOutcome::CapacityRejected {
        tx.execute("INSERT INTO runtime_meta(key,value)VALUES('capacity_rejections','1')ON CONFLICT(key)DO UPDATE SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT)",[])?;
    }
    tx.commit()?;
    Ok(outcome)
}
fn claim(
    c: &mut Connection,
    max: usize,
    turn: &str,
    now: DateTime<Utc>,
) -> Result<Option<InboxBatch>, StoreError> {
    if max == 0 {
        return Ok(None);
    }
    let tx = c.transaction()?;
    let now = stamp(now);
    let ch:Option<String>=tx.query_row("SELECT q.channel_id FROM inbox_events q WHERE q.state='queued' AND(q.available_at IS NULL OR q.available_at<=?1)AND NOT EXISTS(SELECT 1 FROM inbox_events i WHERE i.channel_id=q.channel_id AND i.state='in_turn')AND NOT EXISTS(SELECT 1 FROM inbox_events older WHERE older.channel_id=q.channel_id AND older.state='queued' AND(older.created_at<q.created_at OR(older.created_at=q.created_at AND older.event_id<q.event_id)))ORDER BY q.created_at,q.event_id LIMIT 1",[&now],|r|r.get(0)).optional()?;
    let Some(ch) = ch else {
        tx.commit()?;
        return Ok(None);
    };
    let mut q=tx.prepare("SELECT event_id,channel_id,sender_pubkey,created_at,received_at,event_json,state,attempt,available_at,turn_id,last_error FROM inbox_events WHERE channel_id=?1 AND state='queued' AND(available_at IS NULL OR available_at<=?2)ORDER BY created_at,event_id LIMIT ?3")?;
    let rows = q
        .query_map(params![ch, now, max as i64], read_record)?
        .collect::<Result<Vec<_>, _>>()?;
    drop(q);
    for v in &rows {
        tx.execute("UPDATE inbox_events SET state='in_turn',turn_id=?1,available_at=NULL WHERE event_id=?2",params![turn,v.event_id])?;
    }
    tx.commit()?;
    if rows.is_empty() {
        Ok(None)
    } else {
        Ok(Some(InboxBatch {
            channel_id: rows[0].channel_id,
            turn_id: turn.into(),
            events: rows,
        }))
    }
}
fn invalid(e: impl ToString) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            e.to_string(),
        )),
    )
}
fn read_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<InboxRecord> {
    let ch: String = r.get(1)?;
    let at: String = r.get(4)?;
    let json: String = r.get(5)?;
    let state: String = r.get(6)?;
    let available: Option<String> = r.get(8)?;
    Ok(InboxRecord {
        event_id: r.get(0)?,
        channel_id: Uuid::parse_str(&ch).map_err(invalid)?,
        sender_pubkey: r.get(2)?,
        created_at: r.get::<_, i64>(3)? as u64,
        received_at: parse_stamp(&at).map_err(invalid)?,
        event: serde_json::from_str(&json).map_err(invalid)?,
        state: match state.as_str() {
            "queued" => InboxState::Queued,
            "in_turn" => InboxState::InTurn,
            "completed" => InboxState::Completed,
            "dead_letter" => InboxState::DeadLetter,
            _ => return Err(invalid(state)),
        },
        attempt: r.get::<_, i64>(7)? as u32,
        available_at: available
            .as_deref()
            .map(parse_stamp)
            .transpose()
            .map_err(invalid)?,
        turn_id: r.get(9)?,
        last_error: r.get(10)?,
    })
}
fn complete(c: &Connection, t: &str) -> Result<usize, StoreError> {
    Ok(c.execute("UPDATE inbox_events SET state='completed',turn_id=NULL,available_at=NULL,last_error=NULL WHERE turn_id=?1 AND state='in_turn'",[t])?)
}
fn dead(c: &Connection, t: &str, e: &str) -> Result<usize, StoreError> {
    Ok(c.execute("UPDATE inbox_events SET state='dead_letter',turn_id=NULL,available_at=NULL,last_error=?1 WHERE turn_id=?2 AND state='in_turn'",params![e,t])?)
}
fn requeue(
    c: &mut Connection,
    t: &str,
    e: &str,
    now: DateTime<Utc>,
) -> Result<RequeueOutcome, StoreError> {
    let tx = c.transaction()?;
    let a = tx.query_row(
        "SELECT COALESCE(MAX(attempt),0)+1 FROM inbox_events WHERE turn_id=?1 AND state='in_turn'",
        [t],
        |r| r.get::<_, i64>(0),
    )? as u32;
    if a > 10 {
        tx.execute("UPDATE inbox_events SET state='dead_letter',attempt=?1,turn_id=NULL,available_at=NULL,last_error=?2 WHERE turn_id=?3 AND state='in_turn'",params![a,e,t])?;
        tx.commit()?;
        return Ok(RequeueOutcome::DeadLettered { attempt: a });
    }
    let at = now + chrono::Duration::from_std(delay(a)).unwrap_or_default();
    tx.execute("UPDATE inbox_events SET state='queued',attempt=?1,turn_id=NULL,available_at=?2,last_error=?3 WHERE turn_id=?4 AND state='in_turn'",params![a,stamp(at),e,t])?;
    tx.commit()?;
    Ok(RequeueOutcome::Requeued {
        attempt: a,
        available_at: at,
    })
}
fn recover(c: &mut Connection, now: DateTime<Utc>) -> Result<RecoveryOutcome, StoreError> {
    let tx = c.transaction()?;
    let mut q = tx.prepare("SELECT event_id,attempt FROM inbox_events WHERE state='in_turn'")?;
    let rows = q
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(q);
    let mut out = RecoveryOutcome::default();
    for (id, old) in rows {
        let a = old + 1;
        if a > 10 {
            tx.execute("UPDATE inbox_events SET state='dead_letter',attempt=?1,turn_id=NULL,available_at=NULL,last_error='recovery_retry_exhausted' WHERE event_id=?2",params![a,id])?;
            out.dead_lettered += 1
        } else {
            let at = now + chrono::Duration::from_std(delay(a)).unwrap_or_default();
            tx.execute("UPDATE inbox_events SET state='queued',attempt=?1,turn_id=NULL,available_at=?2,last_error='runtime_recovery' WHERE event_id=?3",params![a,stamp(at),id])?;
            out.requeued += 1
        }
    }
    tx.commit()?;
    Ok(out)
}
fn watermark(c: &Connection, id: Option<Uuid>) -> Result<Option<u64>, StoreError> {
    let v = if let Some(id) = id {
        c.query_row(
            "SELECT MAX(created_at)FROM inbox_events WHERE channel_id=?1",
            [id.to_string()],
            |r| r.get::<_, Option<i64>>(0),
        )?
    } else {
        c.query_row("SELECT MIN(channel_max)FROM(SELECT MAX(created_at)AS channel_max FROM inbox_events GROUP BY channel_id)",[],|r|r.get::<_,Option<i64>>(0))?
    };
    Ok(v.map(|v| v as u64))
}
fn depths(c: &Connection) -> Result<QueueDepths, StoreError> {
    let n = |s: &str| -> Result<u64, StoreError> {
        Ok(c.query_row(
            "SELECT COUNT(*)FROM inbox_events WHERE state=?1",
            [s],
            |r| r.get::<_, i64>(0),
        )? as u64)
    };
    let cap = c
        .query_row(
            "SELECT value FROM runtime_meta WHERE key='capacity_rejections'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    Ok(QueueDepths {
        queued: n("queued")?,
        in_turn: n("in_turn")?,
        completed: n("completed")?,
        dead_letter: n("dead_letter")?,
        capacity_rejections: cap,
    })
}
fn get_session(c: &Connection, id: Uuid) -> Result<Option<SessionRecord>, StoreError> {
    c.query_row("SELECT session_id,adapter_fingerprint,cwd,config_hash,resume_mode,updated_at FROM channel_sessions WHERE channel_id=?1",[id.to_string()],|r|{let mode:String=r.get(4)?;let at:String=r.get(5)?;Ok(SessionRecord{channel_id:id,session_id:r.get(0)?,adapter_fingerprint:r.get(1)?,cwd:r.get(2)?,config_hash:r.get(3)?,resume_mode:ResumeMode::parse(&mode).map_err(invalid)?,updated_at:parse_stamp(&at).map_err(invalid)?})}).optional().map_err(StoreError::from)
}
fn channel_session_ids(c: &Connection) -> Result<Vec<Uuid>, StoreError> {
    let mut statement = c.prepare("SELECT channel_id FROM channel_sessions ORDER BY channel_id")?;
    let sessions = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .map(|row| {
            row.map_err(StoreError::from).and_then(|channel| {
                Uuid::parse_str(&channel)
                    .map_err(|error| StoreError::InvalidData(error.to_string()))
            })
        })
        .collect();
    sessions
}
fn channel_sessions(c: &Connection) -> Result<Vec<SessionRecord>, StoreError> {
    channel_session_ids(c)?
        .into_iter()
        .map(|channel| {
            get_session(c, channel)?
                .ok_or_else(|| StoreError::InvalidData("listed channel session disappeared".into()))
        })
        .collect()
}
fn upsert_session(c: &Connection, v: &SessionRecord) -> Result<(), StoreError> {
    c.execute("INSERT INTO channel_sessions(channel_id,session_id,adapter_fingerprint,cwd,config_hash,resume_mode,updated_at)VALUES(?1,?2,?3,?4,?5,?6,?7)ON CONFLICT(channel_id)DO UPDATE SET session_id=excluded.session_id,adapter_fingerprint=excluded.adapter_fingerprint,cwd=excluded.cwd,config_hash=excluded.config_hash,resume_mode=excluded.resume_mode,updated_at=excluded.updated_at",params![v.channel_id.to_string(),v.session_id,v.adapter_fingerprint,v.cwd,v.config_hash,v.resume_mode.text(),stamp(v.updated_at)])?;
    Ok(())
}
fn delete_session(c: &Connection, id: Uuid) -> Result<bool, StoreError> {
    Ok(c.execute(
        "DELETE FROM channel_sessions WHERE channel_id=?1",
        [id.to_string()],
    )? > 0)
}
fn release(c: &Connection, turn: &str) -> Result<usize, StoreError> {
    Ok(c.execute("UPDATE inbox_events SET state='queued',turn_id=NULL,available_at=NULL,last_error=NULL WHERE turn_id=?1 AND state='in_turn'",[turn])?)
}
fn complete_event(c: &Connection, id: &str) -> Result<bool, StoreError> {
    Ok(c.execute("UPDATE inbox_events SET state='completed',turn_id=NULL,available_at=NULL,last_error=NULL WHERE event_id=?1 AND state IN('queued','in_turn')",[id])?>0)
}
fn dead_channel(c: &mut Connection, id: Uuid, error: &str) -> Result<Vec<String>, StoreError> {
    let tx = c.transaction()?;
    let mut q =
        tx.prepare("SELECT event_id FROM inbox_events WHERE channel_id=?1 AND state='queued'")?;
    let ids = q
        .query_map([id.to_string()], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(q);
    tx.execute("UPDATE inbox_events SET state='dead_letter',available_at=NULL,turn_id=NULL,last_error=?1 WHERE channel_id=?2 AND state='queued'",params![error,id.to_string()])?;
    tx.commit()?;
    Ok(ids)
}

fn create_job(
    c: &mut Connection,
    job: NewJob,
    outbox: OutboxEvent,
) -> Result<CreateJobOutcome, StoreError> {
    validate_request_outbox(&job, &outbox)?;
    create_job_record(c, job, Some(outbox), None)
}

fn create_assignment_job(
    c: &mut Connection,
    assignment_id: &str,
    job: NewJob,
    outbox: OutboxEvent,
) -> Result<CreateJobOutcome, StoreError> {
    validate_request_outbox(&job, &outbox)?;
    create_job_record(c, job, Some(outbox), Some(assignment_id))
}

fn validate_request_outbox(job: &NewJob, outbox: &OutboxEvent) -> Result<(), StoreError> {
    if outbox.event_id != job.request_event_id
        || outbox.job_id != Some(job.job_id)
        || outbox.channel_id != job.request.channel_id
        || outbox.kind != 43_001
        || outbox.is_terminal
        || outbox.ordering_key != format!("job:{}", job.job_id)
    {
        return Err(StoreError::InvalidData(
            "request outbox does not match job".into(),
        ));
    }
    validate_outbox(outbox)
}

fn create_remote_job(c: &mut Connection, job: NewJob) -> Result<CreateJobOutcome, StoreError> {
    create_job_record(c, job, None, None)
}
fn record_remote_cancel(
    c: &mut Connection,
    tombstone: RemoteCancelTombstone,
) -> Result<RecordCancelOutcome, StoreError> {
    if !lower_hex_64(&tombstone.request_event_id)
        || !lower_hex_64(&tombstone.cancel_event_id)
        || !lower_hex_64(&tombstone.canceller_pubkey)
    {
        return Err(StoreError::InvalidData(
            "remote cancel identity is invalid".into(),
        ));
    }
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(existing) = remote_cancel_by_event(&tx, &tombstone.cancel_event_id)? {
        let same_signed_identity = existing.job_id == tombstone.job_id
            && existing.request_event_id == tombstone.request_event_id
            && existing.channel_id == tombstone.channel_id
            && existing.canceller_pubkey == tombstone.canceller_pubkey;
        if !same_signed_identity {
            return Err(StoreError::InvalidData(
                "remote cancel event identity collision".into(),
            ));
        }
        if tombstone.authorized_without_request && !existing.authorized_without_request {
            tx.execute(
                "UPDATE job_cancel_tombstones
                 SET authorized_without_request=1
                 WHERE cancel_event_id=?1",
                [&tombstone.cancel_event_id],
            )?;
        }
        tx.commit()?;
        return Ok(RecordCancelOutcome::Duplicate);
    }
    let count: i64 = tx.query_row("SELECT COUNT(*) FROM job_cancel_tombstones", [], |row| {
        row.get(0)
    })?;
    if count >= MAX_REMOTE_CANCEL_TOMBSTONES as i64 {
        return Err(StoreError::CancelTombstoneCapacity);
    }
    tx.execute(
        "INSERT INTO job_cancel_tombstones(
            cancel_event_id,job_id,request_event_id,channel_id,canceller_pubkey,
            authorized_without_request,created_at
         )VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            tombstone.cancel_event_id,
            tombstone.job_id.to_string(),
            tombstone.request_event_id,
            tombstone.channel_id.to_string(),
            tombstone.canceller_pubkey,
            i64::from(tombstone.authorized_without_request),
            stamp(tombstone.created_at),
        ],
    )?;
    tx.commit()?;
    Ok(RecordCancelOutcome::Recorded)
}

fn remote_cancels(
    c: &Connection,
    job_id: JobId,
    request_event_id: &str,
) -> Result<Vec<RemoteCancelTombstone>, StoreError> {
    let mut statement = c.prepare(
        "SELECT cancel_event_id,job_id,request_event_id,channel_id,canceller_pubkey,
                authorized_without_request,created_at
         FROM job_cancel_tombstones
         WHERE job_id=?1 AND request_event_id=?2
         ORDER BY created_at,cancel_event_id",
    )?;
    let rows = statement
        .query_map(
            params![job_id.to_string(), request_event_id],
            remote_cancel_row,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter().map(parse_remote_cancel).collect()
}

fn discard_remote_cancels(
    c: &Connection,
    job_id: JobId,
    request_event_id: &str,
) -> Result<usize, StoreError> {
    Ok(c.execute(
        "DELETE FROM job_cancel_tombstones WHERE job_id=?1 AND request_event_id=?2",
        params![job_id.to_string(), request_event_id],
    )?)
}

fn create_cancelled_remote_job(
    c: &mut Connection,
    cancelled: CancelledRemoteJob,
) -> Result<CreateJobOutcome, StoreError> {
    let argv = validate_new_job(&cancelled.job)?;
    let event = &cancelled.terminal_event;
    if event.job_id != Some(cancelled.job.job_id)
        || event.channel_id != cancelled.job.request.channel_id
        || event.ordering_key != format!("job:{}", cancelled.job.job_id)
        || event.kind != 43_006
        || !event.is_terminal
        || event.seq.is_some()
    {
        return Err(StoreError::InvalidData(
            "cancelled remote job outbox mismatch".into(),
        ));
    }
    validate_outbox(event)?;
    if cancelled.result_json.len() > MAX_EVENT_JSON_BYTES {
        return Err(StoreError::EventTooLarge);
    }

    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(existing) = get_job_tx(&tx, cancelled.job.job_id)? {
        tx.commit()?;
        return Ok(CreateJobOutcome::Duplicate(existing));
    }
    let tombstone = remote_cancel_by_event(&tx, &cancelled.cancel_event_id)?
        .ok_or_else(|| StoreError::InvalidData("remote cancel tombstone not found".into()))?;
    if tombstone.job_id != cancelled.job.job_id
        || tombstone.request_event_id != cancelled.job.request_event_id
        || tombstone.channel_id != cancelled.job.request.channel_id
        || (!tombstone.authorized_without_request
            && tombstone.canceller_pubkey != cancelled.job.requester_pubkey)
    {
        return Err(StoreError::InvalidData(
            "remote cancel tombstone does not authorize this request".into(),
        ));
    }
    let at = stamp(event.created_at);
    tx.execute(
        "INSERT INTO jobs(
            job_id,request_event_id,source_event_id,channel_id,requester_pubkey,driver,
            executable,argv_json,cwd,summary,state,attempt,result_json,error_code,
            terminal_event_id,publication_state,created_at,finished_at,updated_at
         )VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'cancelled',?11,?12,
                  'cancelled_before_request',?13,'pending',?14,?15,?15)",
        params![
            cancelled.job.job_id.to_string(),
            cancelled.job.request_event_id,
            cancelled.job.request.source_event_id,
            cancelled.job.request.channel_id.to_string(),
            cancelled.job.requester_pubkey,
            cancelled.job.request.driver,
            cancelled.job.executable.to_string_lossy().into_owned(),
            argv,
            cancelled.job.request.cwd,
            cancelled.job.request.summary,
            cancelled.job.attempt as i64,
            cancelled.result_json,
            event.event_id,
            stamp(cancelled.job.created_at),
            at,
        ],
    )?;
    insert_outbox(&tx, event)?;
    tx.execute(
        "DELETE FROM job_cancel_tombstones WHERE job_id=?1 AND request_event_id=?2",
        params![
            cancelled.job.job_id.to_string(),
            cancelled.job.request_event_id
        ],
    )?;
    let record = get_job_tx(&tx, cancelled.job.job_id)?
        .ok_or_else(|| StoreError::InvalidData("created cancelled job disappeared".into()))?;
    tx.commit()?;
    Ok(CreateJobOutcome::Created(record))
}

type RemoteCancelSqlRow = (String, String, String, String, String, i64, String);

fn remote_cancel_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RemoteCancelSqlRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

fn parse_remote_cancel(row: RemoteCancelSqlRow) -> Result<RemoteCancelTombstone, StoreError> {
    Ok(RemoteCancelTombstone {
        cancel_event_id: row.0,
        job_id: Uuid::parse_str(&row.1)
            .map_err(|error| StoreError::InvalidData(error.to_string()))?,
        request_event_id: row.2,
        channel_id: Uuid::parse_str(&row.3)
            .map_err(|error| StoreError::InvalidData(error.to_string()))?,
        canceller_pubkey: row.4,
        authorized_without_request: row.5 != 0,
        created_at: parse_stamp(&row.6)?,
    })
}

fn remote_cancel_by_event(
    c: &Connection,
    cancel_event_id: &str,
) -> Result<Option<RemoteCancelTombstone>, StoreError> {
    let row = c
        .query_row(
            "SELECT cancel_event_id,job_id,request_event_id,channel_id,canceller_pubkey,
                    authorized_without_request,created_at
             FROM job_cancel_tombstones WHERE cancel_event_id=?1",
            [cancel_event_id],
            remote_cancel_row,
        )
        .optional()?;
    row.map(parse_remote_cancel).transpose()
}

fn lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_new_job(job: &NewJob) -> Result<String, StoreError> {
    job.request
        .validate()
        .map_err(|error| StoreError::InvalidData(error.to_string()))?;
    if job.attempt == 0 || !job.executable.is_absolute() {
        return Err(StoreError::InvalidData("invalid new job".into()));
    }
    let argv = serde_json::to_string(&job.request.argv)?;
    if argv.len() > MAX_ARGV_JSON_BYTES {
        return Err(StoreError::ArgvTooLarge);
    }
    Ok(argv)
}

fn create_job_record(
    c: &mut Connection,
    job: NewJob,
    outbox: Option<OutboxEvent>,
    assignment_id: Option<&str>,
) -> Result<CreateJobOutcome, StoreError> {
    let argv = validate_new_job(&job)?;
    let at = stamp(job.created_at);
    // An immediate transaction makes admission atomic across independently opened
    // runtime-store handles. `requested` is the reservation state: counting it
    // prevents two concurrent callers from both spawning before either runner is
    // accepted.
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(assignment_id) = assignment_id {
        let assignment =
            get_assignment(&tx, assignment_id)?.ok_or(StoreError::AssignmentJobMismatch)?;
        let is_current = active_assignment_tx(&tx)?
            .as_ref()
            .is_some_and(|active| active.assignment_id == assignment_id);
        let source_matches = assignment.source_event_id.is_some()
            && assignment.source_event_id == job.request.source_event_id;
        if assignment.state.is_terminal()
            || !is_current
            || assignment.channel_id != job.request.channel_id
            || !source_matches
        {
            return Err(StoreError::AssignmentJobMismatch);
        }
        if assignment
            .active_job_id
            .is_some_and(|existing| existing != job.job_id)
        {
            return Err(StoreError::ActiveJobExists);
        }
    }
    if let Some(existing) = get_job_tx(&tx, job.job_id)? {
        tx.commit()?;
        return Ok(CreateJobOutcome::Duplicate(existing));
    }
    let active_job_exists = tx
        .query_row(
            "SELECT 1 FROM jobs
             WHERE state IN('requested','accepted','running','cancelling')
             LIMIT 1",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if active_job_exists {
        return Err(StoreError::ActiveJobExists);
    }
    tx.execute("INSERT INTO jobs(job_id,request_event_id,source_event_id,channel_id,requester_pubkey,driver,executable,argv_json,cwd,summary,state,attempt,created_at,updated_at)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'requested',?11,?12,?12)",
        params![job.job_id.to_string(),job.request_event_id,job.request.source_event_id,job.request.channel_id.to_string(),job.requester_pubkey,job.request.driver,job.executable.to_string_lossy().into_owned(),argv,job.request.cwd,job.request.summary,job.attempt as i64,at])?;
    if let Some(event) = outbox.as_ref() {
        insert_outbox(&tx, event)?
    }
    if let Some(assignment_id) = assignment_id {
        tx.execute(
            "UPDATE assignments SET active_job_id=?1,state='working',last_progress_at=?2,
                reason=NULL,blocker=NULL,approval_gate_id=NULL,updated_at=?2
             WHERE assignment_id=?3",
            params![job.job_id.to_string(), &at, assignment_id],
        )?;
    }
    let record = get_job_tx(&tx, job.job_id)?
        .ok_or_else(|| StoreError::InvalidData("created job disappeared".into()))?;
    tx.commit()?;
    Ok(CreateJobOutcome::Created(record))
}

fn transition_job(
    c: &mut Connection,
    t: JobTransition,
    outbox: Option<OutboxEvent>,
) -> Result<JobRecord, StoreError> {
    if let Some(event) = outbox.as_ref() {
        validate_outbox(event)?;
        if event.job_id != Some(t.job_id) {
            return Err(StoreError::InvalidData(
                "transition outbox job mismatch".into(),
            ));
        }
    }
    let tx = c.transaction()?;
    let current = get_job_tx(&tx, t.job_id)?
        .ok_or_else(|| StoreError::InvalidData("job not found".into()))?;
    if let Some(event) = outbox.as_ref() {
        if event.channel_id != current.channel_id
            || event.ordering_key != format!("job:{}", t.job_id)
        {
            return Err(StoreError::InvalidData(
                "transition outbox scope mismatch".into(),
            ));
        }
    }
    if current.attempt != t.attempt
        || current.state.is_terminal()
        || !valid_job_transition(current.state, t.next_state)
    {
        return Err(StoreError::InvalidJobTransition {
            from: current.state,
            to: t.next_state,
        });
    }
    if let Some(seq) = t.progress_seq {
        if seq != current.progress_seq.saturating_add(1) {
            return Err(StoreError::InvalidData(
                "progress sequence must increase by one".into(),
            ));
        }
        let event = outbox
            .as_ref()
            .ok_or_else(|| StoreError::InvalidData("progress requires outbox event".into()))?;
        if event.kind != 43_003
            || event.is_terminal
            || event.seq != Some(seq)
            || !matches!(t.next_state, JobState::Running | JobState::Cancelling)
        {
            return Err(StoreError::InvalidData("progress outbox mismatch".into()));
        }
    } else if current.state == t.next_state {
        return Err(StoreError::InvalidData(
            "same-state transition requires progress".into(),
        ));
    }
    if t.next_state.is_terminal() {
        let terminal_id = t.terminal_event_id.as_deref().ok_or_else(|| {
            StoreError::InvalidData("terminal transition requires terminal event id".into())
        })?;
        let event = outbox.as_ref().ok_or_else(|| {
            StoreError::InvalidData("terminal transition requires outbox event".into())
        })?;
        let expected_kind = if t.next_state == JobState::Succeeded {
            43_004
        } else {
            43_006
        };
        if !event.is_terminal || event.event_id != terminal_id || event.kind != expected_kind {
            return Err(StoreError::InvalidData("terminal outbox mismatch".into()));
        }
    } else if outbox.as_ref().is_some_and(|event| event.is_terminal) {
        return Err(StoreError::InvalidData("terminal outbox mismatch".into()));
    }
    let state = job_state_text(t.next_state);
    let runner = t.runner.as_ref();
    let started = if matches!(t.next_state, JobState::Accepted | JobState::Running) {
        Some(stamp(t.occurred_at))
    } else {
        None
    };
    let finished = if t.next_state.is_terminal() {
        Some(stamp(t.occurred_at))
    } else {
        None
    };
    tx.execute("UPDATE jobs SET state=?1,runner_pid=COALESCE(?2,runner_pid),runner_start_marker=COALESCE(?3,runner_start_marker),process_group=COALESCE(?4,process_group),progress_seq=COALESCE(?5,progress_seq),exit_code=COALESCE(?6,exit_code),result_json=COALESCE(?7,result_json),error_code=COALESCE(?8,error_code),terminal_event_id=COALESCE(?9,terminal_event_id),publication_state=COALESCE(?10,publication_state),publication_error=COALESCE(?11,publication_error),started_at=COALESCE(started_at,?12),finished_at=COALESCE(finished_at,?13),updated_at=?14 WHERE job_id=?15 AND attempt=?16",
        params![state,runner.map(|v|v.pid as i64),runner.map(|v|v.start_marker.as_str()),runner.map(|v|v.process_group.as_str()),t.progress_seq.map(|v|v as i64),t.exit_code,t.result_json,t.error_code,t.terminal_event_id,t.publication_state.map(publication_state_text),t.publication_error,started,finished,stamp(t.occurred_at),t.job_id.to_string(),t.attempt as i64])?;
    if let Some(event) = outbox.as_ref() {
        insert_outbox(&tx, event)?
    }
    let record = get_job_tx(&tx, t.job_id)?
        .ok_or_else(|| StoreError::InvalidData("transitioned job disappeared".into()))?;
    tx.commit()?;
    Ok(record)
}

fn valid_job_transition(from: JobState, to: JobState) -> bool {
    matches!(
        (from, to),
        (
            JobState::Requested,
            JobState::Accepted | JobState::Running | JobState::Failed
        ) | (
            JobState::Accepted,
            JobState::Running | JobState::Cancelling | JobState::Failed | JobState::Lost
        ) | (
            JobState::Running,
            JobState::Running
                | JobState::Cancelling
                | JobState::Succeeded
                | JobState::Failed
                | JobState::Cancelled
                | JobState::Lost
        ) | (
            JobState::Cancelling,
            JobState::Cancelling | JobState::Cancelled | JobState::Failed | JobState::Lost
        )
    )
}

fn get_job(c: &Connection, id: JobId) -> Result<Option<JobRecord>, StoreError> {
    get_job_tx(c, id)
}
fn get_job_tx(c: &Connection, id: JobId) -> Result<Option<JobRecord>, StoreError> {
    c.query_row("SELECT job_id,request_event_id,source_event_id,channel_id,requester_pubkey,driver,executable,argv_json,cwd,summary,state,runner_pid,runner_start_marker,process_group,attempt,progress_seq,exit_code,result_json,error_code,terminal_event_id,publication_state,publication_error,created_at,started_at,finished_at,updated_at FROM jobs WHERE job_id=?1",[id.to_string()],read_job).optional().map_err(StoreError::from)
}
fn read_job(r: &rusqlite::Row<'_>) -> rusqlite::Result<JobRecord> {
    let job: String = r.get(0)?;
    let channel: String = r.get(3)?;
    let argv: String = r.get(7)?;
    let state: String = r.get(10)?;
    let pid: Option<i64> = r.get(11)?;
    let marker: Option<String> = r.get(12)?;
    let group: Option<String> = r.get(13)?;
    let publication: String = r.get(20)?;
    let created: String = r.get(22)?;
    let started: Option<String> = r.get(23)?;
    let finished: Option<String> = r.get(24)?;
    let updated: String = r.get(25)?;
    let runner = match (pid, marker, group) {
        (Some(pid), Some(start_marker), Some(process_group)) => Some(RunnerIdentity {
            pid: pid as u32,
            start_marker,
            process_group,
        }),
        (None, None, None) => None,
        _ => return Err(invalid("partial runner identity")),
    };
    Ok(JobRecord {
        job_id: Uuid::parse_str(&job).map_err(invalid)?,
        request_event_id: r.get(1)?,
        source_event_id: r.get(2)?,
        channel_id: Uuid::parse_str(&channel).map_err(invalid)?,
        requester_pubkey: r.get(4)?,
        driver: r.get(5)?,
        executable: r.get(6)?,
        argv: serde_json::from_str(&argv).map_err(invalid)?,
        cwd: r.get(8)?,
        summary: r.get(9)?,
        state: parse_job_state(&state).map_err(invalid)?,
        runner,
        attempt: r.get::<_, i64>(14)? as u32,
        progress_seq: r.get::<_, i64>(15)? as u64,
        exit_code: r.get(16)?,
        result_json: r.get(17)?,
        error_code: r.get(18)?,
        terminal_event_id: r.get(19)?,
        publication_state: parse_publication_state(&publication).map_err(invalid)?,
        publication_error: r.get(21)?,
        created_at: parse_stamp(&created).map_err(invalid)?,
        started_at: started
            .as_deref()
            .map(parse_stamp)
            .transpose()
            .map_err(invalid)?,
        finished_at: finished
            .as_deref()
            .map(parse_stamp)
            .transpose()
            .map_err(invalid)?,
        updated_at: parse_stamp(&updated).map_err(invalid)?,
    })
}

fn list_jobs(c: &Connection, filter: JobListFilter) -> Result<Vec<JobRecord>, StoreError> {
    let mut statement=c.prepare("SELECT job_id FROM jobs WHERE (?1 IS NULL OR channel_id=?1)AND(?2 IS NULL OR state=?2)ORDER BY created_at DESC,job_id")?;
    let channel = filter.channel_id.map(|v| v.to_string());
    let state = filter.state.map(job_state_text);
    let ids = statement
        .query_map(params![channel, state], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.into_iter()
        .map(|id| {
            Uuid::parse_str(&id)
                .map_err(|e| StoreError::InvalidData(e.to_string()))
                .and_then(|id| {
                    get_job(c, id)?
                        .ok_or_else(|| StoreError::InvalidData("listed job disappeared".into()))
                })
        })
        .collect()
}

fn validate_outbox(event: &OutboxEvent) -> Result<(), StoreError> {
    if event.event_json.len() > MAX_EVENT_JSON_BYTES {
        return Err(StoreError::EventTooLarge);
    }
    if event.event_id.is_empty() || event.ordering_key.is_empty() || event.job_id.is_none() {
        return Err(StoreError::InvalidData("invalid outbox event".into()));
    }
    match event.kind {
        43_003 if event.seq.is_some() && !event.is_terminal => Ok(()),
        43_004 | 43_006 if event.seq.is_none() && event.is_terminal => Ok(()),
        43_001 | 43_002 | 43_005 if event.seq.is_none() && !event.is_terminal => Ok(()),
        43_001..=43_006 => Err(StoreError::InvalidData(
            "invalid job outbox metadata".into(),
        )),
        _ => Err(StoreError::InvalidData("unsupported outbox kind".into())),
    }
}
fn insert_outbox(c: &Connection, event: &OutboxEvent) -> Result<(), StoreError> {
    if event.kind == 43_003 {
        c.execute("UPDATE relay_outbox SET state='superseded' WHERE job_id=?1 AND kind=43003 AND state='pending'",
            [event.job_id.map(|v|v.to_string())])?;
    }
    c.execute("INSERT INTO relay_outbox(event_id,job_id,channel_id,ordering_key,kind,seq,is_terminal,event_json,state,created_at)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending',?9)",
        params![event.event_id,event.job_id.map(|v|v.to_string()),event.channel_id.to_string(),event.ordering_key,event.kind as i64,event.seq.map(|v|v as i64),if event.is_terminal{1_i64}else{0_i64},event.event_json,stamp(event.created_at)])?;
    Ok(())
}
fn pending_outbox(
    connection: &Connection,
    limit: usize,
    now: DateTime<Utc>,
) -> Result<Vec<OutboxRecord>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT o.id,o.event_id,o.job_id,o.channel_id,o.ordering_key,o.kind,o.seq,
            o.is_terminal,o.event_json,o.attempt,o.created_at
         FROM relay_outbox o
         WHERE o.state='pending'
           AND(o.available_at IS NULL OR o.available_at<=?1)
           AND o.id=(
               SELECT MIN(i.id) FROM relay_outbox i
               WHERE i.ordering_key=o.ordering_key
                 AND i.state='pending'
           )
         ORDER BY o.id LIMIT ?2",
    )?;
    let rows = statement
        .query_map(params![stamp(now), limit.min(256) as i64], |row| {
            let job: Option<String> = row.get(2)?;
            let channel: String = row.get(3)?;
            let created: String = row.get(10)?;
            Ok(OutboxRecord {
                id: row.get(0)?,
                event: OutboxEvent {
                    event_id: row.get(1)?,
                    job_id: job
                        .as_deref()
                        .map(Uuid::parse_str)
                        .transpose()
                        .map_err(invalid)?,
                    channel_id: Uuid::parse_str(&channel).map_err(invalid)?,
                    ordering_key: row.get(4)?,
                    kind: row.get::<_, i64>(5)? as u16,
                    seq: row.get::<_, Option<i64>>(6)?.map(|value| value as u64),
                    is_terminal: row.get::<_, i64>(7)? != 0,
                    event_json: row.get(8)?,
                    created_at: parse_stamp(&created).map_err(invalid)?,
                },
                attempt: row.get::<_, i64>(9)? as u32,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
fn mark_outbox_published(
    connection: &mut Connection,
    id: i64,
    at: DateTime<Utc>,
) -> Result<bool, StoreError> {
    let transaction = connection.transaction()?;
    let row = transaction
        .query_row(
            "SELECT job_id,is_terminal FROM relay_outbox WHERE id=?1 AND state='pending'",
            [id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)? != 0)),
        )
        .optional()?;
    let Some((job_id, is_terminal)) = row else {
        transaction.commit()?;
        return Ok(false);
    };
    transaction.execute(
        "UPDATE relay_outbox SET state='published',published_at=?1,
            available_at=NULL,last_error=NULL WHERE id=?2 AND state='pending'",
        params![stamp(at), id],
    )?;
    if is_terminal {
        if let Some(job_id) = job_id {
            transaction.execute(
                "UPDATE jobs SET publication_state='published',publication_error=NULL,
                    updated_at=?1 WHERE job_id=?2",
                params![stamp(at), job_id],
            )?;
        }
    }
    transaction.commit()?;
    Ok(true)
}

fn mark_outbox_retry(
    connection: &Connection,
    id: i64,
    error: &str,
    available_at: DateTime<Utc>,
) -> Result<bool, StoreError> {
    Ok(connection.execute(
        "UPDATE relay_outbox SET attempt=attempt+1,available_at=?1,last_error=?2
         WHERE id=?3 AND state='pending'",
        params![stamp(available_at), error, id],
    )? > 0)
}

fn mark_outbox_rejected(
    connection: &mut Connection,
    id: i64,
    reason: &str,
    at: DateTime<Utc>,
) -> Result<bool, StoreError> {
    let transaction = connection.transaction()?;
    let row = transaction
        .query_row(
            "SELECT job_id,is_terminal FROM relay_outbox WHERE id=?1 AND state='pending'",
            [id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)? != 0)),
        )
        .optional()?;
    let Some((job_id, is_terminal)) = row else {
        transaction.commit()?;
        return Ok(false);
    };
    transaction.execute(
        "UPDATE relay_outbox SET state='rejected',rejected_at=?1,
            available_at=NULL,last_error=?2 WHERE id=?3 AND state='pending'",
        params![stamp(at), reason, id],
    )?;
    if is_terminal {
        if let Some(job_id) = job_id {
            transaction.execute(
                "UPDATE jobs SET publication_state='failed',publication_error=?1,
                    updated_at=?2 WHERE job_id=?3",
                params![reason, stamp(at), job_id],
            )?;
        }
    }
    transaction.commit()?;
    Ok(true)
}

const ACTIVE_ASSIGNMENT_STATES: &str =
    "'reading','working','waiting','needs_approval','blocked','recovering'";

fn validate_assignment_record(record: &AssignmentRecord) -> Result<(), StoreError> {
    if record.assignment_id.trim().is_empty()
        || record.assignment_id.len() > MAX_ASSIGNMENT_TEXT_BYTES
        || record.summary.trim().is_empty()
        || record.summary.len() > MAX_ASSIGNMENT_TEXT_BYTES
    {
        return Err(StoreError::InvalidAssignment(
            "identity and summary are required and bounded".into(),
        ));
    }
    if record.source_event_id.as_deref().is_some_and(|value| {
        value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) {
        return Err(StoreError::InvalidAssignment(
            "source event id must be lowercase hexadecimal".into(),
        ));
    }
    if record
        .session_id
        .as_deref()
        .is_some_and(|value| value.is_empty() || value.len() > MAX_ASSIGNMENT_TEXT_BYTES)
    {
        return Err(StoreError::InvalidAssignment(
            "session id is required and bounded when present".into(),
        ));
    }
    if record.state != AssignmentState::Reading
        || record.active_job_id.is_some()
        || record.reason.is_some()
        || record.blocker.is_some()
        || record.approval_gate_id.is_some()
        || record.delivery_evidence.is_some()
    {
        return Err(StoreError::InvalidAssignment(
            "new assignment must begin in reading state".into(),
        ));
    }
    Ok(())
}

fn create_assignment(
    connection: &mut Connection,
    assignment: AssignmentRecord,
) -> Result<AssignmentRecord, StoreError> {
    validate_assignment_record(&assignment)?;
    let transaction = connection.transaction()?;
    if let Some(active) = active_assignment_tx(&transaction)? {
        return Err(StoreError::InvalidAssignment(format!(
            "assignment {} is already active",
            active.assignment_id
        )));
    }
    insert_assignment(&transaction, &assignment)?;
    transaction.commit()?;
    Ok(assignment)
}

fn claim_assignment(
    connection: &mut Connection,
    channel_id: Uuid,
    source_event_id: Option<String>,
    summary: String,
    session_id: Option<String>,
    now: DateTime<Utc>,
) -> Result<AssignmentRecord, StoreError> {
    let transaction = connection.transaction()?;
    if let Some(active) = active_assignment_tx(&transaction)? {
        transaction.commit()?;
        return Ok(active);
    }
    let assignment = AssignmentRecord {
        assignment_id: Uuid::new_v4().to_string(),
        source_event_id,
        channel_id,
        state: AssignmentState::Reading,
        summary,
        active_job_id: None,
        session_id,
        reply_event_id: None,
        last_progress_at: now,
        reason: None,
        blocker: None,
        approval_gate_id: None,
        delivery_evidence: None,
        updated_at: now,
    };
    validate_assignment_record(&assignment)?;
    insert_assignment(&transaction, &assignment)?;
    transaction.commit()?;
    Ok(assignment)
}

fn insert_assignment(
    connection: &Connection,
    assignment: &AssignmentRecord,
) -> Result<(), StoreError> {
    connection.execute(
        "INSERT INTO assignments(
            assignment_id,source_event_id,channel_id,state,summary,active_job_id,
            session_id,reply_event_id,last_progress_at,reason,blocker,
            approval_gate_id,delivery_evidence,updated_at
        )VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            assignment.assignment_id,
            assignment.source_event_id,
            assignment.channel_id.to_string(),
            assignment_state_text(assignment.state),
            assignment.summary,
            assignment.active_job_id.map(|value| value.to_string()),
            assignment.session_id,
            assignment.reply_event_id,
            stamp(assignment.last_progress_at),
            assignment.reason,
            assignment.blocker,
            assignment.approval_gate_id,
            assignment.delivery_evidence,
            stamp(assignment.updated_at),
        ],
    )?;
    Ok(())
}

fn active_assignment(connection: &Connection) -> Result<Option<AssignmentRecord>, StoreError> {
    active_assignment_tx(connection)
}

fn active_assignment_tx(connection: &Connection) -> Result<Option<AssignmentRecord>, StoreError> {
    connection
        .query_row(
            &format!(
                "SELECT assignment_id,source_event_id,channel_id,state,summary,
                    active_job_id,session_id,reply_event_id,last_progress_at,reason,
                    blocker,approval_gate_id,delivery_evidence,updated_at
                 FROM assignments WHERE state IN({ACTIVE_ASSIGNMENT_STATES}) LIMIT 1"
            ),
            [],
            read_assignment,
        )
        .optional()
        .map_err(StoreError::from)
}

fn get_assignment(
    connection: &Connection,
    assignment_id: &str,
) -> Result<Option<AssignmentRecord>, StoreError> {
    connection
        .query_row(
            "SELECT assignment_id,source_event_id,channel_id,state,summary,
                active_job_id,session_id,reply_event_id,last_progress_at,reason,
                blocker,approval_gate_id,delivery_evidence,updated_at
             FROM assignments WHERE assignment_id=?1",
            [assignment_id],
            read_assignment,
        )
        .optional()
        .map_err(StoreError::from)
}

fn read_assignment(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssignmentRecord> {
    let channel_id: String = row.get(2)?;
    let state: String = row.get(3)?;
    let active_job_id: Option<String> = row.get(5)?;
    let last_progress_at: String = row.get(8)?;
    let updated_at: String = row.get(13)?;
    Ok(AssignmentRecord {
        assignment_id: row.get(0)?,
        source_event_id: row.get(1)?,
        channel_id: Uuid::parse_str(&channel_id).map_err(invalid)?,
        state: parse_assignment_state(&state).map_err(invalid)?,
        summary: row.get(4)?,
        active_job_id: active_job_id
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(invalid)?,
        session_id: row.get(6)?,
        reply_event_id: row.get(7)?,
        last_progress_at: parse_stamp(&last_progress_at).map_err(invalid)?,
        reason: row.get(9)?,
        blocker: row.get(10)?,
        approval_gate_id: row.get(11)?,
        delivery_evidence: row.get(12)?,
        updated_at: parse_stamp(&updated_at).map_err(invalid)?,
    })
}

fn set_assignment_state(
    connection: &mut Connection,
    assignment_id: &str,
    request: AssignmentSetStateRequest,
    now: DateTime<Utc>,
) -> Result<AssignmentRecord, StoreError> {
    request
        .validate()
        .map_err(|error| StoreError::InvalidAssignment(error.to_string()))?;
    let transaction = connection.transaction()?;
    let current = get_assignment(&transaction, assignment_id)?
        .ok_or_else(|| StoreError::AssignmentNotFound(assignment_id.into()))?;
    if current.state.is_terminal() {
        return Err(StoreError::TerminalAssignment {
            assignment_id: assignment_id.into(),
            state: current.state,
        });
    }
    let active = active_assignment_tx(&transaction)?;
    if active.as_ref().map(|value| value.assignment_id.as_str()) != Some(assignment_id) {
        return Err(StoreError::AssignmentNotCurrent(assignment_id.into()));
    }
    if !valid_assignment_transition(current.state, request.state) {
        return Err(StoreError::InvalidAssignmentTransition {
            from: current.state,
            to: request.state,
        });
    }
    let evidence = request
        .delivery_evidence
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    if request.state == AssignmentState::Completed {
        let linked_job_succeeded = current
            .active_job_id
            .map(|job_id| get_job_tx(&transaction, job_id))
            .transpose()?
            .flatten()
            .is_some_and(|job| job.state == JobState::Succeeded);
        if !linked_job_succeeded && !evidence {
            return Err(StoreError::AssignmentCompletionUnverified);
        }
    }
    let summary = request.summary.unwrap_or_else(|| current.summary.clone());
    if summary.trim().is_empty() || summary.len() > MAX_ASSIGNMENT_TEXT_BYTES {
        return Err(StoreError::InvalidAssignment(
            "summary is required and bounded".into(),
        ));
    }
    let reason = matches!(
        request.state,
        AssignmentState::Waiting | AssignmentState::Failed | AssignmentState::Cancelled
    )
    .then_some(request.reason)
    .flatten();
    let blocker = (request.state == AssignmentState::Blocked)
        .then_some(request.blocker)
        .flatten();
    let approval_gate_id = (request.state == AssignmentState::NeedsApproval)
        .then_some(request.approval_gate_id)
        .flatten();
    let delivery_evidence = (request.state == AssignmentState::Completed)
        .then_some(request.delivery_evidence)
        .flatten();
    transaction.execute(
        "UPDATE assignments SET state=?1,summary=?2,reply_event_id=COALESCE(?3,reply_event_id),
            last_progress_at=?4,reason=?5,blocker=?6,approval_gate_id=?7,
            delivery_evidence=?8,updated_at=?4 WHERE assignment_id=?9",
        params![
            assignment_state_text(request.state),
            summary,
            request.reply_event_id,
            stamp(now),
            reason,
            blocker,
            approval_gate_id,
            delivery_evidence,
            assignment_id,
        ],
    )?;
    if request.state.is_terminal() {
        transaction.execute(
            "INSERT INTO runtime_meta(key,value)VALUES('hot_terminal_assignment',?1)
             ON CONFLICT(key)DO UPDATE SET value=excluded.value",
            [assignment_id],
        )?;
    }
    let updated = get_assignment(&transaction, assignment_id)?
        .ok_or_else(|| StoreError::AssignmentNotFound(assignment_id.into()))?;
    transaction.commit()?;
    Ok(updated)
}

fn valid_assignment_transition(from: AssignmentState, to: AssignmentState) -> bool {
    !from.is_terminal()
        && (from == to
            || matches!(
                to,
                AssignmentState::Reading
                    | AssignmentState::Working
                    | AssignmentState::Waiting
                    | AssignmentState::NeedsApproval
                    | AssignmentState::Blocked
                    | AssignmentState::Recovering
                    | AssignmentState::Completed
                    | AssignmentState::Failed
                    | AssignmentState::Cancelled
            ))
}

fn link_assignment_job(
    connection: &mut Connection,
    assignment_id: &str,
    job_id: JobId,
    now: DateTime<Utc>,
) -> Result<AssignmentRecord, StoreError> {
    let transaction = connection.transaction()?;
    let current = get_assignment(&transaction, assignment_id)?
        .ok_or_else(|| StoreError::AssignmentNotFound(assignment_id.into()))?;
    if current.state.is_terminal() {
        return Err(StoreError::TerminalAssignment {
            assignment_id: assignment_id.into(),
            state: current.state,
        });
    }
    if active_assignment_tx(&transaction)?
        .as_ref()
        .map(|value| value.assignment_id.as_str())
        != Some(assignment_id)
    {
        return Err(StoreError::AssignmentNotCurrent(assignment_id.into()));
    }
    if current
        .active_job_id
        .is_some_and(|existing| existing != job_id)
    {
        return Err(StoreError::InvalidAssignment(
            "assignment already links another job".into(),
        ));
    }
    let job = get_job_tx(&transaction, job_id)?
        .ok_or_else(|| StoreError::InvalidData("linked job not found".into()))?;
    if job.channel_id != current.channel_id
        || (job.source_event_id.is_some()
            && current.source_event_id.is_some()
            && job.source_event_id != current.source_event_id)
    {
        return Err(StoreError::InvalidAssignment(
            "linked job source does not match assignment".into(),
        ));
    }
    transaction.execute(
        "UPDATE assignments SET active_job_id=?1,state='working',last_progress_at=?2,
            reason=NULL,blocker=NULL,approval_gate_id=NULL,updated_at=?2
         WHERE assignment_id=?3",
        params![job_id.to_string(), stamp(now), assignment_id],
    )?;
    let updated = get_assignment(&transaction, assignment_id)?
        .ok_or_else(|| StoreError::AssignmentNotFound(assignment_id.into()))?;
    transaction.commit()?;
    Ok(updated)
}

fn clear_terminal_assignment(connection: &Connection) -> Result<bool, StoreError> {
    Ok(connection.execute(
        "DELETE FROM runtime_meta WHERE key='hot_terminal_assignment'",
        [],
    )? > 0)
}

fn begin_startup_recovery(
    connection: &mut Connection,
    reason: &str,
) -> Result<StartupRecoverySnapshot, StoreError> {
    // Commit the recovery marker before the first nonterminal-state query. If
    // any later query or reconciliation fails, the durable state stays truthful.
    set_recovery_state(connection, true, Some(reason.to_owned()))?;
    let transaction = connection.transaction()?;
    for phase in [
        StartupRecoveryPhase::Inbox,
        StartupRecoveryPhase::Sessions,
        StartupRecoveryPhase::Assignments,
        StartupRecoveryPhase::Runners,
    ] {
        transaction.execute(
            "INSERT INTO runtime_meta(key,value)VALUES(?1,'1')
             ON CONFLICT(key)DO UPDATE SET value='1'",
            [phase.key()],
        )?;
    }
    transaction.commit()?;

    let snapshot = assignment_snapshot(connection)?;
    let sessions = channel_session_ids(connection)?;
    Ok(StartupRecoverySnapshot {
        in_turn_inbox: snapshot.queue_depths.in_turn,
        active_assignment: snapshot.active_assignment,
        active_jobs: snapshot.active_jobs,
        channel_sessions: sessions,
    })
}

fn complete_startup_recovery_phase(
    connection: &mut Connection,
    phase: StartupRecoveryPhase,
) -> Result<bool, StoreError> {
    let transaction = connection.transaction()?;
    transaction.execute("DELETE FROM runtime_meta WHERE key=?1", [phase.key()])?;
    let pending: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM runtime_meta WHERE key LIKE 'recovery_pending_%'",
        [],
        |row| row.get(0),
    )?;
    let complete = pending == 0;
    if complete {
        transaction.execute(
            "DELETE FROM runtime_meta WHERE key IN('recovering','recovery_reason')",
            [],
        )?;
    }
    transaction.commit()?;
    Ok(complete)
}
fn set_recovery_state(
    connection: &mut Connection,
    recovering: bool,
    reason: Option<String>,
) -> Result<(), StoreError> {
    let transaction = connection.transaction()?;
    if recovering {
        transaction.execute(
            "INSERT INTO runtime_meta(key,value)VALUES('recovering','1')
             ON CONFLICT(key)DO UPDATE SET value='1'",
            [],
        )?;
        transaction.execute("DELETE FROM runtime_meta WHERE key='recovery_reason'", [])?;
        if let Some(reason) = reason.as_deref().and_then(safe_recovery_reason) {
            transaction.execute(
                "INSERT INTO runtime_meta(key,value)VALUES('recovery_reason',?1)
                 ON CONFLICT(key)DO UPDATE SET value=excluded.value",
                [reason],
            )?;
        }
    } else {
        let pending: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM runtime_meta WHERE key LIKE 'recovery_pending_%'",
            [],
            |row| row.get(0),
        )?;
        if pending > 0 {
            return Err(StoreError::InvalidData(
                "startup recovery components remain pending".into(),
            ));
        }
        transaction.execute(
            "DELETE FROM runtime_meta
             WHERE key IN('recovering','recovery_reason')
                OR key LIKE 'recovery_pending_%'",
            [],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn safe_recovery_reason(reason: &str) -> Option<&'static str> {
    match reason {
        "runtime_restart" => Some("runtime_restart"),
        "inbox_reconciliation" => Some("inbox_reconciliation"),
        "assignment_reconciliation" => Some("assignment_reconciliation"),
        "job_reconciliation" => Some("job_reconciliation"),
        "runner_reconciliation" => Some("runner_reconciliation"),
        "session_reconciliation" => Some("session_reconciliation"),
        _ if reason.trim().is_empty() => None,
        _ => Some("runtime_reconciliation"),
    }
}

fn operational_diagnostics(connection: &Connection) -> Result<StoreDiagnostics, StoreError> {
    let schema: String = connection.query_row(
        "SELECT value FROM runtime_meta WHERE key='schema_version'",
        [],
        |row| row.get(0),
    )?;
    let schema_version = schema.parse::<u32>().map_err(|error| {
        StoreError::InvalidData(format!("invalid store schema version: {error}"))
    })?;
    let published: Option<String> = connection.query_row(
        "SELECT MAX(published_at) FROM relay_outbox WHERE kind=43003 AND state='published'",
        [],
        |row| row.get(0),
    )?;
    Ok(StoreDiagnostics {
        schema_version,
        last_relay_progress_published_at: published.as_deref().map(parse_stamp).transpose()?,
    })
}

fn assignment_snapshot(connection: &mut Connection) -> Result<AssignmentSnapshot, StoreError> {
    let transaction = connection.transaction()?;
    let queue_depths = depths(&transaction)?;
    let active_assignment = active_assignment_tx(&transaction)?;
    let active_jobs = {
        let mut statement = transaction.prepare(
            "SELECT job_id FROM jobs
             WHERE state IN('requested','accepted','running','cancelling')
             ORDER BY created_at,job_id",
        )?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .map(|row| {
                row.map_err(StoreError::from).and_then(|value| {
                    Uuid::parse_str(&value)
                        .map_err(|error| StoreError::InvalidData(error.to_string()))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let terminal_assignment = transaction
        .query_row(
            "SELECT value FROM runtime_meta WHERE key='hot_terminal_assignment'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|assignment_id| get_assignment(&transaction, &assignment_id))
        .transpose()?
        .flatten();
    let recovering = transaction
        .query_row(
            "SELECT 1 FROM runtime_meta WHERE key='recovering'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    let recovery_reason = transaction
        .query_row(
            "SELECT value FROM runtime_meta WHERE key='recovery_reason'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    transaction.commit()?;
    Ok(AssignmentSnapshot {
        queue_depths,
        active_assignment,
        terminal_assignment,
        active_jobs,
        recovering,
        recovery_reason,
    })
}

fn job_state_text(v: JobState) -> &'static str {
    match v {
        JobState::Requested => "requested",
        JobState::Accepted => "accepted",
        JobState::Running => "running",
        JobState::Cancelling => "cancelling",
        JobState::Succeeded => "succeeded",
        JobState::Failed => "failed",
        JobState::Cancelled => "cancelled",
        JobState::Lost => "lost",
    }
}
fn parse_job_state(v: &str) -> Result<JobState, String> {
    match v {
        "requested" => Ok(JobState::Requested),
        "accepted" => Ok(JobState::Accepted),
        "running" => Ok(JobState::Running),
        "cancelling" => Ok(JobState::Cancelling),
        "succeeded" => Ok(JobState::Succeeded),
        "failed" => Ok(JobState::Failed),
        "cancelled" => Ok(JobState::Cancelled),
        "lost" => Ok(JobState::Lost),
        _ => Err(format!("invalid job state {v}")),
    }
}
fn publication_state_text(v: PublicationState) -> &'static str {
    match v {
        PublicationState::NotStarted => "not_started",
        PublicationState::Pending => "pending",
        PublicationState::Published => "published",
        PublicationState::Failed => "failed",
    }
}
fn parse_publication_state(v: &str) -> Result<PublicationState, String> {
    match v {
        "not_started" => Ok(PublicationState::NotStarted),
        "pending" => Ok(PublicationState::Pending),
        "published" => Ok(PublicationState::Published),
        "failed" => Ok(PublicationState::Failed),
        _ => Err(format!("invalid publication state {v}")),
    }
}
fn assignment_state_text(v: AssignmentState) -> &'static str {
    match v {
        AssignmentState::Reading => "reading",
        AssignmentState::Working => "working",
        AssignmentState::Waiting => "waiting",
        AssignmentState::NeedsApproval => "needs_approval",
        AssignmentState::Blocked => "blocked",
        AssignmentState::Recovering => "recovering",
        AssignmentState::Completed => "completed",
        AssignmentState::Failed => "failed",
        AssignmentState::Cancelled => "cancelled",
    }
}
fn parse_assignment_state(v: &str) -> Result<AssignmentState, String> {
    match v {
        "reading" => Ok(AssignmentState::Reading),
        "working" => Ok(AssignmentState::Working),
        "waiting" => Ok(AssignmentState::Waiting),
        "needs_approval" => Ok(AssignmentState::NeedsApproval),
        "blocked" => Ok(AssignmentState::Blocked),
        "recovering" => Ok(AssignmentState::Recovering),
        "completed" => Ok(AssignmentState::Completed),
        "failed" => Ok(AssignmentState::Failed),
        "cancelled" => Ok(AssignmentState::Cancelled),
        _ => Err(format!("invalid assignment state {v}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};

    fn input(channel_id: Uuid, content: &str) -> InboxEvent {
        InboxEvent {
            channel_id,
            event: EventBuilder::new(Kind::Custom(9), content)
                .sign_with_keys(&Keys::generate())
                .unwrap(),
            received_at: Utc::now(),
        }
    }
    fn assignment(state: AssignmentState) -> AssignmentRecord {
        let now = Utc::now();
        AssignmentRecord {
            assignment_id: Uuid::new_v4().to_string(),
            source_event_id: Some("a".repeat(64)),
            channel_id: Uuid::new_v4(),
            state,
            summary: "durable assignment".into(),
            active_job_id: None,
            session_id: Some("session".into()),
            reply_event_id: None,
            last_progress_at: now,
            reason: (state == AssignmentState::Waiting).then(|| "coworker reply".into()),
            blocker: (state == AssignmentState::Blocked).then(|| "dependency unavailable".into()),
            approval_gate_id: (state == AssignmentState::NeedsApproval).then(|| "gate".into()),
            delivery_evidence: None,
            updated_at: now,
        }
    }

    fn job(state: JobState) -> JobRecord {
        let now = Utc::now();
        JobRecord {
            job_id: Uuid::new_v4(),
            request_event_id: Some("request".into()),
            source_event_id: Some("a".repeat(64)),
            channel_id: Uuid::new_v4(),
            requester_pubkey: "requester".into(),
            driver: "lh".into(),
            executable: "/bin/echo".into(),
            argv: vec![],
            cwd: "/tmp".into(),
            summary: "job".into(),
            state,
            runner: None,
            attempt: 1,
            progress_seq: 0,
            exit_code: None,
            result_json: None,
            error_code: None,
            terminal_event_id: None,
            publication_state: PublicationState::NotStarted,
            publication_error: None,
            created_at: now,
            started_at: None,
            finished_at: None,
            updated_at: now,
        }
    }

    fn state_request(state: AssignmentState) -> AssignmentSetStateRequest {
        AssignmentSetStateRequest {
            state,
            summary: None,
            reason: (state == AssignmentState::Waiting).then(|| "awaiting review".into()),
            blocker: (state == AssignmentState::Blocked).then(|| "missing input".into()),
            approval_gate_id: (state == AssignmentState::NeedsApproval).then(|| "gate-1".into()),
            delivery_evidence: None,
            reply_event_id: None,
        }
    }

    async fn create_requested_job(
        store: &StoreHandle,
        channel_id: Uuid,
        source_event_id: String,
    ) -> JobId {
        let job_id = Uuid::new_v4();
        let request_event_id = format!("request-{job_id}");
        let request = JobStartRequest {
            channel_id,
            source_event_id: Some(source_event_id),
            driver: "lh".into(),
            argv: vec!["run".into()],
            cwd: "/tmp".into(),
            summary: "durable job".into(),
        };
        store
            .create_local_job(
                NewJob {
                    job_id,
                    request_event_id: request_event_id.clone(),
                    requester_pubkey: "requester".into(),
                    executable: PathBuf::from("/bin/echo"),
                    request,
                    attempt: 1,
                    created_at: Utc::now(),
                },
                OutboxEvent {
                    event_id: request_event_id,
                    job_id: Some(job_id),
                    channel_id,
                    ordering_key: format!("job:{job_id}"),
                    kind: 43_001,
                    seq: None,
                    is_terminal: false,
                    event_json: "{}".into(),
                    created_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        job_id
    }
    fn remote_job(job_id: JobId, channel_id: Uuid) -> NewJob {
        NewJob {
            job_id,
            request_event_id: format!("request-{job_id}"),
            requester_pubkey: "requester".into(),
            executable: std::env::current_exe().unwrap().canonicalize().unwrap(),
            request: JobStartRequest {
                channel_id,
                source_event_id: None,
                driver: "lh".into(),
                argv: vec!["run".into()],
                cwd: "/tmp".into(),
                summary: "durable job".into(),
            },
            attempt: 1,
            created_at: Utc::now(),
        }
    }
    fn request_outbox(job: &NewJob) -> OutboxEvent {
        OutboxEvent {
            event_id: job.request_event_id.clone(),
            job_id: Some(job.job_id),
            channel_id: job.request.channel_id,
            ordering_key: format!("job:{}", job.job_id),
            kind: 43_001,
            seq: None,
            is_terminal: false,
            event_json: "{}".into(),
            created_at: job.created_at.to_owned(),
        }
    }

    fn terminal_failure(job: &JobRecord) -> (JobTransition, OutboxEvent) {
        let occurred_at = Utc::now();
        let event_id = format!("terminal-{}", job.job_id);
        (
            JobTransition {
                job_id: job.job_id,
                attempt: job.attempt,
                next_state: JobState::Failed,
                runner: None,
                progress_seq: None,
                exit_code: Some(1),
                result_json: None,
                error_code: Some("test_failure".into()),
                terminal_event_id: Some(event_id.clone()),
                publication_state: Some(PublicationState::Pending),
                publication_error: None,
                occurred_at,
            },
            OutboxEvent {
                event_id,
                job_id: Some(job.job_id),
                channel_id: job.channel_id,
                ordering_key: format!("job:{}", job.job_id),
                kind: 43_006,
                seq: None,
                is_terminal: true,
                event_json: "{}".into(),
                created_at: occurred_at,
            },
        )
    }

    #[tokio::test]
    async fn concurrent_distinct_job_admission_reserves_one_runner_slot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("runtime.sqlite3");
        let first_store = StoreHandle::open(&path).unwrap();
        let second_store = StoreHandle::open(&path).unwrap();
        let channel_id = Uuid::new_v4();
        let first_id = Uuid::new_v4();
        let second_id = Uuid::new_v4();
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));
        let runner_launches = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let launch =
            |store: StoreHandle,
             job: NewJob,
             barrier: std::sync::Arc<tokio::sync::Barrier>,
             runner_launches: std::sync::Arc<std::sync::atomic::AtomicUsize>| async move {
                barrier.wait().await;
                let outcome = store.create_remote_job(job).await;
                if matches!(&outcome, Ok(CreateJobOutcome::Created(_))) {
                    runner_launches.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                outcome
            };
        let first = tokio::spawn(launch(
            first_store.clone(),
            remote_job(first_id, channel_id),
            barrier.clone(),
            runner_launches.clone(),
        ));
        let second = tokio::spawn(launch(
            second_store.clone(),
            remote_job(second_id, channel_id),
            barrier.clone(),
            runner_launches.clone(),
        ));
        barrier.wait().await;
        let outcomes = [first.await.unwrap(), second.await.unwrap()];

        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Ok(CreateJobOutcome::Created(_))))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Err(StoreError::ActiveJobExists)))
                .count(),
            1
        );
        assert_eq!(
            runner_launches.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "only the durably admitted caller may spawn a runner"
        );
        assert_eq!(
            first_store
                .list_jobs(JobListFilter::default())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn active_job_replay_is_idempotent_and_terminal_state_releases_slot() {
        let dir = tempfile::tempdir().unwrap();
        let store = StoreHandle::open(dir.path().join("state").join("runtime.sqlite3")).unwrap();
        let channel_id = Uuid::new_v4();
        let first_id = Uuid::new_v4();
        let first_job = remote_job(first_id, channel_id);
        let created = store.create_remote_job(first_job.clone()).await.unwrap();
        let CreateJobOutcome::Created(first_record) = created else {
            panic!("first job must be created");
        };

        assert!(matches!(
            store.create_remote_job(first_job).await.unwrap(),
            CreateJobOutcome::Duplicate(record) if record.job_id == first_id
        ));
        assert!(matches!(
            store
                .create_remote_job(remote_job(Uuid::new_v4(), channel_id))
                .await,
            Err(StoreError::ActiveJobExists)
        ));

        let (transition, outbox) = terminal_failure(&first_record);
        store
            .transition_job(transition, Some(outbox))
            .await
            .unwrap();
        let next_id = Uuid::new_v4();
        assert!(matches!(
            store
                .create_remote_job(remote_job(next_id, channel_id))
                .await
                .unwrap(),
            CreateJobOutcome::Created(record) if record.job_id == next_id
        ));
    }

    #[tokio::test]
    async fn assignment_bound_admission_fails_closed_without_current_match() {
        let dir = tempfile::tempdir().unwrap();
        let store = StoreHandle::open(dir.path().join("state").join("runtime.sqlite3")).unwrap();
        let channel_id = Uuid::new_v4();
        let missing_job = remote_job(Uuid::new_v4(), channel_id);
        let missing_outbox = request_outbox(&missing_job);
        assert!(matches!(
            store
                .create_local_job_for_assignment("missing-assignment", missing_job, missing_outbox,)
                .await,
            Err(StoreError::AssignmentJobMismatch)
        ));

        let current = store
            .create_assignment(assignment(AssignmentState::Reading))
            .await
            .unwrap();
        let mut mismatched = remote_job(Uuid::new_v4(), current.channel_id);
        mismatched.request.source_event_id = Some("b".repeat(64));
        let mismatched_outbox = request_outbox(&mismatched);
        assert!(matches!(
            store
                .create_local_job_for_assignment(
                    &current.assignment_id,
                    mismatched,
                    mismatched_outbox,
                )
                .await,
            Err(StoreError::AssignmentJobMismatch)
        ));
        assert!(store
            .list_jobs(JobListFilter::default())
            .await
            .unwrap()
            .is_empty());
        assert!(store
            .pending_outbox(10, Utc::now())
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn assignment_bound_and_remote_admission_share_one_atomic_slot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("runtime.sqlite3");
        let model_store = StoreHandle::open(&path).unwrap();
        let remote_store = StoreHandle::open(&path).unwrap();
        let current = model_store
            .create_assignment(assignment(AssignmentState::Reading))
            .await
            .unwrap();
        let mut model_job = remote_job(Uuid::new_v4(), current.channel_id);
        model_job.request.source_event_id = current.source_event_id.clone();
        let model_outbox = request_outbox(&model_job);
        let remote_job = remote_job(Uuid::new_v4(), current.channel_id);
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));

        let model_barrier = barrier.clone();
        let assignment_id = current.assignment_id.clone();
        let model = tokio::spawn(async move {
            model_barrier.wait().await;
            model_store
                .create_local_job_for_assignment(&assignment_id, model_job, model_outbox)
                .await
        });
        let remote_barrier = barrier.clone();
        let remote = tokio::spawn(async move {
            remote_barrier.wait().await;
            remote_store.create_remote_job(remote_job).await
        });
        barrier.wait().await;
        let outcomes = [model.await.unwrap(), remote.await.unwrap()];

        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Ok(CreateJobOutcome::Created(_))))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Err(StoreError::ActiveJobExists)))
                .count(),
            1
        );
        let read_store = StoreHandle::open(&path).unwrap();
        let jobs = read_store
            .list_jobs(JobListFilter::default())
            .await
            .unwrap();
        assert_eq!(jobs.len(), 1);
        let outbox = read_store.pending_outbox(10, Utc::now()).await.unwrap();
        assert!(outbox.is_empty() || outbox[0].event.job_id == Some(jobs[0].job_id));
    }

    #[tokio::test]
    async fn accepted_row_survives_reopen_and_replay_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("runtime.sqlite3");
        let channel = Uuid::new_v4();
        let original = input(channel, "once");
        let replay = original.clone();
        let store = StoreHandle::open(&path).unwrap();
        assert_eq!(
            store.enqueue_inbox(original).await.unwrap(),
            EnqueueOutcome::Enqueued
        );
        drop(store);
        let reopened = StoreHandle::open(&path).unwrap();
        assert_eq!(
            reopened.enqueue_inbox(replay).await.unwrap(),
            EnqueueOutcome::Duplicate
        );
        assert_eq!(
            reopened
                .claim_inbox_batch(50, "turn".into(), Utc::now())
                .await
                .unwrap()
                .unwrap()
                .events
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn in_turn_recovers_to_queued_with_retry() {
        let dir = tempfile::tempdir().unwrap();
        let store = StoreHandle::open(dir.path().join("state").join("runtime.sqlite3")).unwrap();
        store
            .enqueue_inbox(input(Uuid::new_v4(), "recover"))
            .await
            .unwrap();
        store
            .claim_inbox_batch(50, "crashed".into(), Utc::now())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(store.recover_in_turn(Utc::now()).await.unwrap().requeued, 1);
        let depths = store.queue_depths().await.unwrap();
        assert_eq!((depths.queued, depths.in_turn), (1, 0));
    }

    #[tokio::test]
    async fn capacity_keeps_prior_rows_and_dead_letters_new_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = StoreHandle::open(dir.path().join("state").join("runtime.sqlite3")).unwrap();
        let channel = Uuid::new_v4();
        for index in 0..MAX_PENDING_PER_CHANNEL {
            assert_eq!(
                store
                    .enqueue_inbox(input(channel, &index.to_string()))
                    .await
                    .unwrap(),
                EnqueueOutcome::Enqueued
            );
        }
        assert_eq!(
            store
                .enqueue_inbox(input(channel, "overflow"))
                .await
                .unwrap(),
            EnqueueOutcome::CapacityRejected
        );
        let depths = store.queue_depths().await.unwrap();
        assert_eq!(
            (
                depths.queued,
                depths.dead_letter,
                depths.capacity_rejections
            ),
            (500, 1, 1)
        );
    }
    #[test]
    fn projector_uses_durable_precedence_for_every_work_state() {
        let reading = assignment(AssignmentState::Reading);
        let working = assignment(AssignmentState::Working);
        let waiting = assignment(AssignmentState::Waiting);
        let blocked = assignment(AssignmentState::Blocked);
        let approval = assignment(AssignmentState::NeedsApproval);
        let recovering = assignment(AssignmentState::Recovering);
        let running = job(JobState::Running);
        assert_eq!(
            project_work_state(false, true, true, true, Some(&blocked), Some(&running)),
            WorkState::Offline
        );
        assert_eq!(
            project_work_state(true, true, true, true, Some(&blocked), Some(&running)),
            WorkState::Recovering
        );
        assert_eq!(
            project_work_state(true, false, true, true, Some(&blocked), Some(&running)),
            WorkState::NeedsApproval
        );
        assert_eq!(
            project_work_state(true, false, false, true, Some(&blocked), Some(&running)),
            WorkState::Blocked
        );
        assert_eq!(
            project_work_state(true, false, false, false, Some(&approval), None),
            WorkState::NeedsApproval
        );
        assert_eq!(
            project_work_state(true, false, false, false, Some(&recovering), None),
            WorkState::Recovering
        );
        assert_eq!(
            project_work_state(true, false, false, false, Some(&working), None),
            WorkState::Working
        );
        assert_eq!(
            project_work_state(true, false, false, false, None, Some(&running)),
            WorkState::Working
        );
        assert_eq!(
            project_work_state(true, false, false, false, Some(&waiting), None),
            WorkState::Waiting
        );
        assert_eq!(
            project_work_state(true, false, false, false, Some(&reading), None),
            WorkState::Reading
        );
        assert_eq!(
            project_work_state(true, false, false, true, None, None),
            WorkState::Reading
        );
        assert_eq!(
            project_work_state(true, false, false, false, None, None),
            WorkState::Idle
        );
        for terminal in [
            AssignmentState::Completed,
            AssignmentState::Failed,
            AssignmentState::Cancelled,
        ] {
            let terminal_assignment = assignment(terminal);
            assert_eq!(
                project_work_state(true, false, false, false, Some(&terminal_assignment), None),
                WorkState::Idle
            );
        }
    }

    #[tokio::test]
    async fn assignment_transitions_require_concrete_state_details() {
        let dir = tempfile::tempdir().unwrap();
        let store = StoreHandle::open(dir.path().join("state").join("runtime.sqlite3")).unwrap();
        let claimed = store
            .claim_assignment(
                Uuid::new_v4(),
                Some("a".repeat(64)),
                "task".into(),
                None,
                Utc::now(),
            )
            .await
            .unwrap();
        let mut invalid_waiting = state_request(AssignmentState::Waiting);
        invalid_waiting.reason = Some("  ".into());
        assert!(matches!(
            store
                .set_assignment_state(&claimed.assignment_id, invalid_waiting, Utc::now())
                .await,
            Err(StoreError::InvalidAssignment(_))
        ));
        let mut invalid_blocked = state_request(AssignmentState::Blocked);
        invalid_blocked.blocker = None;
        assert!(store
            .set_assignment_state(&claimed.assignment_id, invalid_blocked, Utc::now())
            .await
            .is_err());
        let mut invalid_approval = state_request(AssignmentState::NeedsApproval);
        invalid_approval.approval_gate_id = None;
        assert!(store
            .set_assignment_state(&claimed.assignment_id, invalid_approval, Utc::now())
            .await
            .is_err());
        for state in [
            AssignmentState::Working,
            AssignmentState::Waiting,
            AssignmentState::Blocked,
            AssignmentState::NeedsApproval,
            AssignmentState::Recovering,
            AssignmentState::Reading,
        ] {
            assert_eq!(
                store
                    .set_assignment_state(&claimed.assignment_id, state_request(state), Utc::now())
                    .await
                    .unwrap()
                    .state,
                state
            );
        }
    }

    #[tokio::test]
    async fn unrelated_source_cannot_replace_assignment_or_consume_inbox() {
        let dir = tempfile::tempdir().unwrap();
        let store = StoreHandle::open(dir.path().join("state").join("runtime.sqlite3")).unwrap();
        let channel = Uuid::new_v4();
        let first = store
            .claim_assignment(
                channel,
                Some("a".repeat(64)),
                "first".into(),
                None,
                Utc::now(),
            )
            .await
            .unwrap();
        store
            .enqueue_inbox(input(Uuid::new_v4(), "unrelated"))
            .await
            .unwrap();
        let second = store
            .claim_assignment(
                Uuid::new_v4(),
                Some("b".repeat(64)),
                "second".into(),
                None,
                Utc::now(),
            )
            .await
            .unwrap();
        assert_eq!(second.assignment_id, first.assignment_id);
        assert_eq!(second.source_event_id, first.source_event_id);
        assert_eq!(second.summary, "first");
        assert_eq!(store.queue_depths().await.unwrap().queued, 1);
    }

    #[tokio::test]
    async fn linked_live_and_failed_jobs_cannot_complete_and_terminal_cannot_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let store = StoreHandle::open(dir.path().join("state").join("runtime.sqlite3")).unwrap();
        let source = "a".repeat(64);
        let assignment = store
            .claim_assignment(
                Uuid::new_v4(),
                Some(source.clone()),
                "task".into(),
                None,
                Utc::now(),
            )
            .await
            .unwrap();
        let job_id = create_requested_job(&store, assignment.channel_id, source).await;
        store
            .link_assignment_job(&assignment.assignment_id, job_id, Utc::now())
            .await
            .unwrap();
        assert!(matches!(
            store
                .complete_assignment(&assignment.assignment_id, None, Utc::now())
                .await,
            Err(StoreError::AssignmentCompletionUnverified)
        ));
        let terminal_event_id = format!("terminal-{job_id}");
        store
            .transition_job(
                JobTransition {
                    job_id,
                    attempt: 1,
                    next_state: JobState::Failed,
                    runner: None,
                    progress_seq: None,
                    exit_code: Some(1),
                    result_json: None,
                    error_code: Some("failed".into()),
                    terminal_event_id: Some(terminal_event_id.clone()),
                    publication_state: Some(PublicationState::Pending),
                    publication_error: None,
                    occurred_at: Utc::now(),
                },
                Some(OutboxEvent {
                    event_id: terminal_event_id,
                    job_id: Some(job_id),
                    channel_id: assignment.channel_id,
                    ordering_key: format!("job:{job_id}"),
                    kind: 43_006,
                    seq: None,
                    is_terminal: true,
                    event_json: "{}".into(),
                    created_at: Utc::now(),
                }),
            )
            .await
            .unwrap();
        assert!(matches!(
            store
                .complete_assignment(&assignment.assignment_id, None, Utc::now())
                .await,
            Err(StoreError::AssignmentCompletionUnverified)
        ));
        let completed = store
            .complete_assignment(
                &assignment.assignment_id,
                Some("verified delivery event".into()),
                Utc::now(),
            )
            .await
            .unwrap();
        assert_eq!(completed.state, AssignmentState::Completed);
        assert!(matches!(
            store
                .set_assignment_state(
                    &assignment.assignment_id,
                    state_request(AssignmentState::Working),
                    Utc::now()
                )
                .await,
            Err(StoreError::TerminalAssignment { .. })
        ));
        assert!(store.active_assignment().await.unwrap().is_none());
        assert_eq!(
            store
                .assignment_snapshot()
                .await
                .unwrap()
                .terminal_assignment
                .map(|value| value.assignment_id),
            Some(assignment.assignment_id.clone())
        );
        assert!(store.clear_terminal_assignment().await.unwrap());
        assert!(store
            .assignment_snapshot()
            .await
            .unwrap()
            .terminal_assignment
            .is_none());
    }

    #[tokio::test]
    async fn failed_and_cancelled_are_terminal_and_allow_only_a_new_assignment() {
        let dir = tempfile::tempdir().unwrap();
        let store = StoreHandle::open(dir.path().join("state").join("runtime.sqlite3")).unwrap();
        let failed = store
            .claim_assignment(
                Uuid::new_v4(),
                Some("a".repeat(64)),
                "first".into(),
                None,
                Utc::now(),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .set_assignment_state(
                    &failed.assignment_id,
                    AssignmentSetStateRequest {
                        reason: Some("delivery failed".into()),
                        ..state_request(AssignmentState::Failed)
                    },
                    Utc::now(),
                )
                .await
                .unwrap()
                .state,
            AssignmentState::Failed
        );
        assert!(matches!(
            store
                .set_assignment_state(
                    &failed.assignment_id,
                    state_request(AssignmentState::Reading),
                    Utc::now()
                )
                .await,
            Err(StoreError::TerminalAssignment { .. })
        ));
        let cancelled = store
            .claim_assignment(
                Uuid::new_v4(),
                Some("b".repeat(64)),
                "second".into(),
                None,
                Utc::now(),
            )
            .await
            .unwrap();
        assert_ne!(cancelled.assignment_id, failed.assignment_id);
        assert_eq!(
            store
                .set_assignment_state(
                    &cancelled.assignment_id,
                    AssignmentSetStateRequest {
                        reason: Some("owner cancelled".into()),
                        ..state_request(AssignmentState::Cancelled)
                    },
                    Utc::now(),
                )
                .await
                .unwrap()
                .state,
            AssignmentState::Cancelled
        );
    }

    #[tokio::test]
    async fn recovery_state_exits_without_a_time_or_pid_heuristic() {
        let dir = tempfile::tempdir().unwrap();
        let store = StoreHandle::open(dir.path().join("state").join("runtime.sqlite3")).unwrap();
        store
            .set_recovery_state(true, Some("/secret/path".into()))
            .await
            .unwrap();
        let recovering = store.assignment_snapshot().await.unwrap();
        assert!(recovering.recovering);
        assert_eq!(
            recovering.recovery_reason.as_deref(),
            Some("runtime_reconciliation")
        );
        store.set_recovery_state(false, None).await.unwrap();
        let recovered = store.assignment_snapshot().await.unwrap();
        assert!(!recovered.recovering);
        assert!(recovered.recovery_reason.is_none());
    }
}
