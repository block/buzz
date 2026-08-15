//! Strict, versioned types for the managed-runtime loopback protocol.

use std::{fmt, net::SocketAddr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Control protocol version: 1, selected by loopback frame validation.
pub const CONTROL_PROTOCOL_VERSION: u16 = 1;
/// Runtime receipt schema version: 2, selected by receipt validation.
pub const RUNTIME_RECEIPT_SCHEMA_VERSION: u8 = 2;
/// Legacy runtime receipt schema version: 1, used for phase-0 handoff compatibility.
pub const LEGACY_RUNTIME_RECEIPT_SCHEMA_VERSION: u8 = 1;
/// Maximum encoded control request: 64 KiB, bounding one loopback frame.
pub const MAX_CONTROL_REQUEST_BYTES: usize = 64 * 1024;
/// Maximum encoded control response: 1 MiB, bounding one loopback frame.
pub const MAX_CONTROL_RESPONSE_BYTES: usize = 1024 * 1024;
/// Control deadline: 5 seconds for connect, read, and write operations.
pub const CONTROL_DEADLINE_SECS: u64 = 5;
/// Supported job driver: `lh`, limiting execution to the managed harness.
pub const SUPPORTED_JOB_DRIVER: &str = "lh";
/// Maximum driver length: 64 UTF-8 bytes, bounding request metadata.
pub const MAX_DRIVER_BYTES: usize = 64;
/// Maximum argv entries: 256, bounding runner input cardinality.
pub const MAX_ARGV_ELEMENTS: usize = 256;
/// Maximum argv-entry length: 8 KiB, bounding one runner argument.
pub const MAX_ARG_BYTES: usize = 8 * 1024;
/// Maximum serialized argv length: 64 KiB, bounding encoded runner input.
pub const MAX_JOB_ARGV_JSON_BYTES: usize = 64 * 1024;
/// Maximum cwd length: 4 KiB, bounding workspace metadata.
pub const MAX_CWD_BYTES: usize = 4 * 1024;
/// Maximum summary or cancellation-reason length: 4 KiB, bounding user-facing text.
pub const MAX_SUMMARY_BYTES: usize = 4 * 1024;
/// Maximum artifact references: 32, bounding response metadata.
pub const MAX_ARTIFACTS: usize = 32;
/// Maximum artifact-name length: 256 UTF-8 bytes, bounding reference metadata.
pub const MAX_ARTIFACT_NAME_BYTES: usize = 256;
/// Maximum artifact-URI length: 2 KiB, bounding reference metadata.
pub const MAX_ARTIFACT_URI_BYTES: usize = 2 * 1024;
/// Default local log-tail length: 100 lines when the caller omits a value.
pub const DEFAULT_LOG_TAIL_LINES: u16 = 100;
/// Maximum local log-tail length: 1,000 lines returned to a caller.
pub const MAX_LOG_TAIL_LINES: u16 = 1_000;
/// Applies default and maximum bounds to a requested local log tail.
pub fn bounded_log_tail_lines(requested: Option<u16>) -> u16 {
    requested
        .unwrap_or(DEFAULT_LOG_TAIL_LINES)
        .min(MAX_LOG_TAIL_LINES)
}
/// Maximum assignment identifier, summary, or state-detail length: 4 KiB.
pub const MAX_ASSIGNMENT_TEXT_BYTES: usize = 4 * 1024;

/// Stable identifier for a managed runtime pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedAgentRuntimeKey {
    /// Agent public key as 64 lowercase hexadecimal characters.
    pub pubkey: String,
    /// Relay URL that scopes the runtime.
    pub relay_url: String,
}

/// Generation-scoped capability token whose debug representation is always redacted.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretToken(String);

impl SecretToken {
    /// Creates a token from an already generated secret value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    /// Generates a random 256-bit hexadecimal token.
    pub fn generate() -> Self {
        Self(hex::encode(rand::random::<[u8; 32]>()))
    }
    /// Returns the secret only for protocol authentication.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretToken([REDACTED])")
    }
}

