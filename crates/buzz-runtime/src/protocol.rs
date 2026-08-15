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
    pub schema_version: u8,
    pub key: ManagedAgentRuntimeKey,
    pub pid: u32,
    pub process_start_marker: String,
    pub desktop_instance_id: String,
    pub started_at: DateTime<Utc>,
    pub lock_protocol_version: u8,
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
    pub schema_version: u8,
    pub key: ManagedAgentRuntimeKey,
    pub runtime_id: String,
    pub pid: u32,
    pub process_start_marker: String,
    pub generation: Uuid,
    pub control_addr: SocketAddr,
    pub controller_token: SecretToken,
    pub model_token: SecretToken,
    pub started_at: DateTime<Utc>,
    pub protocol_version: u16,
    pub lock_protocol_version: u8,
    pub lock_path_hash: String,
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
    Controller,
    Model,
}
/// Capability authenticated by the control server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizedCapability {
    Controller,
    Model,
}
/// A durable job identifier.
pub type JobId = Uuid;
/// Durable assignment lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentState {
    Reading,
    Working,
    Waiting,
    NeedsApproval,
    Blocked,
    Recovering,
    Completed,
    Failed,
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
    Idle,
    Reading,
    Working,
    Waiting,
    NeedsApproval,
    Blocked,
    Recovering,
    Offline,
}

/// Strict durable assignment record. Terminal rows remain as history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssignmentRecord {
    pub assignment_id: String,
    pub source_event_id: Option<String>,
    pub channel_id: Uuid,
    pub state: AssignmentState,
    pub summary: String,
    pub active_job_id: Option<JobId>,
    pub session_id: Option<String>,
    pub reply_event_id: Option<String>,
    pub last_progress_at: DateTime<Utc>,
    pub reason: Option<String>,
    pub blocker: Option<String>,
    pub approval_gate_id: Option<String>,
    pub delivery_evidence: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// Model-capability request to update the current assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssignmentSetStateRequest {
    pub state: AssignmentState,
    pub summary: Option<String>,
    pub reason: Option<String>,
    pub blocker: Option<String>,
    pub approval_gate_id: Option<String>,
    pub delivery_evidence: Option<String>,
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
    pub channel_id: Uuid,
    pub source_event_id: Option<String>,
    pub driver: String,
    pub argv: Vec<String>,
    pub cwd: String,
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
    Requested,
    Accepted,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
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
    NotStarted,
    Pending,
    Published,
    Failed,
}

/// Stable local job status returned by control operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobStatus {
    pub job_id: JobId,
    pub request_event_id: Option<String>,
    pub source_event_id: Option<String>,
    pub channel_id: Uuid,
    pub state: JobState,
    pub attempt: u32,
    pub progress_seq: u64,
    pub summary: String,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub error_code: Option<String>,
    pub publication_state: PublicationState,
    pub runner_pid: Option<u32>,
    pub runner_start_marker: Option<String>,
}

/// Filters accepted by `jobs.list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobListFilter {
    pub channel_id: Option<Uuid>,
    pub state: Option<JobState>,
}

/// Bounded same-host raw log tail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobLogs {
    pub job_id: JobId,
    pub local_only: bool,
    pub lines: Vec<String>,
}

/// Safe runner-receipt health projected without PIDs or local paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerReceiptHealth {
    Ready,
    Terminal,
    Missing,
    Invalid,
    IdentityMismatch,
}

/// Receipt health for one active job attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobRunnerReceiptHealth {
    pub job_id: JobId,
    pub attempt: u32,
    pub health: RunnerReceiptHealth,
}

/// Owner-safe operational runtime diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeDiagnostics {
    pub store_schema_version: u32,
    pub runner_receipts: Vec<JobRunnerReceiptHealth>,
    pub last_relay_progress_published_at: Option<DateTime<Utc>>,
}

/// Redacted assignment facts safe for runtime diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssignmentStatusSnapshot {
    pub assignment_id: String,
    pub source_event_id: Option<String>,
    pub channel_id: Uuid,
    pub state: AssignmentState,
    pub summary: String,
    pub active_job_id: Option<JobId>,
    pub last_progress_at: DateTime<Utc>,
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
    pub runtime_id: String,
    pub generation: Uuid,
    pub work_state: WorkState,
    pub recovering: bool,
    pub recovery_reason: Option<String>,
    pub queued_inbox: u64,
    pub in_turn_inbox: u64,
    pub dead_letter_inbox: u64,
    pub capacity_rejections: u64,
    pub active_assignment: Option<AssignmentStatusSnapshot>,
    pub active_job: Option<JobId>,
    pub active_jobs: Vec<JobId>,
    pub diagnostics: RuntimeDiagnostics,
}

/// Compatibility name retained for existing controller call sites.
pub type RuntimeStatus = RuntimeStatusSnapshot;

/// Generation and capability proof returned by authenticated hello.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelloResponse {
    pub runtime_id: String,
    pub generation: Uuid,
    pub capability: String,
}

/// Every control request carries protocol version, generation, and one capability token.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlRequest {
    pub protocol_version: u16,
    pub generation: Uuid,
    pub control_token: SecretToken,
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
    Hello,
    Status,
    JobsList(JobListFilter),
    JobsStart(JobStartRequest),
    JobsStatus {
        job_id: JobId,
    },
    JobsCancel {
        job_id: JobId,
    },
    JobsLogs {
        job_id: JobId,
        tail_lines: Option<u16>,
    },
    AssignmentSetState {
        assignment_id: String,
        request: AssignmentSetStateRequest,
    },
    Reconcile,
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
    Hello(HelloResponse),
    Status(RuntimeStatusSnapshot),
    Jobs(Vec<JobStatus>),
    Job(JobStatus),
    Logs(JobLogs),
    Assignment(AssignmentRecord),
    Ack,
}

/// Stable error returned to a local client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlError {
    pub code: String,
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
    pub protocol_version: u16,
    pub result: Option<ControlPayload>,
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
    InvalidReceipt,
    #[error("{0} exceeds its protocol bound")]
    BoundExceeded(&'static str),
    #[error("unsupported driver")]
    UnsupportedDriver,
    #[error("invalid assignment: {0}")]
    InvalidAssignment(&'static str),
    #[error("protocol serialization failed: {0}")]
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
