//! Runtime-neutral execution-node protocol types.
//!
//! This module is deliberately free of transport and runtime concerns. Desktop,
//! `buzz-node`, and future clients share these types at the relay-protocol
//! boundary, while authorization, encryption, persistence, and workload
//! reconciliation remain owned by their respective components.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;
use uuid::Uuid;

/// The current wire version for execution-node commands and receipts.
pub const EXECUTION_PROTOCOL_VERSION: u16 = 1;

/// The maximum lifetime of a command envelope.
pub const MAX_COMMAND_TTL: Duration = Duration::minutes(15);

const MAX_DISPLAY_NAME_BYTES: usize = 128;
const MAX_PROVIDER_BYTES: usize = 128;
const MAX_RUNTIME_BYTES: usize = 128;
const MAX_SESSION_ID_BYTES: usize = 128;

/// Validation failures for execution protocol values.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExecutionValidationError {
    /// A required text field was empty.
    #[error("{field} must not be empty")]
    Empty {
        /// Name of the empty field.
        field: &'static str,
    },
    /// A text field contained a NUL byte.
    #[error("{field} must not contain a NUL byte")]
    NulByte {
        /// Name of the field containing the NUL byte.
        field: &'static str,
    },
    /// A text field exceeded its protocol size limit.
    #[error("{field} exceeds the maximum size of {max} bytes")]
    TooLong {
        /// Name of the oversized field.
        field: &'static str,
        /// Maximum permitted byte length.
        max: usize,
    },
    /// A text field contained an unsafe control character.
    #[error("{field} contains an unsafe control character")]
    UnsafeCharacter {
        /// Name of the field containing the unsafe character.
        field: &'static str,
    },
    /// A node identity was not a canonical Nostr public key.
    #[error("execution node identity must be 64 hexadecimal characters")]
    InvalidNodeId,
    /// A workload identity was not a canonical UUID.
    #[error("workload identity must be a UUID")]
    InvalidWorkloadId,
    /// The protocol version is not supported by this implementation.
    #[error("unsupported execution protocol version: {0}")]
    UnsupportedProtocolVersion(u16),
    /// The command expiry did not occur after issuance.
    #[error("command expiry must be after issuance")]
    InvalidExpiry,
    /// The command lived longer than the protocol allows.
    #[error("command lifetime exceeds the protocol maximum")]
    ExpiryTooLong,
    /// The command was received after its expiry.
    #[error("command has expired")]
    Expired,
    /// A credential reference was repeated in one workload specification.
    #[error("workload contains a duplicate credential reference")]
    DuplicateCredential,
    /// A receipt sequence number was zero.
    #[error("receipt sequence must be greater than zero")]
    InvalidSequence,
    /// A receipt sequence did not advance beyond the previous sequence.
    #[error("receipt sequence {current} does not follow previous sequence {previous}")]
    InvalidSequenceOrder {
        /// Previously observed sequence number.
        previous: u64,
        /// Sequence number that failed to advance.
        current: u64,
    },
    /// A receipt workload did not match the command workload.
    #[error("receipt workload does not match the command workload")]
    WorkloadMismatch,
    /// A failed or rejected receipt did not include a safe error code.
    #[error("terminal failure receipts must include a safe error code")]
    MissingError,
    /// A successful or non-terminal receipt included an error code.
    #[error("successful and non-terminal receipts must not include an error code")]
    UnexpectedError,
}

/// Errors returned when decoding and validating a JSON command envelope.
#[derive(Debug, Error)]
pub enum ExecutionDecodeError {
    /// The JSON representation was malformed.
    #[error("invalid execution command JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The decoded command failed protocol validation.
    #[error(transparent)]
    Validation(#[from] ExecutionValidationError),
}

fn validate_text(
    field: &'static str,
    value: &str,
    max: usize,
    allow_empty: bool,
) -> Result<(), ExecutionValidationError> {
    if !allow_empty && value.is_empty() {
        return Err(ExecutionValidationError::Empty { field });
    }
    if value.len() > max {
        return Err(ExecutionValidationError::TooLong { field, max });
    }
    if value.contains('\0') {
        return Err(ExecutionValidationError::NulByte { field });
    }
    if value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(ExecutionValidationError::UnsafeCharacter { field });
    }
    Ok(())
}

