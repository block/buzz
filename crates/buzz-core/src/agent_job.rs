//! Strict public wire protocol for durable agent jobs (kinds 43001–43006).

use chrono::{DateTime, Utc};
use nostr::{Event, EventId, PublicKey};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::kind::{
    KIND_JOB_ACCEPTED, KIND_JOB_CANCEL, KIND_JOB_ERROR, KIND_JOB_PROGRESS, KIND_JOB_REQUEST,
    KIND_JOB_RESULT,
};

/// Current public agent-job payload schema.
pub const AGENT_JOB_SCHEMA: u8 = 1;
/// Maximum serialized event content size.
pub const MAX_AGENT_JOB_CONTENT_BYTES: usize = 128 * 1024;
/// Maximum driver name size in UTF-8 bytes.
pub const MAX_JOB_DRIVER_BYTES: usize = 64;
/// Maximum number of argv entries.
pub const MAX_JOB_ARGV_ENTRIES: usize = 256;
/// Maximum size of one argv entry in UTF-8 bytes.
pub const MAX_JOB_ARG_BYTES: usize = 8 * 1024;
/// Maximum JSON-serialized argv array size.
pub const MAX_JOB_ARGV_JSON_BYTES: usize = 64 * 1024;
/// Maximum working-directory size in UTF-8 bytes.
pub const MAX_JOB_CWD_BYTES: usize = 4 * 1024;
/// Maximum summary size in UTF-8 bytes.
pub const MAX_JOB_SUMMARY_BYTES: usize = 4 * 1024;
/// Maximum cancellation reason size in UTF-8 bytes.
pub const MAX_JOB_REASON_BYTES: usize = 4 * 1024;
/// Maximum artifact references per progress or terminal payload.
pub const MAX_JOB_ARTIFACTS: usize = 32;
/// Maximum artifact name size in UTF-8 bytes.
pub const MAX_JOB_ARTIFACT_NAME_BYTES: usize = 256;
/// Maximum artifact URI size in UTF-8 bytes.
pub const MAX_JOB_ARTIFACT_URI_BYTES: usize = 2 * 1024;

/// Validation error for an agent-job payload or event envelope.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentJobValidationError {
    /// The event kind is not part of the agent-job protocol.
    #[error("unsupported agent job kind {0}")]
    UnsupportedKind(u32),
    /// JSON content exceeded the protocol limit.
    #[error("agent job content exceeds {max} bytes (got {got})")]
    ContentTooLarge {
        /// Maximum accepted byte length.
        max: usize,
        /// Actual byte length.
        got: usize,
    },
    /// JSON content did not match the strict payload for its kind.
    #[error("invalid agent job content: {0}")]
    InvalidContent(String),
    /// A payload field failed semantic validation.
    #[error("invalid agent job field {field}: {message}")]
    InvalidField {
        /// Field name.
        field: &'static str,
        /// Validation failure.
        message: String,
    },
    /// A required tag was absent.
    #[error("missing required agent job tag {0}")]
    MissingTag(&'static str),
    /// A singleton tag appeared more than once.
    #[error("duplicate agent job tag {0}")]
    DuplicateTag(String),
    /// A tag was not allowed for the event kind or did not have two elements.
    #[error("invalid agent job tag: {0}")]
    InvalidTag(String),
    /// A payload value did not match its corresponding tag.
    #[error("agent job payload/tag mismatch for {0}")]
    PayloadTagMismatch(&'static str),
    /// The event did not match a caller-supplied signer or link expectation.
    #[error("agent job event expectation mismatch for {0}")]
    ExpectationMismatch(&'static str),
}

fn invalid_field(field: &'static str, message: impl Into<String>) -> AgentJobValidationError {
    AgentJobValidationError::InvalidField {
        field,
        message: message.into(),
    }
}

fn check_len(field: &'static str, value: &str, max: usize) -> Result<(), AgentJobValidationError> {
    if value.len() > max {
        return Err(invalid_field(
            field,
            format!("exceeds {max} UTF-8 bytes (got {})", value.len()),
        ));
    }
    Ok(())
}

fn check_schema(schema: u8) -> Result<(), AgentJobValidationError> {
    if schema != AGENT_JOB_SCHEMA {
        return Err(invalid_field(
            "schema",
            format!("must be {AGENT_JOB_SCHEMA} (got {schema})"),
        ));
    }
    Ok(())
}

fn check_attempt(attempt: u32) -> Result<(), AgentJobValidationError> {
    if attempt == 0 {
        return Err(invalid_field("attempt", "must be at least 1"));
    }
    Ok(())
}

/// Reference to an artifact produced by a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobArtifact {
    /// Human-readable artifact name.
    pub name: String,
    /// URI from which the artifact can be retrieved.
    pub uri: String,
    /// Optional lowercase hexadecimal SHA-256 digest.
    pub sha256: Option<String>,
}

impl JobArtifact {
    /// Validate artifact field bounds and digest syntax.
    pub fn validate(&self) -> Result<(), AgentJobValidationError> {
        check_len("artifacts[].name", &self.name, MAX_JOB_ARTIFACT_NAME_BYTES)?;
        check_len("artifacts[].uri", &self.uri, MAX_JOB_ARTIFACT_URI_BYTES)?;
        if self.name.is_empty() {
            return Err(invalid_field("artifacts[].name", "must not be empty"));
        }
        if self.uri.is_empty() {
            return Err(invalid_field("artifacts[].uri", "must not be empty"));
        }
        if let Some(digest) = &self.sha256 {
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(invalid_field(
                    "artifacts[].sha256",
                    "must be exactly 64 lowercase hexadecimal characters",
                ));
            }
        }
        Ok(())
    }
}