/// Owner-only Phase-0 receipt proving that a schema-v1 harness acquired the
/// pair-scoped OS lock before announcing its PID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyRuntimeReceipt {
    /// Legacy receipt schema version: 1.
    pub schema_version: u8,
    /// Agent runtime-pair identity.
    pub key: ManagedAgentRuntimeKey,
    /// Runtime process id at start time.
    pub pid: u32,
    /// Process start marker binding the PID to one boot.
    pub process_start_marker: String,
    /// Desktop instance identity that acquired the lock.
    pub desktop_instance_id: String,
    /// Receipt creation timestamp.
    pub started_at: DateTime<Utc>,
    /// Lock file-protocol version: 1.
    pub lock_protocol_version: u8,
    /// SHA-256 over the lock path, lowercase hex.
    pub lock_path_hash: String,
}

impl LegacyRuntimeReceipt {
    /// Validates the immutable proof fields before Desktop uses the receipt to
    /// decide whether schema-v2 cutover is safe.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.schema_version != LEGACY_RUNTIME_RECEIPT_SCHEMA_VERSION
            || self.pid == 0
            || self.process_start_marker.is_empty()
            || self.desktop_instance_id.is_empty()
            || self.lock_protocol_version != 1
            || !is_lower_hex(&self.key.pubkey, 64)
            || self.key.relay_url.is_empty()
            || !is_lower_hex(&self.lock_path_hash, 64)
        {
            return Err(ProtocolError::InvalidReceipt);
        }
        Ok(())
    }
}

/// Owner-only schema-v2 receipt used to authenticate and adopt a runtime.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeReceipt {
    /// Runtime receipt schema version: 2.
    pub schema_version: u8,
    /// Agent runtime-pair identity.
    pub key: ManagedAgentRuntimeKey,
    /// Owning runtime identifier.
    pub runtime_id: String,
    /// Runtime process id at start time.
    pub pid: u32,
    /// Process start marker binding the PID to one boot.
    pub process_start_marker: String,
    /// Runtime generation this receipt proves.
    pub generation: Uuid,
    /// Loopback address of the control server.
    pub control_addr: SocketAddr,
    /// Secret bearer token for controller capability.
    pub controller_token: SecretToken,
    /// Secret bearer token for model capability.
    pub model_token: SecretToken,
    /// Receipt creation timestamp.
    pub started_at: DateTime<Utc>,
    /// Control protocol version the runtime speaks.
    pub protocol_version: u16,
    /// Lock file-protocol version: 1.
    pub lock_protocol_version: u8,
    /// SHA-256 over the lock path, lowercase hex.
    pub lock_path_hash: String,
    /// Whether the runtime finished bringing up its control server.
    pub ready: bool,
}

impl fmt::Debug for RuntimeReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeReceipt")
            .field("schema_version", &self.schema_version)
            .field("key", &self.key)
            .field("runtime_id", &self.runtime_id)
            .field("pid", &self.pid)
            .field("process_start_marker", &self.process_start_marker)
            .field("generation", &self.generation)
            .field("control_addr", &self.control_addr)
            .field("controller_token", &self.controller_token)
            .field("model_token", &self.model_token)
            .field("started_at", &self.started_at)
            .field("protocol_version", &self.protocol_version)
            .field("lock_protocol_version", &self.lock_protocol_version)
            .field("lock_path_hash", &self.lock_path_hash)
            .field("ready", &self.ready)
            .finish()
    }
}

impl RuntimeReceipt {
    /// Validates immutable receipt fields before a client trusts its endpoint.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.schema_version != RUNTIME_RECEIPT_SCHEMA_VERSION
            || self.protocol_version != CONTROL_PROTOCOL_VERSION
            || !self.ready
            || self.pid == 0
            || self.process_start_marker.is_empty()
            || self.runtime_id.is_empty()
            || self.generation.is_nil()
            || !self.control_addr.ip().is_loopback()
            || self.control_addr.port() == 0
            || self.lock_protocol_version != 1
            || !is_lower_hex(&self.key.pubkey, 64)
            || self.key.relay_url.is_empty()
            || !is_lower_hex(&self.lock_path_hash, 64)
            || !is_lower_hex(self.controller_token.expose_secret(), 64)
            || !is_lower_hex(self.model_token.expose_secret(), 64)
            || self.controller_token == self.model_token
        {
            return Err(ProtocolError::InvalidReceipt);
        }
        Ok(())
    }
}

