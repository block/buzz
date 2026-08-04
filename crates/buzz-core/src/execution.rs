//! Runtime-neutral execution-node protocol types.
//!
//! This module is deliberately free of transport and runtime concerns. Desktop,
//! `buzz-node`, and future clients share these types at the relay-protocol
//! boundary, while authorization, encryption, persistence, and workload
//! reconciliation remain owned by their respective components.

use chrono::{DateTime, Duration, Utc};
use nostr::hashes::sha256::Hash as Sha256Hash;
use nostr::hashes::Hash;
use nostr::secp256k1::schnorr::Signature;
use nostr::secp256k1::Message;
use nostr::{Keys, PublicKey, SECP256K1};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;
use uuid::Uuid;

/// The first released wire contract for execution-node commands, announcements,
/// and receipts. Earlier values were unreleased development iterations.
pub const EXECUTION_PROTOCOL_VERSION: u16 = 1;

/// The first released launch-contract version carried inside workloads and
/// provider deploy payloads. Evolves independently of
/// [`EXECUTION_PROTOCOL_VERSION`]: the envelope version covers command
/// transport, this version covers the resolved launch configuration.
pub const LAUNCH_SPEC_VERSION: u32 = 1;

/// LLM provider credential and endpoint variables that must never travel
/// inside a workload's launch environment. Credential material remains local
/// to the executing substrate: Desktop reads these from the user's own
/// environment, `buzz-node` forwards them from the node operator's
/// environment. Shared here so the Desktop-side strip and the node-side
/// allowlist cannot drift.
pub const PROVIDER_CREDENTIAL_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_BASE_URL",
    "OPENAI_API_KEY",
    "OPENAI_COMPAT_API_KEY",
    "OPENAI_COMPAT_MODEL",
    "OPENAI_COMPAT_BASE_URL",
    "OPENAI_COMPAT_API",
    "OPENROUTER_API_KEY",
    "OPENROUTER_MODEL",
    "OPENROUTER_BASE_URL",
    "DATABRICKS_HOST",
    "DATABRICKS_TOKEN",
    "DATABRICKS_MODEL",
];

/// The maximum lifetime of a command envelope.
pub const MAX_COMMAND_TTL: Duration = Duration::minutes(15);

const MAX_DISPLAY_NAME_BYTES: usize = 128;
const MAX_PROVIDER_BYTES: usize = 128;
const MAX_RUNTIME_BYTES: usize = 128;
const MAX_SESSION_ID_BYTES: usize = 128;
const MAX_AUTH_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_AUTH_TAG_BYTES: usize = 1024;
const MAX_RELAY_URL_BYTES: usize = 2048;
const MAX_PRIVATE_KEY_BYTES: usize = 128;
const MAX_AGENT_ARGS: usize = 128;
const MAX_AGENT_ARG_BYTES: usize = 4096;
const MAX_AGENT_ARGS_BYTES: usize = 64 * 1024;
const MAX_LAUNCH_COMMAND_BYTES: usize = 1024;
const MAX_LAUNCH_ENV_ENTRIES: usize = 512;
const MAX_LAUNCH_ENV_KEY_BYTES: usize = 256;
// The system prompt and team instructions travel as policy environment
// values, so the per-value cap must fit a full prompt. The total budget
// leaves headroom for real layered user environments on top of that.
const MAX_LAUNCH_ENV_VALUE_BYTES: usize = 64 * 1024;
const MAX_LAUNCH_ENV_TOTAL_BYTES: usize = 512 * 1024;

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
    /// An authentication response was empty.
    #[error("authentication response must not be empty")]
    EmptyAuthenticationResponse,
    /// An owner public key in an execution-node attestation was malformed.
    #[error("execution-node attestation owner identity is invalid")]
    InvalidAttestationOwner,
    /// A managed-agent identity or audience key was malformed.
    #[error("managed-agent identity is invalid")]
    InvalidAgentIdentity,
    /// An execution-node attestation did not verify for the expected node and relay.
    #[error("execution-node attestation is invalid")]
    InvalidAttestation,
    /// A managed-agent launch key was malformed or did not match its public identity.
    #[error("managed-agent launch key does not match its public identity")]
    InvalidAgentKey,
    /// A managed-agent launch contained too many arguments.
    #[error("managed-agent contains too many launch arguments")]
    TooManyAgentArgs,
    /// The launch contract version is not supported by this implementation.
    #[error("unsupported launch contract version: {0}")]
    UnsupportedLaunchVersion(u32),
    /// A launch environment exceeded its entry count or byte budget.
    #[error("launch environment exceeds the protocol limits")]
    LaunchEnvTooLarge,
    /// A launch environment key was empty or contained `=`.
    #[error("launch environment contains an invalid variable name")]
    InvalidLaunchEnvKey,
    /// The launch owner identity was not a canonical Nostr public key.
    #[error("launch owner identity is invalid")]
    InvalidLaunchOwner,
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