fn check_artifacts(artifacts: &[JobArtifact]) -> Result<(), AgentJobValidationError> {
    if artifacts.len() > MAX_JOB_ARTIFACTS {
        return Err(invalid_field(
            "artifacts",
            format!(
                "contains more than {MAX_JOB_ARTIFACTS} entries (got {})",
                artifacts.len()
            ),
        ));
    }
    for artifact in artifacts {
        artifact.validate()?;
    }
    Ok(())
}

/// Request payload for kind 43001.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentJobRequest {
    /// Wire schema version. Schema 1 is the only accepted value.
    pub schema: u8,
    /// Privileged runtime driver. Schema 1 accepts only `lh`.
    pub driver: String,
    /// Argument vector passed directly to the configured driver executable.
    pub argv: Vec<String>,
    /// Operator-approved absolute working directory.
    pub cwd: String,
    /// Human-readable job summary.
    pub summary: String,
}

impl AgentJobRequest {
    /// Validate the request payload and all aggregate limits.
    pub fn validate(&self) -> Result<(), AgentJobValidationError> {
        check_schema(self.schema)?;
        check_len("driver", &self.driver, MAX_JOB_DRIVER_BYTES)?;
        if self.driver != "lh" {
            return Err(invalid_field("driver", "schema 1 accepts only \"lh\""));
        }
        if self.argv.len() > MAX_JOB_ARGV_ENTRIES {
            return Err(invalid_field(
                "argv",
                format!(
                    "contains more than {MAX_JOB_ARGV_ENTRIES} entries (got {})",
                    self.argv.len()
                ),
            ));
        }
        for arg in &self.argv {
            check_len("argv[]", arg, MAX_JOB_ARG_BYTES)?;
        }
        let argv_json = serde_json::to_vec(&self.argv)
            .map_err(|error| invalid_field("argv", error.to_string()))?;
        if argv_json.len() > MAX_JOB_ARGV_JSON_BYTES {
            return Err(invalid_field(
                "argv",
                format!(
                    "serialized JSON exceeds {MAX_JOB_ARGV_JSON_BYTES} bytes (got {})",
                    argv_json.len()
                ),
            ));
        }
        check_len("cwd", &self.cwd, MAX_JOB_CWD_BYTES)?;
        check_len("summary", &self.summary, MAX_JOB_SUMMARY_BYTES)?;
        Ok(())
    }
}

/// The only valid state in a kind-43002 accepted payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentJobAcceptedState {
    /// The target runtime accepted the job.
    Accepted,
}