/// Capability selected by a same-host caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Full-trust capability: may run every control operation.
    Controller,
    /// Restricted capability: may run model-allowed operations only.
    Model,
}
/// Capability authenticated by the control server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizedCapability {
    /// Controller capability verified by bearer token.
    Controller,
    /// Model capability verified by bearer token.
    Model,
}
/// A durable job identifier.
pub type JobId = Uuid;
/// Durable assignment lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentState {
    /// Reading queued input before work starts.
    Reading,
    /// Executing the assignment.
    Working,
    /// Waiting on an external event or timer.
    Waiting,
    /// Paused pending explicit user approval.
    NeedsApproval,
    /// Blocked by a recorded blocker.
    Blocked,
    /// Recovering after a crash or restart.
    Recovering,
    /// Finished successfully; terminal.
    Completed,
    /// Finished unsuccessfully; terminal.
    Failed,
    /// Cancelled by the operator; terminal.
    Cancelled,
}

impl AssignmentState {
    /// Returns whether this assignment can never be reopened.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// User-visible work state projected only from durable runtime facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkState {
    /// No assignment active.
    Idle,
    /// Reading queued input before work starts.
    Reading,
    /// Executing the active assignment.
    Working,
    /// Waiting on an external event or timer.
    Waiting,
    /// Paused pending explicit user approval.
    NeedsApproval,
    /// Blocked by a recorded blocker.
    Blocked,
    /// Recovering after a crash or restart.
    Recovering,
    /// Runtime unreachable; projected from staleness.
    Offline,
}

/// Strict durable assignment record. Terminal rows remain as history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssignmentRecord {
    /// Stable assignment identifier.
    pub assignment_id: String,
    /// Inbox event that created the assignment, if retained.
    pub source_event_id: Option<String>,
    /// Channel the assignment serves.
    pub channel_id: Uuid,
    /// Current lifecycle state.
    pub state: AssignmentState,
    /// Bounded human-readable summary.
    pub summary: String,
    /// Active job identifier, if any.
    pub active_job_id: Option<JobId>,
    /// Private session identifier; never relayed.
    pub session_id: Option<String>,
    /// Event id of the latest reply, if published.
    pub reply_event_id: Option<String>,
    /// Last accepted progress timestamp.
    pub last_progress_at: DateTime<Utc>,
    /// State-transition reason; required by some states.
    pub reason: Option<String>,
    /// Human-readable blocker; required while blocked.
    pub blocker: Option<String>,
    /// Approval gate identifier; required while awaiting approval.
    pub approval_gate_id: Option<String>,
    /// Delivery evidence string; required to complete.
    pub delivery_evidence: Option<String>,
    /// Last durable write timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Model-capability request to update the current assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssignmentSetStateRequest {
    /// Requested next lifecycle state.
    pub state: AssignmentState,
    /// Replacement summary.
    pub summary: Option<String>,
    /// State-transition reason.
    pub reason: Option<String>,
    /// Blocker description.
    pub blocker: Option<String>,
    /// Approval gate identifier.
    pub approval_gate_id: Option<String>,
    /// Delivery evidence string.
    pub delivery_evidence: Option<String>,
    /// Event id this update replies to, if any.
    pub reply_event_id: Option<String>,
}