/// Stable identity of an execution node, represented by its Nostr public key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExecutionNodeId(String);

impl ExecutionNodeId {
    /// Create a canonical execution-node identity from a 64-character hex key.
    pub fn new(value: impl Into<String>) -> Result<Self, ExecutionValidationError> {
        Self::try_from(value.into())
    }

    /// Return the canonical lowercase public key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ExecutionNodeId {
    type Error = ExecutionValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ExecutionValidationError::InvalidNodeId);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }
}

impl From<ExecutionNodeId> for String {
    fn from(value: ExecutionNodeId) -> Self {
        value.0
    }
}

/// Stable identity of a managed workload.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct WorkloadId(String);

impl WorkloadId {
    /// Create a workload identity from a UUID string.
    pub fn new(value: impl Into<String>) -> Result<Self, ExecutionValidationError> {
        Self::try_from(value.into())
    }

    /// Create a new random workload identity.
    pub fn random() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Return the canonical UUID string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for WorkloadId {
    type Error = ExecutionValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let uuid =
            Uuid::parse_str(&value).map_err(|_| ExecutionValidationError::InvalidWorkloadId)?;
        Ok(Self(uuid.to_string()))
    }
}

impl From<WorkloadId> for String {
    fn from(value: WorkloadId) -> Self {
        value.0
    }
}

/// Correlation identity for one command request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(Uuid);

impl RequestId {
    /// Create a new request correlation identity.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

/// Idempotency identity for one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommandId(Uuid);

impl CommandId {
    /// Create a new command identity.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CommandId {
    fn default() -> Self {
        Self::new()
    }
}

/// A reference to a credential stored by the execution node.
///
/// This type intentionally has no field for a secret, private key, token, or
/// environment value. The reference is safe to include in an encrypted command
/// payload and in node-owned state, while the credential material remains local
/// to the node.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialRef {
    /// Provider namespace for the credential.
    pub provider: String,
    /// Node-local credential handle.
    pub name: String,
}

impl CredentialRef {
    /// Create a validated node-local credential reference.
    pub fn new(
        provider: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, ExecutionValidationError> {
        let credential = Self {
            provider: provider.into(),
            name: name.into(),
        };
        validate_text(
            "credential provider",
            &credential.provider,
            MAX_PROVIDER_BYTES,
            false,
        )?;
        validate_text(
            "credential name",
            &credential.name,
            MAX_SESSION_ID_BYTES,
            false,
        )?;
        Ok(credential)
    }
}

/// Safe workload projection sent in a deploy command.
///
/// Runtime-specific infrastructure such as Docker images, sockets, container
/// contexts, and raw environment secrets is intentionally absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkloadSpec {
    /// Stable workload identity.
    pub workload_id: WorkloadId,
    /// User-visible workload name.
    pub display_name: String,
    /// ACP runtime identifier, such as `goose` or `codex`.
    pub runtime: String,
    /// Optional runtime-specific model identifier.
    pub model: Option<String>,
    /// Optional inference provider identifier.
    pub provider: Option<String>,
    /// References to credentials already stored by the node.
    pub credential_refs: Vec<CredentialRef>,
}

impl WorkloadSpec {
    /// Build a safe agent workload projection.
    pub fn agent(
        workload_id: WorkloadId,
        display_name: impl Into<String>,
        runtime: impl Into<String>,
        model: Option<String>,
        provider: Option<String>,
        credential_refs: Vec<CredentialRef>,
    ) -> Result<Self, ExecutionValidationError> {
        let workload = Self {
            workload_id,
            display_name: display_name.into(),
            runtime: runtime.into(),
            model,
            provider,
            credential_refs,
        };
        workload.validate()?;
        Ok(workload)
    }