fn execution_node_attestation_message(node_id: &ExecutionNodeId, relay_authority: &str) -> Message {
    let preimage = format!(
        "nostr:buzz-execution-node:{}/{}",
        node_id.as_str(),
        relay_authority
    );
    Message::from_digest(Sha256Hash::hash(preimage.as_bytes()).to_byte_array())
}

/// Owner proof binding an execution node to one relay authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionNodeAttestation {
    /// Owner public key that authorized this node.
    pub owner_pubkey: String,
    /// Relay authority this node was paired for.
    pub relay_authority: String,
    /// BIP-340 Schnorr signature over the node and relay authority.
    pub signature: String,
}

impl ExecutionNodeAttestation {
    /// Sign an owner proof for a node and relay authority.
    pub fn sign(
        owner_keys: &Keys,
        node_id: &ExecutionNodeId,
        relay_authority: impl Into<String>,
    ) -> Result<Self, ExecutionValidationError> {
        let relay_authority = relay_authority.into();
        validate_text("relay authority", &relay_authority, 256, false)?;
        let signature = owner_keys.sign_schnorr(&execution_node_attestation_message(
            node_id,
            &relay_authority,
        ));
        Ok(Self {
            owner_pubkey: owner_keys.public_key().to_hex(),
            relay_authority,
            signature: signature.to_string(),
        })
    }