impl AssignmentSetStateRequest {
    /// Validates bounded state-specific details before handler execution.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        for (name, value) in [
            ("summary", self.summary.as_deref()),
            ("reason", self.reason.as_deref()),
            ("blocker", self.blocker.as_deref()),
            ("approval gate", self.approval_gate_id.as_deref()),
            ("delivery evidence", self.delivery_evidence.as_deref()),
            ("reply event", self.reply_event_id.as_deref()),
        ] {
            if value.is_some_and(|value| value.len() > MAX_ASSIGNMENT_TEXT_BYTES) {
                return Err(ProtocolError::BoundExceeded(name));
            }
        }
        if self.blocker.is_some() && self.state != AssignmentState::Blocked {
            return Err(ProtocolError::InvalidAssignment(
                "blocker is valid only for blocked",
            ));
        }
        if self.approval_gate_id.is_some() && self.state != AssignmentState::NeedsApproval {
            return Err(ProtocolError::InvalidAssignment(
                "approval gate is valid only for needs approval",
            ));
        }
        if self.delivery_evidence.is_some() && self.state != AssignmentState::Completed {
            return Err(ProtocolError::InvalidAssignment(
                "delivery evidence is valid only for completed",
            ));
        }
        if self.reason.is_some()
            && !matches!(
                self.state,
                AssignmentState::Waiting | AssignmentState::Failed | AssignmentState::Cancelled
            )
        {
            return Err(ProtocolError::InvalidAssignment(
                "reason is not valid for this state",
            ));
        }
        match self.state {
            AssignmentState::Waiting if !nonempty(self.reason.as_deref()) => {
                Err(ProtocolError::InvalidAssignment("waiting requires reason"))
            }
            AssignmentState::Blocked if !nonempty(self.blocker.as_deref()) => {
                Err(ProtocolError::InvalidAssignment("blocked requires blocker"))
            }
            AssignmentState::NeedsApproval if !nonempty(self.approval_gate_id.as_deref()) => Err(
                ProtocolError::InvalidAssignment("needs approval requires gate id"),
            ),
            _ => Ok(()),
        }
    }
}

fn nonempty(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

/// Strict local job-start request. It cannot carry a shell string, environment, or stdin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobStartRequest {
    /// Channel the job serves.
    pub channel_id: Uuid,
    /// Inbox event that requested the job, if retained.
    pub source_event_id: Option<String>,
    /// Job driver name; only `lh` is supported.
    pub driver: String,
    /// Bounded argv vector; no shell string or stdin.
    pub argv: Vec<String>,
    /// Working directory for the runner.
    pub cwd: String,
    /// Bounded human-readable summary.
    pub summary: String,
}

impl JobStartRequest {
    /// Applies all request and serialized-argv bounds before persistence.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.source_event_id.as_deref().is_some_and(|id| {
            id.len() != 64
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }) {
            return Err(ProtocolError::BoundExceeded("source event id"));
        }
        if self.driver.len() > MAX_DRIVER_BYTES {
            return Err(ProtocolError::BoundExceeded("driver"));
        }
        if self.driver != SUPPORTED_JOB_DRIVER {
            return Err(ProtocolError::UnsupportedDriver);
        }
        if self.argv.len() > MAX_ARGV_ELEMENTS {
            return Err(ProtocolError::BoundExceeded("argv"));
        }
        if self.argv.iter().any(|arg| arg.len() > MAX_ARG_BYTES) {
            return Err(ProtocolError::BoundExceeded("argv element"));
        }
        let argv_json = serde_json::to_vec(&self.argv).map_err(ProtocolError::Serialization)?;
        if argv_json.len() > MAX_JOB_ARGV_JSON_BYTES {
            return Err(ProtocolError::BoundExceeded("argv json"));
        }
        if self.cwd.is_empty() || self.cwd.len() > MAX_CWD_BYTES {
            return Err(ProtocolError::BoundExceeded("cwd"));
        }
        if self.summary.is_empty() || self.summary.len() > MAX_SUMMARY_BYTES {
            return Err(ProtocolError::BoundExceeded("summary"));
        }
        Ok(())
    }
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Job lifecycle state stored by the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Accepted but not yet started.
    Requested,
    /// Runner spawned, awaiting start confirmation.
    Accepted,
    /// Runner is executing.
    Running,
    /// Cancel requested, awaiting termination.
    Cancelling,
    /// Runner exited zero; terminal.
    Succeeded,
    /// Runner exited nonzero; terminal.
    Failed,
    /// Cancelled before terminal exit; terminal.
    Cancelled,
    /// Runner vanished without a receipt; terminal.
    Lost,
}
impl JobState {
    /// Returns whether this state is terminal and immutable.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Lost
        )
    }
}

/// Relay publication state for a durable job projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationState {
    /// Projection not yet queued for publication.
    NotStarted,
    /// Queued for relay publication.
    Pending,
    /// Published to the relay.
    Published,
    /// Publication attempts failed; terminal.
    Failed,
}