    /// Validate the safe projection before it crosses a protocol boundary.
    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        validate_text(
            "workload display name",
            &self.display_name,
            MAX_DISPLAY_NAME_BYTES,
            false,
        )?;
        validate_text("workload runtime", &self.runtime, MAX_RUNTIME_BYTES, false)?;
        if let Some(model) = &self.model {
            validate_text("workload model", model, MAX_PROVIDER_BYTES, false)?;
        }
        if let Some(provider) = &self.provider {
            validate_text("workload provider", provider, MAX_PROVIDER_BYTES, false)?;
        }
        let mut unique_credentials = BTreeSet::new();
        for credential in &self.credential_refs {
            let validated = CredentialRef::new(&credential.provider, &credential.name)?;
            if !unique_credentials.insert(validated) {
                return Err(ExecutionValidationError::DuplicateCredential);
            }
        }
        Ok(())
    }
}

/// A provider-authentication session request.
///
/// The session identifies where authentication state should be stored. It does
/// not carry the provider response, token, private key, or raw login material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderAuthSession {
    /// Workload whose provider configuration is being authenticated.
    pub workload_id: WorkloadId,
    /// Provider namespace to authenticate.
    pub provider: String,
    /// Node-local session handle used to correlate the encrypted response.
    pub session_id: String,
    /// Time at which the authentication session must be abandoned.
    pub expires_at: DateTime<Utc>,
}

impl ProviderAuthSession {
    /// Create a validated provider-authentication session request.
    pub fn new(
        workload_id: WorkloadId,
        provider: impl Into<String>,
        session_id: impl Into<String>,
        expires_at: DateTime<Utc>,
    ) -> Result<Self, ExecutionValidationError> {
        let session = Self {
            workload_id,
            provider: provider.into(),
            session_id: session_id.into(),
            expires_at,
        };
        session.validate_at(Utc::now())?;
        Ok(session)
    }

    /// Validate the session at a supplied clock instant.
    pub fn validate_at(&self, now: DateTime<Utc>) -> Result<(), ExecutionValidationError> {
        validate_text("provider", &self.provider, MAX_PROVIDER_BYTES, false)?;
        validate_text(
            "provider auth session",
            &self.session_id,
            MAX_SESSION_ID_BYTES,
            false,
        )?;
        if self.expires_at <= now {
            return Err(ExecutionValidationError::Expired);
        }
        if self.expires_at - now > MAX_COMMAND_TTL {
            return Err(ExecutionValidationError::ExpiryTooLong);
        }
        Ok(())
    }
}

/// Typed operations understood by an execution node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase")]
pub enum ExecutionCommand {
    /// Create or reconcile one workload from a safe specification.
    Deploy {
        /// Desired workload configuration.
        workload: WorkloadSpec,
    },
    /// Start an existing workload.
    Start {
        /// Workload to start.
        workload_id: WorkloadId,
    },
    /// Stop an existing workload.
    Stop {
        /// Workload to stop.
        workload_id: WorkloadId,
    },
    /// Restart an existing workload.
    Restart {
        /// Workload to restart.
        workload_id: WorkloadId,
    },
    /// Remove an existing workload and its node-side runtime state.
    Remove {
        /// Workload to remove.
        workload_id: WorkloadId,
    },
    /// Start a provider-authentication session for an existing workload.
    AuthenticateProvider {
        /// Authentication session metadata.
        session: ProviderAuthSession,
    },
}

impl ExecutionCommand {
    /// Return the workload targeted by this command.
    pub fn workload_id(&self) -> &WorkloadId {
        match self {
            Self::Deploy { workload } => &workload.workload_id,
            Self::Start { workload_id }
            | Self::Stop { workload_id }
            | Self::Restart { workload_id }
            | Self::Remove { workload_id } => workload_id,
            Self::AuthenticateProvider { session } => &session.workload_id,
        }
    }

    /// Validate command payload fields at a supplied clock instant.
    pub fn validate_at(&self, now: DateTime<Utc>) -> Result<(), ExecutionValidationError> {
        match self {
            Self::Deploy { workload } => workload.validate(),
            Self::Start { .. } | Self::Stop { .. } | Self::Restart { .. } | Self::Remove { .. } => {
                Ok(())
            }
            Self::AuthenticateProvider { session } => session.validate_at(now),
        }
    }
}