    /// Verify this proof against the expected node, relay, and optional owner.
    pub fn verify(
        &self,
        node_id: &ExecutionNodeId,
        expected_relay_authority: &str,
        expected_owner_pubkey: Option<&str>,
    ) -> Result<(), ExecutionValidationError> {
        if self.relay_authority != expected_relay_authority {
            return Err(ExecutionValidationError::InvalidAttestation);
        }
        if expected_owner_pubkey.is_some_and(|owner| owner != self.owner_pubkey) {
            return Err(ExecutionValidationError::InvalidAttestation);
        }
        let owner = PublicKey::from_hex(&self.owner_pubkey)
            .map_err(|_| ExecutionValidationError::InvalidAttestationOwner)?;
        let signature = Signature::from_str(&self.signature)
            .map_err(|_| ExecutionValidationError::InvalidAttestation)?;
        let xonly = owner
            .xonly()
            .map_err(|_| ExecutionValidationError::InvalidAttestation)?;
        SECP256K1
            .verify_schnorr(
                &signature,
                &execution_node_attestation_message(node_id, &self.relay_authority),
                &xonly,
            )
            .map_err(|_| ExecutionValidationError::InvalidAttestation)
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

    /// Derive a stable workload identity for one managed agent.
    pub fn stable_for_agent(pubkey: &str) -> Result<Self, ExecutionValidationError> {
        let identity = PublicKey::from_hex(pubkey)
            .map_err(|_| ExecutionValidationError::InvalidAgentIdentity)?;
        let digest =
            Sha256Hash::hash(format!("buzz-execution-workload:{}", identity.to_hex()).as_bytes())
                .to_byte_array();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x50;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(Self(Uuid::from_bytes(bytes).to_string()))
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

/// The resolved, substrate-neutral launch contract for one agent body.
///
/// Desktop resolves this once — from the same effective-harness descriptor and
/// policy helpers that drive local spawn — and every remote execution path
/// consumes it as-is: provider deployments receive it as the `launch` block,
/// execution nodes receive it inside the workload. Substrates adapt it to
/// their runtime (resolving command names to host executables or in-image
/// paths) but never reconstruct configuration from runtime identifiers.
///
/// Serialized with snake_case field names because this struct *is* the
/// provider `launch` block consumed outside this repository
/// (`sprout-backend-blox`); its shape may only evolve additively, signalled
/// through `version`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchSpec {
    /// Launch contract version. See [`LAUNCH_SPEC_VERSION`].
    pub version: u32,
    /// Effective agent command name (e.g. `goose`). Substrates resolve it to
    /// an executable; it is never a substrate-local path on the wire.
    pub command: String,
    /// Normalized effective arguments for `command`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Developer MCP command name, when the runtime uses one. Resolved by the
    /// substrate like `command`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_command: Option<String>,
    /// The authoritative layered environment (baked floor → runtime metadata →
    /// harness definition → global → persona → agent). Applied *above*
    /// `policy_env` so user-provided values keep winning, mirroring local
    /// spawn. Reserved identity/transport keys are stripped before this
    /// crosses a protocol boundary and again by the consuming substrate.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Buzz-set policy values (harness contract, prompts, timeouts, audience
    /// gate) applied *below* `env`, preserving power-user override semantics.
    #[serde(default)]
    pub policy_env: BTreeMap<String, String>,
    /// Workspace owner identity the body treats as its operator. Optional to
    /// match the provider wire contract (`deploy-no-owner`): without it or an
    /// NIP-OA attestation the respond-to gate cannot resolve an owner, which
    /// consumers handle rather than this type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pubkey: Option<String>,
}

impl LaunchSpec {
    /// Build and validate a launch contract at the current version.
    pub fn new(
        command: impl Into<String>,
        args: Vec<String>,
        mcp_command: Option<String>,
        env: BTreeMap<String, String>,
        policy_env: BTreeMap<String, String>,
        owner_pubkey: Option<String>,
    ) -> Result<Self, ExecutionValidationError> {
        let launch = Self {
            version: LAUNCH_SPEC_VERSION,
            command: command.into(),
            args,
            mcp_command,
            env,
            policy_env,
            owner_pubkey,
        };
        launch.validate()?;
        Ok(launch)
    }

    /// Validate the launch contract before it crosses a protocol boundary.
    pub fn validate(&self) -> Result<(), ExecutionValidationError> {
        if self.version != LAUNCH_SPEC_VERSION {
            return Err(ExecutionValidationError::UnsupportedLaunchVersion(
                self.version,
            ));
        }
        validate_text(
            "launch command",
            &self.command,
            MAX_LAUNCH_COMMAND_BYTES,
            false,
        )?;
        if self.args.len() > MAX_AGENT_ARGS {
            return Err(ExecutionValidationError::TooManyAgentArgs);
        }
        let mut total_arg_bytes = 0;
        for argument in &self.args {
            validate_text("launch argument", argument, MAX_AGENT_ARG_BYTES, true)?;
            total_arg_bytes += argument.len();
        }
        if total_arg_bytes > MAX_AGENT_ARGS_BYTES {
            return Err(ExecutionValidationError::TooManyAgentArgs);
        }
        if let Some(mcp_command) = &self.mcp_command {
            validate_text(
                "launch MCP command",
                mcp_command,
                MAX_LAUNCH_COMMAND_BYTES,
                false,
            )?;
        }
        validate_launch_env(&self.env)?;
        validate_launch_env(&self.policy_env)?;
        // Same canonical-hex check as `ExecutionNodeId`: the owner key is an
        // identity reference, not a signing input, so hex shape is the
        // protocol invariant (curve validity is the verifier's concern).
        if let Some(owner_pubkey) = &self.owner_pubkey {
            if owner_pubkey.len() != 64
                || !owner_pubkey.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(ExecutionValidationError::InvalidLaunchOwner);
            }
        }
        Ok(())
    }

    /// Return the contract with provider credential variables removed.
    ///
    /// Execution nodes receive credentials from the node operator's own
    /// environment ([`PROVIDER_CREDENTIAL_ENV`]), never through workload
    /// configuration. Desktop strips them at the node boundary so they are
    /// neither transmitted nor persisted in the node's workload ledger.
    pub fn without_provider_credentials(mut self) -> Self {
        for name in PROVIDER_CREDENTIAL_ENV {
            self.env.remove(*name);
            self.policy_env.remove(*name);
        }
        self
    }
}

fn validate_launch_env(env: &BTreeMap<String, String>) -> Result<(), ExecutionValidationError> {
    if env.len() > MAX_LAUNCH_ENV_ENTRIES {
        return Err(ExecutionValidationError::LaunchEnvTooLarge);
    }
    let mut total_bytes = 0;
    for (key, value) in env {
        if key.is_empty() || key.contains('=') {
            return Err(ExecutionValidationError::InvalidLaunchEnvKey);
        }
        validate_text(
            "launch environment key",
            key,
            MAX_LAUNCH_ENV_KEY_BYTES,
            false,
        )?;
        // Values are deliberately looser than other protocol text: real user
        // environments legitimately carry control characters (ANSI sequences,
        // embedded escapes), and local spawn has always passed them through.
        // Only NUL — which cannot cross an environment boundary at all — and
        // the size budget are protocol invariants.
        if value.contains('\0') {
            return Err(ExecutionValidationError::NulByte {
                field: "launch environment value",
            });
        }
        if value.len() > MAX_LAUNCH_ENV_VALUE_BYTES {
            return Err(ExecutionValidationError::TooLong {
                field: "launch environment value",
                max: MAX_LAUNCH_ENV_VALUE_BYTES,
            });
        }
        total_bytes += key.len() + value.len();
    }
    if total_bytes > MAX_LAUNCH_ENV_TOTAL_BYTES {
        return Err(ExecutionValidationError::LaunchEnvTooLarge);
    }
    Ok(())
}

/// Identity context for a managed agent workload.
///
/// This carries only what the workload's launch contract cannot: the agent's
/// identity (with the key as an encrypted one-time launch handoff), the relay
/// binding, the NIP-OA authorization, and the deployment's channel context.
/// Behavior configuration — prompt, audience gate, timeouts, parallelism —
/// travels in the workload's [`LaunchSpec`] so every execution body consumes
/// one resolved contract. Process-launch details remain node-local.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentWorkloadContext {
    /// Public identity of the managed agent.
    pub pubkey: String,
    /// Private identity key handed over for this one encrypted launch payload.
    /// The node must move it into its secure provider secret before persisting
    /// workload state; it is not part of the durable Desktop record projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key_nsec: Option<String>,
    /// Relay configuration the managed agent should use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_url: Option<String>,
    /// NIP-OA profile authorization for the managed agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_tag: Option<String>,
    /// Channel context selected for this deployment, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
}