/// Stable local job status returned by control operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobStatus {
    /// Durable job identifier.
    pub job_id: JobId,
    /// Event id of the start request, if retained.
    pub request_event_id: Option<String>,
    /// Inbox event that requested the job, if retained.
    pub source_event_id: Option<String>,
    /// Channel the job serves.
    pub channel_id: Uuid,
    /// Current lifecycle state.
    pub state: JobState,
    /// Attempt number; positive, one spec per attempt.
    pub attempt: u32,
    /// Monotonic progress sequence for this job.
    pub progress_seq: u64,
    /// Bounded human-readable summary.
    pub summary: String,
    /// First runner start timestamp, if started.
    pub started_at: Option<DateTime<Utc>>,
    /// Terminal timestamp, if finished.
    pub finished_at: Option<DateTime<Utc>>,
    /// Relay publication state of this projection.
    pub exit_code: Option<i32>,
    /// Runner process id, if a runner is recorded.
    pub error_code: Option<String>,
    /// Runner process start marker, if recorded.
    pub publication_state: PublicationState,
    /// Relay publication state of the latest projection.
    pub runner_pid: Option<u32>,
    /// Runner process id at start time, if recorded.
    pub runner_start_marker: Option<String>,
}

/// Filters accepted by `jobs.list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobListFilter {
    /// Restrict results to one channel.
    pub channel_id: Option<Uuid>,
    /// Restrict results to one lifecycle state.
    pub state: Option<JobState>,
}

/// Bounded same-host raw log tail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobLogs {
    /// Job the lines belong to.
    pub job_id: JobId,
    /// Whether the tail was served without relay fan-out.
    pub local_only: bool,
    /// Bounded log lines, oldest first.
    pub lines: Vec<String>,
}

/// Safe runner-receipt health projected without PIDs or local paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerReceiptHealth {
    /// Runner spawned and reporting ready.
    Ready,
    /// Receipt records a terminal state.
    Terminal,
    /// No receipt found for the attempt.
    Missing,
    /// Receipt failed schema validation.
    Invalid,
    /// Receipt identity does not match the job.
    IdentityMismatch,
}

/// Receipt health for one active job attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobRunnerReceiptHealth {
    /// Job the health describes.
    pub job_id: JobId,
    /// Attempt number of the inspected receipt.
    pub attempt: u32,
    /// Receipt health verdict.
    pub health: RunnerReceiptHealth,
}

/// Owner-safe operational runtime diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeDiagnostics {
    /// Store schema version the runtime manages.
    pub store_schema_version: u32,
    /// Per-attempt receipt health for active jobs.
    pub runner_receipts: Vec<JobRunnerReceiptHealth>,
    /// Last relay progress publication timestamp, if any.
    pub last_relay_progress_published_at: Option<DateTime<Utc>>,
}

/// Redacted assignment facts safe for runtime diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssignmentStatusSnapshot {
    /// Stable assignment identifier.
    pub assignment_id: String,
    /// Inbox event that created the assignment, if retained.
    pub source_event_id: Option<String>,
    /// Channel the assignment serves.
    pub channel_id: Uuid,
    /// Current lifecycle state.
    pub state: AssignmentState,
    /// Bounded human-readable summary.
    pub summary: String,
    /// Active job identifier, if any.
    pub active_job_id: Option<JobId>,
    /// Last accepted progress timestamp.
    pub last_progress_at: DateTime<Utc>,
    /// Whether a blocker is recorded.
    pub has_blocker: bool,
}

impl From<&AssignmentRecord> for AssignmentStatusSnapshot {
    fn from(record: &AssignmentRecord) -> Self {
        Self {
            assignment_id: record.assignment_id.clone(),
            source_event_id: record.source_event_id.clone(),
            channel_id: record.channel_id,
            state: record.state,
            summary: record.summary.clone(),
            active_job_id: record.active_job_id,
            last_progress_at: record.last_progress_at,
            has_blocker: record.blocker.is_some(),
        }
    }
}