/// Correlated, time-bounded command envelope sent to one execution node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionCommandEnvelope {
    /// Wire protocol version.
    pub protocol_version: u16,
    /// Idempotency identity for this command.
    pub command_id: CommandId,
    /// Request identity used to correlate receipts.
    pub request_id: RequestId,
    /// Target execution node.
    pub node_id: ExecutionNodeId,
    /// Time at which the command was issued.
    pub issued_at: DateTime<Utc>,
    /// Time after which the node must reject the command.
    pub expires_at: DateTime<Utc>,
    /// Typed execution operation.
    pub command: ExecutionCommand,
}

impl ExecutionCommandEnvelope {
    /// Construct and validate a new command envelope.
    pub fn new(
        node_id: ExecutionNodeId,
        issued_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        command: ExecutionCommand,
    ) -> Result<Self, ExecutionValidationError> {
        let envelope = Self {
            protocol_version: EXECUTION_PROTOCOL_VERSION,
            command_id: CommandId::new(),
            request_id: RequestId::new(),
            node_id,
            issued_at,
            expires_at,
            command,
        };
        envelope.validate_at(issued_at)?;
        Ok(envelope)
    }

    /// Validate protocol version, lifetime, and command payload.
    pub fn validate_at(&self, now: DateTime<Utc>) -> Result<(), ExecutionValidationError> {
        if self.protocol_version != EXECUTION_PROTOCOL_VERSION {
            return Err(ExecutionValidationError::UnsupportedProtocolVersion(
                self.protocol_version,
            ));
        }
        if self.expires_at <= self.issued_at {
            return Err(ExecutionValidationError::InvalidExpiry);
        }
        if self.expires_at - self.issued_at > MAX_COMMAND_TTL {
            return Err(ExecutionValidationError::ExpiryTooLong);
        }
        if now >= self.expires_at {
            return Err(ExecutionValidationError::Expired);
        }
        self.command.validate_at(now)
    }

    /// Decode and validate a JSON command envelope at a supplied clock instant.
    pub fn from_json_at(input: &str, now: DateTime<Utc>) -> Result<Self, ExecutionDecodeError> {
        let envelope: Self = serde_json::from_str(input)?;
        envelope.validate_at(now)?;
        Ok(envelope)
    }

    /// Return the command target.
    pub fn node_id(&self) -> &ExecutionNodeId {
        &self.node_id
    }

    /// Return the request correlation identity.
    pub fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Return the command idempotency identity.
    pub fn command_id(&self) -> CommandId {
        self.command_id
    }
}

/// Lifecycle projection for an execution node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionNodeLifecycle {
    /// The node is not connected to the relay.
    Unavailable,
    /// The node is establishing relay connectivity.
    Connecting,
    /// The node is connected and can accept work.
    Ready,
    /// The node is connected but one or more workloads are active.
    Busy,
    /// The node is connected with a degraded capability or runtime condition.
    Degraded,
    /// The node is shutting down and will not accept new work.
    Draining,
}

/// Lifecycle projection for one managed workload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadLifecycle {
    /// The workload is known but has not been deployed.
    Pending,
    /// The node is reconciling the desired workload.
    Deploying,
    /// The node is starting the workload.
    Starting,
    /// The workload is running.
    Running,
    /// The node is stopping the workload.
    Stopping,
    /// The node is restarting the workload.
    Restarting,
    /// The workload is stopped and can be started again.
    Stopped,
    /// The workload failed and may be retried.
    Failed,
    /// The node is removing the workload.
    Removing,
    /// The workload no longer exists on the node.
    Removed,
}

impl WorkloadLifecycle {
    /// Return whether this lifecycle state is terminal for the current action.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Failed | Self::Removed)
    }
}

/// Capabilities that may be advertised by a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionCapability {
    /// The node can deploy workloads.
    Deploy,
    /// The node can start workloads.
    Start,
    /// The node can stop workloads.
    Stop,
    /// The node can restart workloads.
    Restart,
    /// The node can remove workloads.
    Remove,
    /// The node can initiate provider-authentication sessions.
    ProviderAuthentication,
}