impl fmt::Debug for AgentWorkloadContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentWorkloadContext")
            .field("pubkey", &self.pubkey)
            .field(
                "private_key_nsec",
                &self.private_key_nsec.as_ref().map(|_| "[redacted]"),
            )
            .field("relay_url", &self.relay_url)
            .field("auth_tag", &self.auth_tag.as_ref().map(|_| "[redacted]"))
            .field("channel_id", &self.channel_id)
            .finish()
    }
}

impl AgentWorkloadContext {
    /// Build and validate the managed-agent context carried by a workload.
    pub fn new(
        pubkey: impl Into<String>,
        relay_url: Option<String>,
        auth_tag: Option<String>,
        channel_id: Option<String>,
    ) -> Result<Self, ExecutionValidationError> {
        let context = Self {
            pubkey: pubkey.into(),
            private_key_nsec: None,
            relay_url,
            auth_tag,
            channel_id,
        };
        context.validate()?;
        Ok(context)
    }

    /// Attach and validate the private key required to launch this identity.
    pub fn with_private_key(
        mut self,
        private_key_nsec: impl Into<String>,
    ) -> Result<Self, ExecutionValidationError> {
        self.private_key_nsec = Some(private_key_nsec.into());
        self.validate()?;
        Ok(self)
    }

    /// Return the context safe to retain in the fake node's durable state.
    pub fn without_private_key(mut self) -> Self {
        self.private_key_nsec = None;
        self
    }