/// Runtime status projection returned over local control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeStatusSnapshot {
    /// Owning runtime identifier.
    pub runtime_id: String,
    /// Runtime generation of this snapshot.
    pub generation: Uuid,
    /// User-visible work state.
    pub work_state: WorkState,
    /// Whether the runtime is in recovery.
    pub recovering: bool,
    /// Machine-readable recovery cause, if recovering.
    pub recovery_reason: Option<String>,
    /// Inbox depth of queued events.
    pub queued_inbox: u64,
    /// Inbox depth of events in turn.
    pub in_turn_inbox: u64,
    /// Inbox depth of dead-lettered events.
    pub dead_letter_inbox: u64,
    /// Count of capacity admissions rejected.
    pub capacity_rejections: u64,
    /// Active assignment projection, if any.
    pub active_assignment: Option<AssignmentStatusSnapshot>,
    /// Single-job compatibility view of `active_jobs`.
    pub active_job: Option<JobId>,
    /// Jobs currently owned by the runtime.
    pub active_jobs: Vec<JobId>,
    /// Owner-safe diagnostic block.
    pub diagnostics: RuntimeDiagnostics,
}

/// Compatibility name retained for existing controller call sites.
pub type RuntimeStatus = RuntimeStatusSnapshot;

/// Generation and capability proof returned by authenticated hello.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelloResponse {
    /// Owning runtime identifier.
    pub runtime_id: String,
    /// Runtime generation this proof covers.
    pub generation: Uuid,
    /// Capability granted by the server.
    pub capability: String,
}

/// Every control request carries protocol version, generation, and one capability token.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlRequest {
    /// Control protocol version: 1.
    pub protocol_version: u16,
    /// Runtime generation being addressed.
    pub generation: Uuid,
    /// Bearer token for the requested capability.
    pub control_token: SecretToken,
    /// Operation to execute.
    pub operation: ControlOperation,
}
impl fmt::Debug for ControlRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlRequest")
            .field("protocol_version", &self.protocol_version)
            .field("generation", &self.generation)
            .field("control_token", &self.control_token)
            .field("operation", &self.operation)
            .finish()
    }
}

/// Version-one control operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "op",
    content = "params",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ControlOperation {
    /// Exchange tokens for a capability proof.
    Hello,
    /// Fetch the runtime status snapshot.
    Status,
    /// List jobs matching a filter.
    JobsList(JobListFilter),
    /// Start a local job from a strict request.
    JobsStart(JobStartRequest),
    /// Fetch one job status by id.
    JobsStatus {
        /// Job to inspect.
        job_id: JobId,
    },
    /// Request cancellation of one job.
    JobsCancel {
        /// Job to cancel.
        job_id: JobId,
    },
    /// Fetch a bounded local log tail.
    JobsLogs {
        /// Job whose logs are read.
        job_id: JobId,
        /// Requested tail length; defaults and caps apply.
        tail_lines: Option<u16>,
    },
    /// Update the current assignment state.
    AssignmentSetState {
        /// Assignment to update.
        assignment_id: String,
        /// Strict state-update request.
        request: AssignmentSetStateRequest,
    },
    /// Reconcile projected state from durable facts.
    Reconcile,
    /// Stop the runtime after current work drains.
    Shutdown,
}
impl ControlOperation {
    /// Returns whether the restricted model capability may invoke this operation.
    pub fn model_allowed(&self) -> bool {
        matches!(
            self,
            Self::Hello
                | Self::Status
                | Self::JobsList(_)
                | Self::JobsStart(_)
                | Self::JobsStatus { .. }
                | Self::JobsLogs { .. }
                | Self::AssignmentSetState { .. }
        )
    }
}

/// Successful response payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum ControlPayload {
    /// Capability proof from a successful hello.
    Hello(HelloResponse),
    /// Runtime status snapshot.
    Status(RuntimeStatusSnapshot),
    /// Jobs matching a `jobs.list` filter.
    Jobs(Vec<JobStatus>),
    /// One job status.
    Job(JobStatus),
    /// Bounded local log tail.
    Logs(JobLogs),
    /// Updated assignment record.
    Assignment(AssignmentRecord),
    /// Success with no payload.
    Ack,
}