impl AgentJobAcceptedState {
    /// Return the canonical wire string.
    pub const fn as_str(self) -> &'static str {
        "accepted"
    }
}

/// Accepted payload for kind 43002.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentJobAccepted {
    /// Wire schema version.
    pub schema: u8,
    /// Durable job UUID.
    pub job: Uuid,
    /// Execution attempt, starting at one.
    pub attempt: u32,
    /// Fixed accepted state.
    pub state: AgentJobAcceptedState,
    /// UTC acceptance time.
    pub accepted_at: DateTime<Utc>,
}

impl AgentJobAccepted {
    /// Validate the accepted payload.
    pub fn validate(&self) -> Result<(), AgentJobValidationError> {
        check_schema(self.schema)?;
        check_attempt(self.attempt)
    }
}

/// Valid states in a kind-43003 progress payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentJobProgressState {
    /// The job is running.
    Running,
    /// Cancellation has been requested and is in progress.
    Cancelling,
}

impl AgentJobProgressState {
    /// Return the canonical wire string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Cancelling => "cancelling",
        }
    }
}

/// Progress payload for kind 43003.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentJobProgress {
    /// Wire schema version.
    pub schema: u8,
    /// Durable job UUID.
    pub job: Uuid,
    /// Execution attempt, starting at one.
    pub attempt: u32,
    /// Monotonically increasing progress sequence, starting at one.
    pub seq: u64,
    /// Running or cancelling state.
    pub state: AgentJobProgressState,
    /// Bounded human-readable progress summary.
    pub summary: String,
    /// Bounded artifact references.
    pub artifacts: Vec<JobArtifact>,
}

impl AgentJobProgress {
    /// Validate the progress payload.
    pub fn validate(&self) -> Result<(), AgentJobValidationError> {
        check_schema(self.schema)?;
        check_attempt(self.attempt)?;
        if self.seq == 0 {
            return Err(invalid_field("seq", "must be at least 1"));
        }
        check_len("summary", &self.summary, MAX_JOB_SUMMARY_BYTES)?;
        check_artifacts(&self.artifacts)
    }
}

/// The only valid state in a kind-43004 result payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentJobResultState {
    /// The job completed successfully.
    Succeeded,
}

impl AgentJobResultState {
    /// Return the canonical wire string.
    pub const fn as_str(self) -> &'static str {
        "succeeded"
    }
}

/// Successful terminal payload for kind 43004.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentJobResult {
    /// Wire schema version.
    pub schema: u8,
    /// Durable job UUID.
    pub job: Uuid,
    /// Execution attempt, starting at one.
    pub attempt: u32,
    /// Fixed succeeded state.
    pub state: AgentJobResultState,
    /// Driver process exit code.
    pub exit_code: i32,
    /// Bounded human-readable result summary.
    pub summary: String,
    /// Bounded artifact references.
    pub artifacts: Vec<JobArtifact>,
    /// UTC completion time.
    pub finished_at: DateTime<Utc>,
}

impl AgentJobResult {
    /// Validate the result payload.
    pub fn validate(&self) -> Result<(), AgentJobValidationError> {
        check_schema(self.schema)?;
        check_attempt(self.attempt)?;
        check_len("summary", &self.summary, MAX_JOB_SUMMARY_BYTES)?;
        check_artifacts(&self.artifacts)
    }
}

/// Cancellation request payload for kind 43005.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentJobCancel {
    /// Wire schema version.
    pub schema: u8,
    /// Durable job UUID.
    pub job: Uuid,
    /// Bounded human-readable cancellation reason.
    pub reason: String,
}

impl AgentJobCancel {
    /// Validate the cancellation payload.
    pub fn validate(&self) -> Result<(), AgentJobValidationError> {
        check_schema(self.schema)?;
        check_len("reason", &self.reason, MAX_JOB_REASON_BYTES)
    }
}

/// Valid states in a kind-43006 error payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentJobErrorState {
    /// The job failed.
    Failed,
    /// The job was cancelled.
    Cancelled,
    /// The runtime lost authoritative runner state.
    Lost,
}