    fn validate(&self) -> Result<(), ExecutionValidationError> {
        PublicKey::from_hex(&self.pubkey)
            .map_err(|_| ExecutionValidationError::InvalidAgentIdentity)?;
        if let Some(private_key_nsec) = &self.private_key_nsec {
            validate_text(
                "managed-agent private key",
                private_key_nsec,
                MAX_PRIVATE_KEY_BYTES,
                false,
            )?;
            let keys = Keys::parse(private_key_nsec)
                .map_err(|_| ExecutionValidationError::InvalidAgentKey)?;
            if !keys
                .public_key()
                .to_hex()
                .eq_ignore_ascii_case(&self.pubkey)
            {
                return Err(ExecutionValidationError::InvalidAgentKey);
            }
        }
        if let Some(relay_url) = &self.relay_url {
            validate_text("workload relay URL", relay_url, MAX_RELAY_URL_BYTES, false)?;
        }
        if let Some(auth_tag) = &self.auth_tag {
            validate_text("workload auth tag", auth_tag, MAX_AUTH_TAG_BYTES, false)?;
        }
        if let Some(channel_id) = &self.channel_id {
            validate_text(
                "workload channel ID",
                channel_id,
                MAX_SESSION_ID_BYTES,
                false,
            )?;
        }
        Ok(())
    }
}