/// Safe status projection for one workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkloadStatus {
    /// Workload identity.
    pub workload_id: WorkloadId,
    /// Current workload lifecycle.
    pub lifecycle: WorkloadLifecycle,
    /// Latest receipt sequence observed for this workload.
    pub sequence: u64,
}

impl WorkloadStatus {
    /// Create a validated workload status projection.
    pub fn new(
        workload_id: WorkloadId,
        lifecycle: WorkloadLifecycle,
        sequence: u64,
    ) -> Result<Self, ExecutionValidationError> {
        if sequence == 0 {
            return Err(ExecutionValidationError::InvalidSequence);
        }
        Ok(Self {
            workload_id,
            lifecycle,
            sequence,
        })
    }
}

/// Safe node status projection suitable for client synchronization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionNodeStatus {
    /// Wire protocol version.
    pub protocol_version: u16,
    /// Node identity.
    pub node_id: ExecutionNodeId,
    /// User-visible node name.
    pub display_name: String,
    /// Current node lifecycle.
    pub lifecycle: ExecutionNodeLifecycle,
    /// Explicit capabilities supported by this node.
    pub capabilities: BTreeSet<ExecutionCapability>,
    /// Safe status projections for workloads on this node.
    pub workloads: Vec<WorkloadStatus>,
    /// Time at which this projection was observed.
    pub observed_at: DateTime<Utc>,
}

impl ExecutionNodeStatus {
    /// Create a node status projection with no workload rows.
    pub fn new<I>(
        node_id: ExecutionNodeId,
        display_name: impl Into<String>,
        lifecycle: ExecutionNodeLifecycle,
        capabilities: I,
    ) -> Result<Self, ExecutionValidationError>
    where
        I: IntoIterator<Item = ExecutionCapability>,
    {
        let status = Self {
            protocol_version: EXECUTION_PROTOCOL_VERSION,
            node_id,
            display_name: display_name.into(),
            lifecycle,
            capabilities: capabilities.into_iter().collect(),
            workloads: Vec::new(),
            observed_at: Utc::now(),
        };
        status.validate()?;
        Ok(status)
    }

    /// Add and validate safe workload status rows to this projection.
    pub fn with_workloads<I>(mut self, workloads: I) -> Result<Self, ExecutionValidationError>
    where
        I: IntoIterator<Item = WorkloadStatus>,
    {
        self.workloads = workloads.into_iter().collect();
        self.validate()?;
        Ok(self)
    }

    /// Validate the public status projection.
    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        if self.protocol_version != EXECUTION_PROTOCOL_VERSION {
            return Err(ExecutionValidationError::UnsupportedProtocolVersion(
                self.protocol_version,
            ));
        }
        validate_text(
            "execution node display name",
            &self.display_name,
            MAX_DISPLAY_NAME_BYTES,
            false,
        )?;
        if self.workloads.iter().any(|workload| workload.sequence == 0) {
            return Err(ExecutionValidationError::InvalidSequence);
        }
        Ok(())
    }

    /// Return the advertised capabilities.
    pub fn capabilities(&self) -> &BTreeSet<ExecutionCapability> {
        &self.capabilities
    }

    /// Return safe workload projections.
    pub fn workloads(&self) -> &[WorkloadStatus] {
        &self.workloads
    }
}

/// Safe error codes that may appear in a receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeErrorCode {
    /// The command could not be decoded or validated.
    InvalidCommand,
    /// The command was not authorized for this node.
    Unauthorized,
    /// The command was already expired.
    Expired,
    /// The command operation is not supported.
    Unsupported,
    /// The requested workload does not exist.
    WorkloadNotFound,
    /// The operation conflicts with current workload state.
    Conflict,
    /// The node or runtime is temporarily unavailable.
    RuntimeUnavailable,
    /// The runtime failed to apply the desired state.
    RuntimeFailed,
    /// The provider-authentication session expired or failed.
    AuthenticationFailed,
}