impl AgentJobErrorState {
    /// Return the canonical wire string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Lost => "lost",
        }
    }
}

/// Failed terminal payload for kind 43006.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentJobError {
    /// Wire schema version.
    pub schema: u8,
    /// Durable job UUID.
    pub job: Uuid,
    /// Execution attempt, starting at one.
    pub attempt: u32,
    /// Failed, cancelled, or lost terminal state.
    pub state: AgentJobErrorState,
    /// Stable machine-readable error code.
    pub code: String,
    /// Bounded human-readable error summary.
    pub summary: String,
    /// Whether a later attempt may be appropriate.
    pub retryable: bool,
    /// Bounded artifact references.
    pub artifacts: Vec<JobArtifact>,
    /// UTC terminal time.
    pub finished_at: DateTime<Utc>,
}

impl AgentJobError {
    /// Validate the error payload.
    pub fn validate(&self) -> Result<(), AgentJobValidationError> {
        check_schema(self.schema)?;
        check_attempt(self.attempt)?;
        check_len("summary", &self.summary, MAX_JOB_SUMMARY_BYTES)?;
        check_artifacts(&self.artifacts)
    }
}

/// Strict typed content carried by one of kinds 43001–43006.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentJobPayload {
    /// Kind 43001 request.
    Request(AgentJobRequest),
    /// Kind 43002 acceptance.
    Accepted(AgentJobAccepted),
    /// Kind 43003 progress.
    Progress(AgentJobProgress),
    /// Kind 43004 success.
    Result(AgentJobResult),
    /// Kind 43005 cancellation request.
    Cancel(AgentJobCancel),
    /// Kind 43006 failure/cancellation/loss.
    Error(AgentJobError),
}

impl AgentJobPayload {
    /// Return the job UUID when the payload carries it.
    pub fn job(&self) -> Option<Uuid> {
        match self {
            Self::Request(_) => None,
            Self::Accepted(payload) => Some(payload.job),
            Self::Progress(payload) => Some(payload.job),
            Self::Result(payload) => Some(payload.job),
            Self::Cancel(payload) => Some(payload.job),
            Self::Error(payload) => Some(payload.job),
        }
    }

    /// Return the attempt for lifecycle payloads that carry one.
    pub fn attempt(&self) -> Option<u32> {
        match self {
            Self::Accepted(payload) => Some(payload.attempt),
            Self::Progress(payload) => Some(payload.attempt),
            Self::Result(payload) => Some(payload.attempt),
            Self::Error(payload) => Some(payload.attempt),
            Self::Request(_) | Self::Cancel(_) => None,
        }
    }

    /// Return the progress sequence when present.
    pub fn seq(&self) -> Option<u64> {
        match self {
            Self::Progress(payload) => Some(payload.seq),
            _ => None,
        }
    }

    /// Return the canonical lifecycle state when the payload carries one.
    pub fn state(&self) -> Option<&'static str> {
        match self {
            Self::Accepted(payload) => Some(payload.state.as_str()),
            Self::Progress(payload) => Some(payload.state.as_str()),
            Self::Result(payload) => Some(payload.state.as_str()),
            Self::Error(payload) => Some(payload.state.as_str()),
            Self::Request(_) | Self::Cancel(_) => None,
        }
    }

    /// Return the human-readable summary or cancellation reason when present.
    pub fn summary(&self) -> Option<&str> {
        match self {
            Self::Request(payload) => Some(&payload.summary),
            Self::Accepted(_) => None,
            Self::Progress(payload) => Some(&payload.summary),
            Self::Result(payload) => Some(&payload.summary),
            Self::Cancel(payload) => Some(&payload.reason),
            Self::Error(payload) => Some(&payload.summary),
        }
    }

    /// Return whether this payload is immutable terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Result(_) | Self::Error(_))
    }
}