/// Workload projection sent inside an encrypted deploy command.
///
/// Runtime-specific infrastructure such as Docker images, sockets, container
/// contexts, and raw provider credentials is intentionally absent. A managed
/// agent's launch key is carried only inside the encrypted `agent` context and
/// must be removed before the node persists its durable workload projection.
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
    /// The resolved launch contract this workload's body runs. `runtime`,
    /// `model`, and `provider` above remain for display and substrate image
    /// selection; the launch contract is the source of execution truth.
    pub launch: LaunchSpec,
    /// The managed-agent contract, when this workload represents an agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentWorkloadContext>,
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
        launch: LaunchSpec,
    ) -> Result<Self, ExecutionValidationError> {
        let workload = Self {
            workload_id,
            display_name: display_name.into(),
            runtime: runtime.into(),
            model,
            provider,
            credential_refs,
            launch,
            agent: None,
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
        self.launch.validate()?;
        if let Some(agent) = &self.agent {
            agent.validate()?;
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

    /// Return the safe projection retained by the fake runtime after launch.
    pub fn without_private_key(mut self) -> Self {
        if let Some(agent) = self.agent.take() {
            self.agent = Some(agent.without_private_key());
        }
        self
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

/// Encrypted provider-authentication response sent only to the paired node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderAuthResponse {
    /// Workload whose provider is being authenticated.
    pub workload_id: WorkloadId,
    /// Session receiving the provider response.
    pub session_id: String,
    /// Provider response or subscription login material.
    pub response: String,
}

impl ProviderAuthResponse {
    /// Construct an authentication response that remains inside the encrypted command.
    pub fn new(
        workload_id: WorkloadId,
        session_id: impl Into<String>,
        response: impl Into<String>,
    ) -> Result<Self, ExecutionValidationError> {
        let response = Self {
            workload_id,
            session_id: session_id.into(),
            response: response.into(),
        };
        validate_text(
            "provider auth session",
            &response.session_id,
            MAX_SESSION_ID_BYTES,
            false,
        )?;
        validate_auth_response(&response.response)?;
        Ok(response)
    }
}

fn validate_auth_response(value: &str) -> Result<(), ExecutionValidationError> {
    if value.is_empty() {
        return Err(ExecutionValidationError::EmptyAuthenticationResponse);
    }
    validate_text(
        "provider auth response",
        value,
        MAX_AUTH_RESPONSE_BYTES,
        false,
    )
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
        workload: Box<WorkloadSpec>,
        /// Highest node-assigned receipt sequence the owner observed for this
        /// workload before issuing the deploy.
        ///
        /// Receipt sequences are assigned by the node and increase strictly
        /// per owner and workload, so a deploy carrying a sequence at or above
        /// a removal's receipt sequence proves it was issued after the owner
        /// observed that removal — it cannot be a stale replay from before the
        /// removal. Nodes use this to clear removal tombstones on deliberate
        /// redeploys while still rejecting stale deploy commands.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        supersedes_removal: Option<u64>,
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
    /// Submit a provider-authentication response to an active session.
    SubmitProviderAuthentication {
        /// Encrypted provider response.
        response: ProviderAuthResponse,
    },
    /// Cancel a provider-authentication session.
    CancelProviderAuthentication {
        /// Workload owning the session.
        workload_id: WorkloadId,
        /// Session to cancel.
        session_id: String,
    },
}

impl ExecutionCommand {
    /// Return the workload targeted by this command.
    pub fn workload_id(&self) -> &WorkloadId {
        match self {
            Self::Deploy { workload, .. } => &workload.workload_id,
            Self::Start { workload_id }
            | Self::Stop { workload_id }
            | Self::Restart { workload_id }
            | Self::Remove { workload_id } => workload_id,
            Self::AuthenticateProvider { session } => &session.workload_id,
            Self::SubmitProviderAuthentication { response } => &response.workload_id,
            Self::CancelProviderAuthentication { workload_id, .. } => workload_id,
        }
    }

    /// Validate command payload fields at a supplied clock instant.
    pub fn validate_at(&self, now: DateTime<Utc>) -> Result<(), ExecutionValidationError> {
        match self {
            Self::Deploy { workload, .. } => workload.validate(),
            Self::Start { .. } | Self::Stop { .. } | Self::Restart { .. } | Self::Remove { .. } => {
                Ok(())
            }
            Self::AuthenticateProvider { session } => session.validate_at(now),
            Self::SubmitProviderAuthentication { response } => {
                validate_text(
                    "provider auth session",
                    &response.session_id,
                    MAX_SESSION_ID_BYTES,
                    false,
                )?;
                validate_auth_response(&response.response)
            }
            Self::CancelProviderAuthentication { session_id, .. } => validate_text(
                "provider auth session",
                session_id,
                MAX_SESSION_ID_BYTES,
                false,
            ),
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
    /// Owner proofs binding this node to a relay authority.
    #[serde(default)]
    pub owner_attestations: Vec<ExecutionNodeAttestation>,
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
            owner_attestations: Vec::new(),
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

    /// Add owner proofs to this public status projection.
    pub fn with_owner_attestations<I>(
        mut self,
        attestations: I,
    ) -> Result<Self, ExecutionValidationError>
    where
        I: IntoIterator<Item = ExecutionNodeAttestation>,
    {
        self.owner_attestations = attestations.into_iter().collect();
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
    /// The provider-authentication session was cancelled.
    AuthenticationCancelled,
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

/// Safe, non-secret detail attached to an execution receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "detail", rename_all = "snake_case")]
pub enum ReceiptDetail {
    /// Actionable provider-authentication challenge for Desktop.
    ProviderAuthChallenge {
        /// Provider namespace requiring authentication.
        provider: String,
        /// Session to include in the encrypted response.
        #[serde(rename = "sessionId")]
        session_id: String,
        /// Safe instructions that contain no credential material.
        instructions: String,
    },
    /// Provider authentication completed successfully.
    ProviderAuthenticated {
        /// Provider namespace that was authenticated.
        provider: String,
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
    /// Optional safe detail for actionable non-secret state.
    #[serde(default)]
    pub detail: Option<ReceiptDetail>,
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
        Self::for_command_with_detail(command, workload_id, sequence, outcome, None)
    }

    /// Construct a receipt with optional safe, non-secret detail.
    pub fn for_command_with_detail(
        command: &ExecutionCommandEnvelope,
        workload_id: WorkloadId,
        sequence: u64,
        outcome: ReceiptOutcome,
        detail: Option<ReceiptDetail>,
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
            detail,
        )
    }

    fn with_correlation(
        node_id: ExecutionNodeId,
        request_id: RequestId,
        command_id: CommandId,
        workload_id: WorkloadId,
        sequence: u64,
        outcome: ReceiptOutcome,
        detail: Option<ReceiptDetail>,
    ) -> Result<Self, ExecutionValidationError> {
        let receipt = Self {
            protocol_version: EXECUTION_PROTOCOL_VERSION,
            node_id,
            request_id,
            command_id,
            workload_id,
            sequence,
            outcome,
            detail,
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