/// Receipt outcome reported by an execution node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ReceiptOutcome {
    /// The node accepted the command for processing.
    Accepted,
    /// The node is still processing the command.
    Progress,
    /// The command completed successfully.
    Succeeded,
    /// The command completed with a safe failure code.
    Failed {
        /// Safe failure classification.
        error: SafeErrorCode,
    },
    /// The node rejected the command before execution.
    Rejected {
        /// Safe rejection classification.
        error: SafeErrorCode,
    },
}

impl ReceiptOutcome {
    /// Return whether the outcome closes the command lifecycle.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed { .. } | Self::Rejected { .. }
        )
    }

    fn error_code(&self) -> Option<SafeErrorCode> {
        match self {
            Self::Failed { error } | Self::Rejected { error } => Some(*error),
            Self::Accepted | Self::Progress | Self::Succeeded => None,
        }
    }
}

/// Correlated, sequenced result emitted by an execution node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionReceipt {
    /// Wire protocol version.
    pub protocol_version: u16,
    /// Node that produced the receipt.
    pub node_id: ExecutionNodeId,
    /// Command request correlation identity.
    pub request_id: RequestId,
    /// Command idempotency identity.
    pub command_id: CommandId,
    /// Workload affected by the command.
    pub workload_id: WorkloadId,
    /// Monotonically increasing sequence for the workload command stream.
    pub sequence: u64,
    /// Receipt lifecycle outcome.
    pub outcome: ReceiptOutcome,
    /// Time at which the node produced the receipt.
    pub observed_at: DateTime<Utc>,
}

impl ExecutionReceipt {
    /// Construct a receipt correlated to a command envelope.
    pub fn for_command(
        command: &ExecutionCommandEnvelope,
        workload_id: WorkloadId,
        sequence: u64,
        outcome: ReceiptOutcome,
    ) -> Result<Self, ExecutionValidationError> {
        if command.command.workload_id() != &workload_id {
            return Err(ExecutionValidationError::WorkloadMismatch);
        }
        Self::with_correlation(
            command.node_id.clone(),
            command.request_id,
            command.command_id,
            workload_id,
            sequence,
            outcome,
        )
    }

    fn with_correlation(
        node_id: ExecutionNodeId,
        request_id: RequestId,
        command_id: CommandId,
        workload_id: WorkloadId,
        sequence: u64,
        outcome: ReceiptOutcome,
    ) -> Result<Self, ExecutionValidationError> {
        let receipt = Self {
            protocol_version: EXECUTION_PROTOCOL_VERSION,
            node_id,
            request_id,
            command_id,
            workload_id,
            sequence,
            outcome,
            observed_at: Utc::now(),
        };
        receipt.validate()?;
        Ok(receipt)
    }

    /// Validate receipt sequencing and safe outcome/error pairing.
    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        if self.protocol_version != EXECUTION_PROTOCOL_VERSION {
            return Err(ExecutionValidationError::UnsupportedProtocolVersion(
                self.protocol_version,
            ));
        }
        if self.sequence == 0 {
            return Err(ExecutionValidationError::InvalidSequence);
        }
        match (self.outcome.is_terminal(), self.outcome.error_code()) {
            (true, Some(_)) | (true, None) if matches!(self.outcome, ReceiptOutcome::Succeeded) => {
                Ok(())
            }
            (true, Some(_)) => Ok(()),
            (true, None) => Err(ExecutionValidationError::MissingError),
            (false, None) => Ok(()),
            (false, Some(_)) => Err(ExecutionValidationError::UnexpectedError),
        }
    }

    /// Validate that this receipt follows a previously observed receipt.
    pub fn validate_after(&self, previous_sequence: u64) -> Result<(), ExecutionValidationError> {
        self.validate()?;
        if self.sequence <= previous_sequence {
            return Err(ExecutionValidationError::InvalidSequenceOrder {
                previous: previous_sequence,
                current: self.sequence,
            });
        }
        Ok(())
    }

    /// Return whether the receipt is terminal.
    pub fn is_terminal(&self) -> bool {
        self.outcome.is_terminal()
    }

    /// Return the sequence number.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}