/// Parsed, structurally validated agent-job event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAgentJobEvent {
    /// Event kind in the 43001–43006 range.
    pub kind: u32,
    /// Channel UUID from the exact singleton `h` tag.
    pub channel_id: Uuid,
    /// Counterparty from the exact singleton `p` tag.
    pub peer: PublicKey,
    /// Durable job UUID from the exact singleton `job` tag.
    pub job: Uuid,
    /// Optional request/source link from the singleton `e` tag.
    pub linked_event_id: Option<EventId>,
    /// Optional parent durable job UUID on request events.
    pub parent_job: Option<Uuid>,
    /// Progress sequence from the singleton `seq` tag.
    pub seq: Option<u64>,
    /// Strict typed JSON payload.
    pub payload: AgentJobPayload,
}

/// Caller-known signer and routing values to enforce after structural parsing.
#[derive(Debug, Clone, Default)]
pub struct AgentJobEventExpectations {
    /// Expected event signer.
    pub author: Option<PublicKey>,
    /// Expected channel UUID.
    pub channel_id: Option<Uuid>,
    /// Expected counterparty in the `p` tag.
    pub peer: Option<PublicKey>,
    /// Expected request/source event link.
    pub linked_event_id: Option<EventId>,
    /// Expected durable job UUID.
    pub job: Option<Uuid>,
}

impl ParsedAgentJobEvent {
    /// Enforce caller-known signer, scope, counterparty, link, and job values.
    pub fn validate_expectations(
        &self,
        event: &Event,
        expectations: &AgentJobEventExpectations,
    ) -> Result<(), AgentJobValidationError> {
        if expectations
            .author
            .as_ref()
            .is_some_and(|value| value != &event.pubkey)
        {
            return Err(AgentJobValidationError::ExpectationMismatch("author"));
        }
        if expectations
            .channel_id
            .is_some_and(|value| value != self.channel_id)
        {
            return Err(AgentJobValidationError::ExpectationMismatch("channel_id"));
        }
        if expectations
            .peer
            .as_ref()
            .is_some_and(|value| value != &self.peer)
        {
            return Err(AgentJobValidationError::ExpectationMismatch("peer"));
        }
        if expectations
            .linked_event_id
            .as_ref()
            .is_some_and(|value| Some(value) != self.linked_event_id.as_ref())
        {
            return Err(AgentJobValidationError::ExpectationMismatch(
                "linked_event_id",
            ));
        }
        if expectations.job.is_some_and(|value| value != self.job) {
            return Err(AgentJobValidationError::ExpectationMismatch("job"));
        }
        Ok(())
    }
}

fn deserialize_payload<T>(content: &str) -> Result<T, AgentJobValidationError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(content)
        .map_err(|error| AgentJobValidationError::InvalidContent(error.to_string()))
}

fn parse_payload(kind: u32, content: &str) -> Result<AgentJobPayload, AgentJobValidationError> {
    let payload = match kind {
        KIND_JOB_REQUEST => {
            let payload: AgentJobRequest = deserialize_payload(content)?;
            payload.validate()?;
            AgentJobPayload::Request(payload)
        }
        KIND_JOB_ACCEPTED => {
            let payload: AgentJobAccepted = deserialize_payload(content)?;
            payload.validate()?;
            AgentJobPayload::Accepted(payload)
        }
        KIND_JOB_PROGRESS => {
            let payload: AgentJobProgress = deserialize_payload(content)?;
            payload.validate()?;
            AgentJobPayload::Progress(payload)
        }
        KIND_JOB_RESULT => {
            let payload: AgentJobResult = deserialize_payload(content)?;
            payload.validate()?;
            AgentJobPayload::Result(payload)
        }
        KIND_JOB_CANCEL => {
            let payload: AgentJobCancel = deserialize_payload(content)?;
            payload.validate()?;
            AgentJobPayload::Cancel(payload)
        }
        KIND_JOB_ERROR => {
            let payload: AgentJobError = deserialize_payload(content)?;
            payload.validate()?;
            AgentJobPayload::Error(payload)
        }
        _ => return Err(AgentJobValidationError::UnsupportedKind(kind)),
    };
    Ok(payload)
}

fn required_tag<'a>(
    values: &'a std::collections::HashMap<String, String>,
    name: &'static str,
) -> Result<&'a str, AgentJobValidationError> {
    values
        .get(name)
        .map(String::as_str)
        .ok_or(AgentJobValidationError::MissingTag(name))
}

