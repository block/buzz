//! Local managed-agent runtime: durable job store, artifact vault, and the
//! same-host control protocol shared by desktop backends and ACP supervisors.
//!
//! The crate is deliberately relay-free: runtimes are per-agent processes and
//! never link against relay code paths.

#![warn(missing_docs)]
#![deny(unsafe_code)]

pub mod artifacts;
pub mod client;
pub mod logs;
pub mod protocol;
pub mod server;
pub mod store;
#[cfg(windows)]
pub mod windows_job;

pub use artifacts::{
    argv_sha256, canonicalize_executable, canonicalize_workspace, canonicalize_workspace_roots,
    current_process_start_marker, ensure_owner_only_runtime_dir, job_attempt_dir,
    process_matches_marker, process_start_marker, read_job_spec, read_legacy_runtime_receipt,
    read_runner_receipt, read_runtime_receipt, runner_receipt_health, write_job_spec,
    write_legacy_runtime_receipt, write_runner_receipt, write_runtime_receipt, ArtifactError,
    JobSpec, RunnerReceipt, RunnerReceiptState, JOB_SPEC_FILE, RUNNER_RECEIPT_FILE,
    RUNNER_RECEIPT_SCHEMA_VERSION,
};
pub use client::{ClientError, RuntimeClient};
pub use logs::{
    tail_rotating_log, RedactingWriter, RotatingLogWriter, MAX_LOG_FILE_BYTES, MAX_LOG_TAIL_BYTES,
    RETAINED_LOG_FILES,
};
pub use protocol::{
    bounded_log_tail_lines, AssignmentRecord, AssignmentSetStateRequest, AssignmentState,
    AssignmentStatusSnapshot, AuthorizedCapability, Capability, ControlError, ControlOperation,
    ControlPayload, ControlRequest, ControlResponse, HelloResponse, JobId, JobListFilter, JobLogs,
    JobRunnerReceiptHealth, JobStartRequest, JobState, JobStatus, LegacyRuntimeReceipt,
    ManagedAgentRuntimeKey, ProtocolError, PublicationState, RunnerReceiptHealth,
    RuntimeDiagnostics, RuntimeReceipt, RuntimeStatus, RuntimeStatusSnapshot, SecretToken,
    WorkState, CONTROL_DEADLINE_SECS, CONTROL_PROTOCOL_VERSION, DEFAULT_LOG_TAIL_LINES,
    LEGACY_RUNTIME_RECEIPT_SCHEMA_VERSION, MAX_ARGV_ELEMENTS, MAX_ARG_BYTES, MAX_ARTIFACTS,
    MAX_ARTIFACT_NAME_BYTES, MAX_ARTIFACT_URI_BYTES, MAX_ASSIGNMENT_TEXT_BYTES,
    MAX_CONTROL_REQUEST_BYTES, MAX_CONTROL_RESPONSE_BYTES, MAX_CWD_BYTES, MAX_DRIVER_BYTES,
    MAX_JOB_ARGV_JSON_BYTES, MAX_LOG_TAIL_LINES, MAX_SUMMARY_BYTES, RUNTIME_RECEIPT_SCHEMA_VERSION,
    SUPPORTED_JOB_DRIVER,
};
pub use server::{
    read_bounded_frame, write_bounded_frame, ControlHandler, ControlHandlerFn, ControlServerConfig,
    HandlerFuture, RuntimeServer, ServerError,
};
pub use store::{
    project_work_state, AssignmentSnapshot, CancelledRemoteJob, CreateJobOutcome, EnqueueOutcome,
    InboxBatch, InboxEvent, InboxRecord, InboxState, JobRecord, JobTransition, NewJob, OutboxEvent,
    OutboxRecord, QueueDepths, RecordCancelOutcome, RecoveryOutcome, RemoteCancelTombstone,
    RequeueOutcome, ResumeMode, RunnerIdentity, SessionRecord, StartupRecoveryPhase,
    StartupRecoverySnapshot, StoreDiagnostics, StoreError, StoreHandle, BASE_RETRY_DELAY_SECS,
    MAX_ARGV_JSON_BYTES, MAX_EVENT_JSON_BYTES, MAX_INBOX_RETRIES, MAX_PENDING_PER_CHANNEL,
    MAX_REMOTE_CANCEL_TOMBSTONES, MAX_RETRY_DELAY_SECS, REPLAY_SKEW_SECS, STORE_COMMAND_CAPACITY,
    STORE_SCHEMA_VERSION,
};