/// Stable error returned to a local client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlError {
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable error detail.
    pub message: String,
}
impl ControlError {
    /// Constructs the deliberately generic authentication/authorization error.
    pub fn unauthorized() -> Self {
        Self {
            code: "unauthorized".into(),
            message: "unauthorized".into(),
        }
    }
    /// Constructs a handler error without attaching secret data.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// One bounded response frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlResponse {
    /// Control protocol version: 1.
    pub protocol_version: u16,
    /// Success payload, present unless failed.
    pub result: Option<ControlPayload>,
    /// Error payload, present unless successful.
    pub error: Option<ControlError>,
}
impl ControlResponse {
    /// Builds a successful response frame with the current protocol version.
    pub fn success(payload: ControlPayload) -> Self {
        Self {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            result: Some(payload),
            error: None,
        }
    }
    /// Builds a failure response frame with the current protocol version.
    pub fn failure(error: ControlError) -> Self {
        Self {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            result: None,
            error: Some(error),
        }
    }
}

/// Protocol validation failure before handler execution.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("invalid runtime receipt")]
    /// Runtime receipt failed validation.
    InvalidReceipt,
    #[error("{0} exceeds its protocol bound")]
    /// A named field exceeded its protocol bound.
    BoundExceeded(&'static str),
    #[error("unsupported driver")]
    /// Requested driver is not supported.
    UnsupportedDriver,
    #[error("invalid assignment: {0}")]
    /// Assignment update failed state-specific validation.
    InvalidAssignment(&'static str),
    #[error("protocol serialization failed: {0}")]
    /// Control payload failed serialization.
    Serialization(serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assignment(state: AssignmentState) -> AssignmentRecord {
        let now = Utc::now();
        AssignmentRecord {
            assignment_id: Uuid::new_v4().to_string(),
            source_event_id: Some("a".repeat(64)),
            channel_id: Uuid::new_v4(),
            state,
            summary: "owned task".into(),
            active_job_id: None,
            session_id: Some("private session".into()),
            reply_event_id: None,
            last_progress_at: now,
            reason: None,
            blocker: Some("secret-token /private/path".into()),
            approval_gate_id: None,
            delivery_evidence: None,
            updated_at: now,
        }
    }

    #[test]
    fn assignment_request_is_strict_and_state_specific() {
        let request = AssignmentSetStateRequest {
            state: AssignmentState::Waiting,
            summary: None,
            reason: None,
            blocker: None,
            approval_gate_id: None,
            delivery_evidence: None,
            reply_event_id: None,
        };
        assert!(matches!(
            request.validate(),
            Err(ProtocolError::InvalidAssignment("waiting requires reason"))
        ));
        let json = serde_json::json!({
            "state": "working",
            "summary": null,
            "reason": null,
            "blocker": null,
            "approvalGateId": null,
            "deliveryEvidence": null,
            "replyEventId": null,
            "unexpected": true
        });
        assert!(serde_json::from_value::<AssignmentSetStateRequest>(json).is_err());
    }

    #[test]
    fn runtime_status_excludes_assignment_secrets_and_process_facts() {
        let record = assignment(AssignmentState::Blocked);
        let snapshot = RuntimeStatusSnapshot {
            runtime_id: "runtime".into(),
            generation: Uuid::new_v4(),
            work_state: WorkState::Blocked,
            recovering: false,
            recovery_reason: None,
            queued_inbox: 2,
            in_turn_inbox: 1,
            dead_letter_inbox: 0,
            capacity_rejections: 0,
            active_assignment: Some(AssignmentStatusSnapshot::from(&record)),
            active_job: None,
            active_jobs: vec![],
            diagnostics: RuntimeDiagnostics::default(),
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(!json.contains("secret-token"));
        assert!(!json.contains("/private/path"));
        assert!(!json.contains("session"));
        assert!(!json.contains("pid"));
        assert!(!json.contains("controlToken"));
    }

    #[test]
    fn model_capability_cannot_cancel_jobs() {
        assert!(!ControlOperation::JobsCancel {
            job_id: Uuid::new_v4()
        }
        .model_allowed());
    }
}