fn validate_auth_decimal(
    value: &str,
    max: u64,
    label: &str,
) -> Result<(), AgentJobValidationError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(AgentJobValidationError::InvalidTag(format!(
            "auth {label} must be a canonical decimal integer"
        )));
    }
    let parsed = value.parse::<u64>().map_err(|error| {
        AgentJobValidationError::InvalidTag(format!("invalid auth {label}: {error}"))
    })?;
    if parsed > max {
        return Err(AgentJobValidationError::InvalidTag(format!(
            "auth {label} exceeds {max}"
        )));
    }
    Ok(())
}

fn validate_auth_tag(parts: &[String]) -> Result<(), AgentJobValidationError> {
    if parts.len() != 4 {
        return Err(AgentJobValidationError::InvalidTag(format!(
            "auth tag must have exactly four elements, got {}",
            parts.len()
        )));
    }
    let owner = &parts[1];
    if owner.len() != 64
        || !owner
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(AgentJobValidationError::InvalidTag(
            "auth owner must be 64 lowercase hexadecimal characters".into(),
        ));
    }
    for clause in parts[2].split('&') {
        if parts[2].is_empty() {
            break;
        }
        if let Some(value) = clause.strip_prefix("kind=") {
            validate_auth_decimal(value, 65_535, "kind")?;
        } else if let Some(value) = clause.strip_prefix("created_at<") {
            validate_auth_decimal(value, 4_294_967_295, "created_at<")?;
        } else if let Some(value) = clause.strip_prefix("created_at>") {
            validate_auth_decimal(value, 4_294_967_295, "created_at>")?;
        } else {
            return Err(AgentJobValidationError::InvalidTag(format!(
                "unsupported auth condition {clause:?}"
            )));
        }
    }
    let signature = &parts[3];
    if signature.len() != 128
        || !signature
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(AgentJobValidationError::InvalidTag(
            "auth signature must be 128 lowercase hexadecimal characters".into(),
        ));
    }
    Ok(())
}

/// Parse and strictly validate a signed or unsigned Nostr job event.
///
/// Job tags must be exact two-element singletons. One canonical four-element
/// NIP-OA `auth` tag is also accepted but is not job linkage. Request events may
/// omit their source `e` link; all lifecycle events require it, and only
/// progress carries `seq`.
pub fn parse_agent_job_event(
    event: &Event,
) -> Result<ParsedAgentJobEvent, AgentJobValidationError> {
    let kind = event.kind.as_u16() as u32;
    if !matches!(
        kind,
        KIND_JOB_REQUEST
            | KIND_JOB_ACCEPTED
            | KIND_JOB_PROGRESS
            | KIND_JOB_RESULT
            | KIND_JOB_CANCEL
            | KIND_JOB_ERROR
    ) {
        return Err(AgentJobValidationError::UnsupportedKind(kind));
    }
    if event.content.len() > MAX_AGENT_JOB_CONTENT_BYTES {
        return Err(AgentJobValidationError::ContentTooLarge {
            max: MAX_AGENT_JOB_CONTENT_BYTES,
            got: event.content.len(),
        });
    }

    let allowed: &[&str] = if kind == KIND_JOB_REQUEST {
        &["h", "p", "job", "e", "parent-job"]
    } else if kind == KIND_JOB_PROGRESS {
        &["h", "p", "job", "e", "seq"]
    } else {
        &["h", "p", "job", "e"]
    };
    let mut values = std::collections::HashMap::with_capacity(allowed.len());
    let mut auth_seen = false;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) == Some("auth") {
            if auth_seen {
                return Err(AgentJobValidationError::DuplicateTag("auth".into()));
            }
            validate_auth_tag(parts)?;
            auth_seen = true;
            continue;
        }
        if parts.len() != 2 {
            return Err(AgentJobValidationError::InvalidTag(format!(
                "expected exactly two elements, got {}",
                parts.len()
            )));
        }
        let name = parts[0].as_str();
        if !allowed.contains(&name) {
            return Err(AgentJobValidationError::InvalidTag(format!(
                "tag {name:?} is not allowed for kind {kind}"
            )));
        }
        if values
            .insert(name.to_owned(), parts[1].to_owned())
            .is_some()
        {
            return Err(AgentJobValidationError::DuplicateTag(name.to_owned()));
        }
    }

    let channel_text = required_tag(&values, "h")?;
    let channel_id = Uuid::parse_str(channel_text)
        .map_err(|error| AgentJobValidationError::InvalidTag(format!("invalid h UUID: {error}")))?;
    if channel_id.to_string() != channel_text {
        return Err(AgentJobValidationError::InvalidTag(
            "h tag UUID is not canonical".into(),
        ));
    }

    let peer_text = required_tag(&values, "p")?;
    let peer = PublicKey::from_hex(peer_text).map_err(|error| {
        AgentJobValidationError::InvalidTag(format!("invalid p pubkey: {error}"))
    })?;
    if peer.to_hex() != peer_text {
        return Err(AgentJobValidationError::InvalidTag(
            "p tag pubkey is not canonical lowercase hex".into(),
        ));
    }

    let job_text = required_tag(&values, "job")?;
    let job = Uuid::parse_str(job_text).map_err(|error| {
        AgentJobValidationError::InvalidTag(format!("invalid job UUID: {error}"))
    })?;
    if job.to_string() != job_text {
        return Err(AgentJobValidationError::InvalidTag(
            "job tag UUID is not canonical".into(),
        ));
    }

    let linked_event_id = match values.get("e") {
        Some(value) => {
            let event_id = EventId::from_hex(value).map_err(|error| {
                AgentJobValidationError::InvalidTag(format!("invalid e event ID: {error}"))
            })?;
            if event_id.to_hex() != *value {
                return Err(AgentJobValidationError::InvalidTag(
                    "e tag event ID is not canonical lowercase hex".into(),
                ));
            }
            Some(event_id)
        }
        None if kind == KIND_JOB_REQUEST => None,
        None => return Err(AgentJobValidationError::MissingTag("e")),
    };

    let parent_job = match values.get("parent-job") {
        Some(value) => {
            let parsed = Uuid::parse_str(value).map_err(|error| {
                AgentJobValidationError::InvalidTag(format!("invalid parent-job UUID: {error}"))
            })?;
            if parsed.to_string() != *value {
                return Err(AgentJobValidationError::InvalidTag(
                    "parent-job UUID is not canonical".into(),
                ));
            }
            Some(parsed)
        }
        None => None,
    };

    let seq = match values.get("seq") {
        Some(value) => {
            let parsed = value.parse::<u64>().map_err(|error| {
                AgentJobValidationError::InvalidTag(format!("invalid seq: {error}"))
            })?;
            if parsed == 0 || parsed.to_string() != *value {
                return Err(AgentJobValidationError::InvalidTag(
                    "seq must be a canonical positive decimal integer".into(),
                ));
            }
            Some(parsed)
        }
        None if kind == KIND_JOB_PROGRESS => {
            return Err(AgentJobValidationError::MissingTag("seq"))
        }
        None => None,
    };

    let payload = parse_payload(kind, &event.content)?;
    if payload.job().is_some_and(|payload_job| payload_job != job) {
        return Err(AgentJobValidationError::PayloadTagMismatch("job"));
    }
    if let AgentJobPayload::Progress(progress) = &payload {
        if Some(progress.seq) != seq {
            return Err(AgentJobValidationError::PayloadTagMismatch("seq"));
        }
    }

    Ok(ParsedAgentJobEvent {
        kind,
        channel_id,
        peer,
        parent_job,
        job,
        linked_event_id,
        seq,
        payload,
    })
}

/// Parse an agent-job event and enforce caller-supplied signer/link expectations.
pub fn validate_agent_job_event(
    event: &Event,
    expectations: &AgentJobEventExpectations,
) -> Result<ParsedAgentJobEvent, AgentJobValidationError> {
    let parsed = parse_agent_job_event(event)?;
    parsed.validate_expectations(event, expectations)?;
    Ok(parsed)
}
