//! Runtime-neutral relay client for a standalone Buzz execution node.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::fs::File;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_CONCURRENT_COMMANDS: usize = 1024;

use buzz_core::execution::{
    ExecutionCapability, ExecutionCommand, ExecutionCommandEnvelope, ExecutionNodeAttestation,
    ExecutionNodeId, ExecutionNodeLifecycle, ExecutionNodeStatus, ExecutionReceipt,
    ProviderAuthResponse, ReceiptDetail, ReceiptOutcome, SafeErrorCode, WorkloadId,
    WorkloadLifecycle, WorkloadSpec, WorkloadStatus, EXECUTION_PROTOCOL_VERSION,
};
use buzz_core::kind::{
    KIND_EXECUTION_NODE_ANNOUNCEMENT, KIND_EXECUTION_NODE_COMMAND, KIND_EXECUTION_NODE_RECEIPT,
};
use chrono::{DateTime, Utc};
use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, PublicKey, Tag, ToBech32};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::{Mutex, Semaphore};

/// Environment-driven configuration for a node process.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Relay WebSocket URL.
    pub relay_url: String,
    /// Durable node data directory.
    pub data_dir: PathBuf,
    /// Safe display name shown to paired clients.
    pub display_name: String,
    /// Optional NIP-OA tag used when authenticating to the relay.
    pub auth_tag: Option<Tag>,
    /// Local HTTP address used for process health and relay readiness probes.
    pub health_addr: SocketAddr,
    /// Maximum number of encrypted commands processed concurrently.
    pub max_concurrent_commands: usize,
}

impl NodeConfig {
    /// Load deployment-neutral configuration from environment variables.
    pub fn from_env() -> Result<Self, NodeError> {
        let relay_url = required_env("BUZZ_RELAY_URL")?;
        let data_dir = std::env::var_os("BUZZ_NODE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".buzz-node"));
        let display_name = std::env::var("BUZZ_NODE_NAME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "Buzz execution node".to_string());
        let auth_tag = std::env::var("BUZZ_AUTH_TAG")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| parse_auth_tag(&value))
            .transpose()?;
        let health_addr = std::env::var("BUZZ_NODE_HEALTH_ADDR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                value.parse().map_err(|error| {
                    NodeError::InvalidConfiguration(format!("BUZZ_NODE_HEALTH_ADDR: {error}"))
                })
            })
            .transpose()?
            .unwrap_or(SocketAddr::from(([127, 0, 0, 1], 8081)));
        let max_concurrent_commands = std::env::var("BUZZ_NODE_MAX_CONCURRENT_COMMANDS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                value.parse::<usize>().map_err(|error| {
                    NodeError::InvalidConfiguration(format!(
                        "BUZZ_NODE_MAX_CONCURRENT_COMMANDS must be a positive integer: {error}"
                    ))
                })
            })
            .transpose()?
            .unwrap_or(8);
        if max_concurrent_commands == 0 || max_concurrent_commands > MAX_CONCURRENT_COMMANDS {
            return Err(NodeError::InvalidConfiguration(format!(
                "BUZZ_NODE_MAX_CONCURRENT_COMMANDS must be between 1 and {MAX_CONCURRENT_COMMANDS}"
            )));
        }

        Ok(Self {
            relay_url,
            data_dir,
            display_name,
            auth_tag,
            health_addr,
            max_concurrent_commands,
        })
    }
}

/// Errors produced by node configuration and durable identity operations.
#[derive(Debug, Error)]
pub enum NodeError {
    /// A required configuration variable was absent.
    #[error("missing required environment variable: {0}")]
    MissingEnvironment(String),
    /// A configured value was malformed.
    #[error("invalid node configuration: {0}")]
    InvalidConfiguration(String),
    /// Filesystem access failed.
    #[error("node storage error: {0}")]
    Storage(#[from] std::io::Error),
    /// Nostr key parsing or encoding failed.
    #[error("node identity error: {0}")]
    Identity(String),
    /// JSON serialization failed.
    #[error("node data error: {0}")]
    Json(#[from] serde_json::Error),
    /// Pairing payload did not identify an owner.
    #[error("pairing payload error: {0}")]
    PairingPayload(String),
    /// An execution command could not be safely processed.
    #[error("invalid execution command: {0}")]
    InvalidCommand(String),
    /// NIP-44 encryption or decryption failed.
    #[error("execution encryption error: {0}")]
    Encryption(String),
}

/// Persistent Nostr identity owned by one execution node.
#[derive(Debug, Clone)]
pub struct NodeIdentity {
    /// Nostr keypair used for relay authentication and node events.
    pub keys: Keys,
    /// File containing the nsec representation.
    pub path: PathBuf,
}

impl NodeIdentity {
    /// Load an existing identity or create it once in `data_dir`.
    pub fn load_or_create(data_dir: &Path) -> Result<Self, NodeError> {
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join("identity.nsec");
        let keys = if path.exists() {
            let encoded = fs::read_to_string(&path)?;
            Keys::parse(encoded.trim()).map_err(|error| NodeError::Identity(error.to_string()))?
        } else {
            let keys = Keys::generate();
            let encoded = keys
                .secret_key()
                .to_bech32()
                .map_err(|error| NodeError::Identity(error.to_string()))?;
            fs::write(&path, format!("{encoded}\n"))?;
            set_private_file_permissions(&path)?;
            keys
        };

        Ok(Self { keys, path })
    }

    /// Return the stable execution-node identity used in shared protocol data.
    pub fn node_id(&self) -> Result<ExecutionNodeId, NodeError> {
        ExecutionNodeId::new(self.keys.public_key().to_hex())
            .map_err(|error| NodeError::Identity(error.to_string()))
    }
}

/// Durable owner allowlist established through NIP-AB pairing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OwnerStore {
    owners: Vec<String>,
    #[serde(default)]
    attestations: Vec<ExecutionNodeAttestation>,
}

impl OwnerStore {
    /// Load the allowlist, treating a missing file as an empty allowlist.
    pub fn load(data_dir: &Path) -> Result<Self, NodeError> {
        let path = data_dir.join("owners.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    /// Add an owner public key and persist the updated allowlist.
    pub fn add(&mut self, owner: &str, data_dir: &Path) -> Result<(), NodeError> {
        let key = nostr::PublicKey::from_hex(owner)
            .map_err(|error| NodeError::PairingPayload(error.to_string()))?;
        let owner = key.to_hex();
        if !self.owners.iter().any(|existing| existing == &owner) {
            self.owners.push(owner);
            self.owners.sort_unstable();
            fs::create_dir_all(data_dir)?;
            let path = data_dir.join("owners.json");
            fs::write(&path, serde_json::to_string_pretty(self)?)?;
            set_private_file_permissions(&path)?;
        }
        Ok(())
    }

    /// Add an owner proof and persist the pairing binding for this relay.
    pub fn add_attestation(
        &mut self,
        attestation: ExecutionNodeAttestation,
        node_id: &ExecutionNodeId,
        relay_authority: &str,
        data_dir: &Path,
    ) -> Result<(), NodeError> {
        attestation
            .verify(node_id, relay_authority, None)
            .map_err(|error| NodeError::PairingPayload(error.to_string()))?;
        let owner = nostr::PublicKey::from_hex(&attestation.owner_pubkey)
            .map_err(|error| NodeError::PairingPayload(error.to_string()))?
            .to_hex();
        if !self.owners.iter().any(|existing| existing == &owner) {
            self.owners.push(owner);
            self.owners.sort_unstable();
        }
        if !self.attestations.contains(&attestation) {
            self.attestations.push(attestation);
            self.attestations.sort_by(|left, right| {
                left.owner_pubkey
                    .cmp(&right.owner_pubkey)
                    .then(left.relay_authority.cmp(&right.relay_authority))
                    .then(left.signature.cmp(&right.signature))
            });
        }
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join("owners.json");
        fs::write(&path, serde_json::to_string_pretty(self)?)?;
        set_private_file_permissions(&path)?;
        Ok(())
    }

    /// Check whether an owner is paired to this node on the active relay.
    ///
    /// The legacy owner list is retained for on-disk compatibility, but it is
    /// not sufficient for command authorization: the attestation must bind
    /// the owner, this node, and the relay the process is currently connected
    /// to.
    pub fn contains_for_relay(
        &self,
        owner: &str,
        node_id: &ExecutionNodeId,
        relay_authority: &str,
    ) -> bool {
        self.attestations.iter().any(|attestation| {
            attestation.owner_pubkey == owner
                && attestation
                    .verify(node_id, relay_authority, Some(owner))
                    .is_ok()
        })
    }

    /// Return paired owner identities without exposing any private material.
    pub fn owners(&self) -> &[String] {
        &self.owners
    }

    /// Return owner proofs included in node announcements.
    pub fn attestations(&self) -> &[ExecutionNodeAttestation] {
        &self.attestations
    }
}

/// Build the safe replaceable announcement published by a node.
pub fn build_announcement(identity: &NodeIdentity, display_name: &str) -> Result<Event, NodeError> {
    build_announcement_with_workloads(identity, display_name, &[])
}

/// Build a replaceable announcement including the node's durable workload view.
pub fn build_announcement_with_workloads(
    identity: &NodeIdentity,
    display_name: &str,
    workloads: &[buzz_core::execution::WorkloadStatus],
) -> Result<Event, NodeError> {
    build_announcement_with_workloads_and_attestations(identity, display_name, workloads, &[])
}

/// Build an announcement including durable workload state and pairing proofs.
pub fn build_announcement_with_workloads_and_attestations(
    identity: &NodeIdentity,
    display_name: &str,
    workloads: &[buzz_core::execution::WorkloadStatus],
    attestations: &[ExecutionNodeAttestation],
) -> Result<Event, NodeError> {
    let node_id = identity.node_id()?;
    let status = ExecutionNodeStatus::new(
        node_id.clone(),
        display_name,
        ExecutionNodeLifecycle::Ready,
        [
            ExecutionCapability::Deploy,
            ExecutionCapability::Start,
            ExecutionCapability::Stop,
            ExecutionCapability::Restart,
            ExecutionCapability::Remove,
            ExecutionCapability::ProviderAuthentication,
        ],
    )?;
    let status = status.with_owner_attestations(attestations.iter().cloned())?;
    let status = status
        .with_workloads(workloads.iter().cloned())
        .map_err(|error| NodeError::InvalidCommand(error.to_string()))?;
    let content = serde_json::to_string(&status)?;
    let d_tag = Tag::parse(["d", node_id.as_str()])
        .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
    EventBuilder::new(
        Kind::Custom(KIND_EXECUTION_NODE_ANNOUNCEMENT as u16),
        content,
    )
    .tags([d_tag])
    .sign_with_keys(&identity.keys)
    .map_err(|error| NodeError::Identity(error.to_string()))
}

/// Payload sent by the existing Desktop NIP-AB pairing flow.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopPairingPayload {
    /// Workspace owner identity. The legacy `pubkey` name is accepted too.
    #[serde(alias = "pubkey")]
    pub owner_pubkey: String,
    /// Relay to use after pairing.
    pub relay_url: String,
    /// Owner key transferred by the existing SAS-protected pairing flow. It
    /// is used only to sign the node binding and is never persisted.
    #[serde(default, skip_serializing)]
    pub nsec: Option<String>,
}

impl fmt::Debug for DesktopPairingPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesktopPairingPayload")
            .field("owner_pubkey", &self.owner_pubkey)
            .field("relay_url", &self.relay_url)
            .field("nsec", &self.nsec.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

/// Parse and validate a Desktop pairing payload. The caller must consume the
/// transient private key and zeroize it after deriving the node attestation.
pub fn parse_desktop_pairing_payload(payload: &str) -> Result<DesktopPairingPayload, NodeError> {
    let parsed: DesktopPairingPayload = serde_json::from_str(payload)?;
    nostr::PublicKey::from_hex(&parsed.owner_pubkey)
        .map_err(|error| NodeError::PairingPayload(error.to_string()))?;
    if parsed.relay_url.trim().is_empty() {
        return Err(NodeError::PairingPayload(
            "relayUrl must not be empty".into(),
        ));
    }
    Ok(parsed)
}

/// Minimal fake runtime used to prove the relay-native execution contract.
///
/// The runtime deliberately stores only safe workload specifications. It does
/// not launch a process or retain credential material; a later ticket can swap
/// this implementation for a durable provider-backed runtime without changing
/// the command and receipt protocol.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FakeWorkloadRuntime {
    workloads: BTreeMap<WorkloadId, RuntimeWorkload>,
    removed_workloads: BTreeSet<WorkloadId>,
    deploy_invocations: usize,
}

/// Encrypted local state proving a provider subscription has been authenticated.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct FakeCredentialStore {
    authenticated: Vec<StoredCredentialState>,
    pending_sessions: Vec<PendingAuthSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCredentialState {
    workload_id: WorkloadId,
    provider: String,
    encrypted_response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingAuthSession {
    workload_id: WorkloadId,
    session_id: String,
    provider: String,
    expires_at: DateTime<Utc>,
}

impl FakeCredentialStore {
    fn begin(
        &mut self,
        session: &buzz_core::execution::ProviderAuthSession,
    ) -> Result<ReceiptDetail, SafeErrorCode> {
        self.pending_sessions.retain(|pending| {
            pending.workload_id != session.workload_id || pending.session_id != session.session_id
        });
        self.pending_sessions.push(PendingAuthSession {
            workload_id: session.workload_id.clone(),
            session_id: session.session_id.clone(),
            provider: session.provider.clone(),
            expires_at: session.expires_at,
        });
        Ok(ReceiptDetail::ProviderAuthChallenge {
            provider: session.provider.clone(),
            session_id: session.session_id.clone(),
            instructions:
                "Complete the provider subscription login, then submit the response from Desktop."
                    .into(),
        })
    }

    fn submit(
        &mut self,
        response: &ProviderAuthResponse,
        now: DateTime<Utc>,
        node_keys: &Keys,
    ) -> Result<ReceiptDetail, SafeErrorCode> {
        let pending_index = self
            .pending_sessions
            .iter()
            .position(|pending| {
                pending.workload_id == response.workload_id
                    && pending.session_id == response.session_id
            })
            .ok_or(SafeErrorCode::AuthenticationFailed)?;
        let pending = self.pending_sessions.remove(pending_index);
        if pending.expires_at <= now {
            return Err(SafeErrorCode::AuthenticationFailed);
        }
        if !self.authenticated.iter().any(|stored| {
            stored.workload_id == response.workload_id && stored.provider == pending.provider
        }) {
            let encrypted_response = nip44::encrypt(
                node_keys.secret_key(),
                &node_keys.public_key(),
                &response.response,
                nip44::Version::V2,
            )
            .map_err(|_| SafeErrorCode::RuntimeFailed)?;
            self.authenticated.push(StoredCredentialState {
                workload_id: response.workload_id.clone(),
                provider: pending.provider.clone(),
                encrypted_response,
            });
        }
        Ok(ReceiptDetail::ProviderAuthenticated {
            provider: pending.provider,
        })
    }

    fn cancel(&mut self, workload_id: &WorkloadId, session_id: &str) -> Result<(), SafeErrorCode> {
        let Some(index) = self.pending_sessions.iter().position(|pending| {
            pending.workload_id == *workload_id && pending.session_id == session_id
        }) else {
            return Err(SafeErrorCode::AuthenticationFailed);
        };
        self.pending_sessions.remove(index);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeWorkload {
    spec: WorkloadSpec,
    lifecycle: WorkloadLifecycle,
}

impl FakeWorkloadRuntime {
    fn without_private_keys(mut self) -> Self {
        for workload in self.workloads.values_mut() {
            workload.spec = workload.spec.clone().without_private_key();
        }
        self
    }

    /// Reconcile a deploy request into the fake runtime.
    pub fn deploy(&mut self, workload: &WorkloadSpec) -> Result<WorkloadLifecycle, SafeErrorCode> {
        if self.removed_workloads.contains(&workload.workload_id) {
            return Err(SafeErrorCode::Conflict);
        }
        let is_new = !self.workloads.contains_key(&workload.workload_id);
        self.workloads.insert(
            workload.workload_id.clone(),
            RuntimeWorkload {
                spec: workload.clone().without_private_key(),
                lifecycle: WorkloadLifecycle::Running,
            },
        );
        if is_new {
            self.deploy_invocations += 1;
        }
        Ok(WorkloadLifecycle::Running)
    }

    /// Start an existing workload.
    pub fn start(&mut self, workload_id: &WorkloadId) -> Result<WorkloadLifecycle, SafeErrorCode> {
        self.transition(workload_id, WorkloadLifecycle::Running)
    }

    /// Stop an existing workload.
    pub fn stop(&mut self, workload_id: &WorkloadId) -> Result<WorkloadLifecycle, SafeErrorCode> {
        self.transition(workload_id, WorkloadLifecycle::Stopped)
    }

    /// Restart an existing workload.
    pub fn restart(
        &mut self,
        workload_id: &WorkloadId,
    ) -> Result<WorkloadLifecycle, SafeErrorCode> {
        self.transition(workload_id, WorkloadLifecycle::Running)
    }

    /// Remove an existing workload.
    pub fn remove(&mut self, workload_id: &WorkloadId) -> Result<WorkloadLifecycle, SafeErrorCode> {
        if self.workloads.remove(workload_id).is_some() {
            self.removed_workloads.insert(workload_id.clone());
            return Ok(WorkloadLifecycle::Removed);
        }
        if self.removed_workloads.contains(workload_id) {
            return Ok(WorkloadLifecycle::Removed);
        }
        Err(SafeErrorCode::WorkloadNotFound)
    }

    fn transition(
        &mut self,
        workload_id: &WorkloadId,
        lifecycle: WorkloadLifecycle,
    ) -> Result<WorkloadLifecycle, SafeErrorCode> {
        let workload = self
            .workloads
            .get_mut(workload_id)
            .ok_or(SafeErrorCode::WorkloadNotFound)?;
        workload.lifecycle = lifecycle;
        Ok(lifecycle)
    }

    /// Number of distinct workload rows currently known to the runtime.
    pub fn workload_count(&self) -> usize {
        self.workloads.len()
    }

    /// Number of first-time deploy reconciliations.
    pub fn deploy_invocations(&self) -> usize {
        self.deploy_invocations
    }

    fn statuses(&self, sequences: &BTreeMap<WorkloadId, u64>) -> Vec<WorkloadStatus> {
        let mut statuses: Vec<_> = self
            .workloads
            .iter()
            .filter_map(|(workload_id, workload)| {
                sequences.get(workload_id).and_then(|sequence| {
                    WorkloadStatus::new(workload_id.clone(), workload.lifecycle, *sequence).ok()
                })
            })
            .collect();
        statuses.extend(self.removed_workloads.iter().filter_map(|workload_id| {
            sequences.get(workload_id).and_then(|sequence| {
                WorkloadStatus::new(workload_id.clone(), WorkloadLifecycle::Removed, *sequence).ok()
            })
        }));
        statuses.sort_by(|left, right| left.workload_id.cmp(&right.workload_id));
        statuses
    }
}

/// Node-side durable command processor for encrypted execution commands.
#[derive(Debug, Clone)]
pub struct ExecutionController {
    state: Arc<Mutex<ControllerState>>,
    workload_locks: Arc<Mutex<HashMap<WorkloadKey, Arc<Mutex<()>>>>>,
    concurrency: Arc<Semaphore>,
    persist_lock: Arc<Mutex<()>>,
}

impl Default for ExecutionController {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default)]
struct ControllerState {
    runtimes: BTreeMap<String, FakeWorkloadRuntime>,
    credentials: BTreeMap<String, FakeCredentialStore>,
    processed: HashMap<JournalKey, ProcessedCommand>,
    conflicts: HashMap<JournalKey, StoredEvents>,
    next_sequences: BTreeMap<String, BTreeMap<WorkloadId, u64>>,
    data_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProcessedCommand {
    fingerprint: String,
    receipts: Vec<ExecutionReceipt>,
    #[serde(default)]
    events: Vec<Event>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEvents {
    receipts: Vec<ExecutionReceipt>,
    events: Vec<Event>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct JournalKey {
    owner: String,
    command_id: buzz_core::execution::CommandId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WorkloadKey {
    owner: String,
    workload_id: WorkloadId,
}

impl ExecutionController {
    /// Create an in-memory controller with bounded concurrent command work.
    pub fn new() -> Self {
        Self::with_concurrency(8)
    }

    /// Create an in-memory controller with an explicit concurrency limit.
    pub fn with_concurrency(limit: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(ControllerState::default())),
            workload_locks: Arc::new(Mutex::new(HashMap::new())),
            concurrency: Arc::new(Semaphore::new(limit.max(1))),
            persist_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Load command idempotency state from a node data directory.
    pub fn load(data_dir: &Path) -> Result<Self, NodeError> {
        Self::load_with_concurrency(data_dir, 8)
    }

    /// Load command state with an explicit bounded command concurrency.
    pub fn load_with_concurrency(data_dir: &Path, limit: usize) -> Result<Self, NodeError> {
        let path = data_dir.join("execution-state.json");
        if !path.exists() {
            let controller = Self::with_concurrency(limit);
            controller
                .state
                .try_lock()
                .map_err(|_| {
                    NodeError::InvalidConfiguration(
                        "new execution controller was unexpectedly locked".into(),
                    )
                })?
                .data_dir = Some(data_dir.to_path_buf());
            return Ok(controller);
        }
        let raw: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
        let state = match serde_json::from_value::<PersistedExecutionState>(raw.clone()) {
            Ok(state) => state,
            Err(_) => {
                let legacy: LegacyPersistedExecutionState = serde_json::from_value(raw)?;
                PersistedExecutionState {
                    runtimes: legacy
                        .runtime
                        .map(|runtime| BTreeMap::from([("legacy".to_string(), runtime)]))
                        .unwrap_or_default(),
                    runtime: None,
                    credentials: BTreeMap::new(),
                    processed: legacy
                        .processed
                        .into_iter()
                        .map(|(command_id, receipts)| PersistedProcessedCommand {
                            key: JournalKey {
                                owner: "legacy".to_string(),
                                command_id,
                            },
                            value: ProcessedCommand {
                                fingerprint: String::new(),
                                receipts,
                                events: Vec::new(),
                            },
                        })
                        .collect(),
                    conflicts: Vec::new(),
                    next_sequences: BTreeMap::from([("legacy".to_string(), legacy.next_sequences)]),
                }
            }
        };
        let controller = Self::with_concurrency(limit);
        *controller.state.try_lock().map_err(|_| {
            NodeError::InvalidConfiguration(
                "new execution controller was unexpectedly locked".into(),
            )
        })? = ControllerState {
            runtimes: state.runtimes,
            credentials: state.credentials,
            processed: state
                .processed
                .into_iter()
                .map(|entry| (entry.key, entry.value))
                .collect(),
            conflicts: state
                .conflicts
                .into_iter()
                .map(|entry| (entry.key, entry.value))
                .collect(),
            next_sequences: state.next_sequences,
            data_dir: Some(data_dir.to_path_buf()),
        };
        Ok(controller)
    }

    /// Process one signed, owner-authorized command event and return its signed
    /// encrypted receipt events. Invalid or unauthorized events are ignored by
    /// the caller's subscription loop and never reach the fake runtime.
    pub async fn handle_command_event(
        &self,
        identity: &NodeIdentity,
        owners: &OwnerStore,
        relay_authority: &str,
        event: &Event,
        now: DateTime<Utc>,
    ) -> Result<Vec<Event>, NodeError> {
        if event.kind.as_u16() as u32 != KIND_EXECUTION_NODE_COMMAND {
            return Ok(Vec::new());
        }
        if !event.verify_id() || !event.verify_signature() {
            return Err(NodeError::InvalidCommand(
                "command event signature verification failed".into(),
            ));
        }

        let node_id = identity.node_id()?;
        let owner = event.pubkey;
        let owner_hex = owner.to_hex();
        if !owners.contains_for_relay(&owner_hex, &node_id, relay_authority) {
            return Ok(Vec::new());
        }
        if !has_exact_p_tag(event, node_id.as_str()) {
            return Err(NodeError::InvalidCommand(
                "command must have exactly one p tag for this node".into(),
            ));
        }

        let plaintext = nip44::decrypt(identity.keys.secret_key(), &owner, &event.content)
            .map_err(|error| NodeError::Encryption(error.to_string()))?;
        let envelope: ExecutionCommandEnvelope =
            serde_json::from_str(&plaintext).map_err(|error| {
                NodeError::InvalidCommand(format!("invalid envelope JSON: {error}"))
            })?;

        let journal_key = JournalKey {
            owner: owner_hex,
            command_id: envelope.command_id(),
        };
        let fingerprint = command_fingerprint(&envelope)?;
        {
            let state = self.state.lock().await;
            if let Some(previous) = state.processed.get(&journal_key) {
                if previous.fingerprint == fingerprint {
                    if !previous.events.is_empty() {
                        return Ok(previous.events.clone());
                    }
                    return self.receipt_events(identity, &owner, previous.receipts.clone());
                }
                if let Some(conflict) = state.conflicts.get(&journal_key) {
                    if !conflict.events.is_empty() {
                        return Ok(conflict.events.clone());
                    }
                    return self.receipt_events(identity, &owner, conflict.receipts.clone());
                }
            }
        }

        if self.state.lock().await.processed.contains_key(&journal_key) {
            let mut state = self.state.lock().await;
            let receipts = vec![next_receipt(
                &mut state,
                &journal_key.owner,
                &envelope,
                ReceiptOutcome::Rejected {
                    error: SafeErrorCode::Conflict,
                },
            )?];
            let events = self.receipt_events(identity, &owner, receipts.clone())?;
            state.conflicts.insert(
                journal_key.clone(),
                StoredEvents {
                    receipts,
                    events: events.clone(),
                },
            );
            drop(state);
            self.persist_current_state().await?;
            return Ok(events);
        }

        let workload_lock = {
            let mut locks = self.workload_locks.lock().await;
            let workload_key = WorkloadKey {
                owner: journal_key.owner.clone(),
                workload_id: envelope.command.workload_id().clone(),
            };
            locks
                .entry(workload_key)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _workload_guard = workload_lock.lock().await;
        let _concurrency_guard = self
            .concurrency
            .acquire()
            .await
            .map_err(|_| NodeError::InvalidCommand("execution controller is closed".into()))?;

        // The runtime seam is asynchronous even for the fake runtime. Yielding
        // here lets commands for different workloads make progress concurrently
        // while the per-workload guard keeps same-workload ordering strict.
        tokio::task::yield_now().await;
        let mut state = self.state.lock().await;
        if let Some(previous) = state.processed.get(&journal_key) {
            if previous.fingerprint == fingerprint {
                if !previous.events.is_empty() {
                    let events = previous.events.clone();
                    drop(state);
                    return Ok(events);
                }
                let receipts = previous.receipts.clone();
                drop(state);
                return self.receipt_events(identity, &owner, receipts);
            }
            if let Some(conflict) = state.conflicts.get(&journal_key) {
                if !conflict.events.is_empty() {
                    let events = conflict.events.clone();
                    drop(state);
                    return Ok(events);
                }
                let receipts = conflict.receipts.clone();
                drop(state);
                return self.receipt_events(identity, &owner, receipts);
            }
            let receipts = vec![next_receipt(
                &mut state,
                &journal_key.owner,
                &envelope,
                ReceiptOutcome::Rejected {
                    error: SafeErrorCode::Conflict,
                },
            )?];
            let events = self.receipt_events(identity, &owner, receipts.clone())?;
            state.conflicts.insert(
                journal_key.clone(),
                StoredEvents {
                    receipts,
                    events: events.clone(),
                },
            );
            drop(state);
            self.persist_current_state().await?;
            return Ok(events);
        }

        let receipts = if envelope.node_id() != &node_id {
            vec![next_receipt(
                &mut state,
                &journal_key.owner,
                &envelope,
                ReceiptOutcome::Rejected {
                    error: SafeErrorCode::Unauthorized,
                },
            )?]
        } else if let Err(error) = envelope.validate_at(now) {
            vec![next_receipt(
                &mut state,
                &journal_key.owner,
                &envelope,
                ReceiptOutcome::Rejected {
                    error: safe_error_for_validation(&error),
                },
            )?]
        } else {
            execute(
                &mut state,
                &journal_key.owner,
                &envelope,
                now,
                &identity.keys,
            )?
        };

        let events = self.receipt_events(identity, &owner, receipts.clone())?;
        state.processed.insert(
            journal_key,
            ProcessedCommand {
                fingerprint,
                receipts,
                events: events.clone(),
            },
        );
        drop(state);
        self.persist_current_state().await?;
        Ok(events)
    }

    async fn persist_current_state(&self) -> Result<(), NodeError> {
        let _persist_guard = self.persist_lock.lock().await;
        let state = self.state.lock().await;
        let snapshot = persisted_state(&state);
        let data_dir = state.data_dir.clone();
        drop(state);
        persist_snapshot(&snapshot, data_dir.as_deref())
    }

    /// Inspect the fake runtime for diagnostics and tests.
    pub async fn runtime(&self) -> FakeWorkloadRuntime {
        let state = self.state.lock().await;
        state.runtimes.values().next().cloned().unwrap_or_default()
    }

    /// Return the current durable workload projection for node announcements.
    pub async fn workload_statuses(&self) -> Vec<buzz_core::execution::WorkloadStatus> {
        let state = self.state.lock().await;
        state
            .runtimes
            .iter()
            .flat_map(|(owner, runtime)| {
                runtime.statuses(state.next_sequences.get(owner).unwrap_or(&BTreeMap::new()))
            })
            .collect()
    }

    fn receipt_events(
        &self,
        identity: &NodeIdentity,
        owner: &PublicKey,
        receipts: Vec<ExecutionReceipt>,
    ) -> Result<Vec<Event>, NodeError> {
        receipts
            .into_iter()
            .map(|receipt| {
                let plaintext = serde_json::to_string(&receipt)?;
                let encrypted = nip44::encrypt(
                    identity.keys.secret_key(),
                    owner,
                    &plaintext,
                    nip44::Version::V2,
                )
                .map_err(|error| NodeError::Encryption(error.to_string()))?;
                EventBuilder::new(Kind::Custom(KIND_EXECUTION_NODE_RECEIPT as u16), encrypted)
                    .tags([Tag::parse(["p", &owner.to_hex()]).map_err(|error| {
                        NodeError::InvalidCommand(format!("receipt p tag: {error}"))
                    })?])
                    .sign_with_keys(&identity.keys)
                    .map_err(|error| NodeError::Identity(error.to_string()))
            })
            .collect()
    }
}

fn execute(
    state: &mut ControllerState,
    owner: &str,
    envelope: &ExecutionCommandEnvelope,
    now: DateTime<Utc>,
    node_keys: &Keys,
) -> Result<Vec<ExecutionReceipt>, NodeError> {
    let accepted = next_receipt(state, owner, envelope, ReceiptOutcome::Accepted)?;
    let runtime = state.runtimes.entry(owner.to_string()).or_default();
    let (outcome, detail) = match &envelope.command {
        ExecutionCommand::Deploy { workload } => (runtime.deploy(workload), None),
        ExecutionCommand::Start { workload_id } => (runtime.start(workload_id), None),
        ExecutionCommand::Stop { workload_id } => (runtime.stop(workload_id), None),
        ExecutionCommand::Restart { workload_id } => (runtime.restart(workload_id), None),
        ExecutionCommand::Remove { workload_id } => (runtime.remove(workload_id), None),
        ExecutionCommand::AuthenticateProvider { session } => {
            let result = if runtime.workloads.contains_key(&session.workload_id) {
                state
                    .credentials
                    .entry(owner.to_string())
                    .or_default()
                    .begin(session)
            } else {
                Err(SafeErrorCode::WorkloadNotFound)
            };
            match result {
                Ok(detail) => (Ok(WorkloadLifecycle::Pending), Some(detail)),
                Err(error) => (Err(error), None),
            }
        }
        ExecutionCommand::SubmitProviderAuthentication { response } => {
            let result = state
                .credentials
                .entry(owner.to_string())
                .or_default()
                .submit(response, now, node_keys);
            match result {
                Ok(detail) => (Ok(WorkloadLifecycle::Running), Some(detail)),
                Err(error) => (Err(error), None),
            }
        }
        ExecutionCommand::CancelProviderAuthentication {
            workload_id,
            session_id,
        } => {
            let result = state
                .credentials
                .entry(owner.to_string())
                .or_default()
                .cancel(workload_id, session_id)
                .map(|_| WorkloadLifecycle::Stopped);
            (result, None)
        }
    };
    let outcome = match outcome {
        Ok(_)
            if matches!(
                &envelope.command,
                ExecutionCommand::AuthenticateProvider { .. }
            ) =>
        {
            ReceiptOutcome::Progress
        }
        Ok(_) => ReceiptOutcome::Succeeded,
        Err(error) => ReceiptOutcome::Failed { error },
    };
    Ok(vec![
        accepted,
        next_receipt_with_detail(state, owner, envelope, outcome, detail)?,
    ])
}

fn next_receipt(
    state: &mut ControllerState,
    owner: &str,
    envelope: &ExecutionCommandEnvelope,
    outcome: ReceiptOutcome,
) -> Result<ExecutionReceipt, NodeError> {
    next_receipt_with_detail(state, owner, envelope, outcome, None)
}

fn next_receipt_with_detail(
    state: &mut ControllerState,
    owner: &str,
    envelope: &ExecutionCommandEnvelope,
    outcome: ReceiptOutcome,
    detail: Option<ReceiptDetail>,
) -> Result<ExecutionReceipt, NodeError> {
    let workload_id = envelope.command.workload_id().clone();
    let sequence = state
        .next_sequences
        .entry(owner.to_string())
        .or_default()
        .entry(workload_id.clone())
        .or_insert(0);
    *sequence += 1;
    ExecutionReceipt::for_command_with_detail(envelope, workload_id, *sequence, outcome, detail)
        .map_err(|error| NodeError::InvalidCommand(error.to_string()))
}

fn command_fingerprint(envelope: &ExecutionCommandEnvelope) -> Result<String, NodeError> {
    let encoded = serde_json::to_vec(&(envelope.node_id(), &envelope.command))?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

fn persisted_state(state: &ControllerState) -> PersistedExecutionState {
    PersistedExecutionState {
        runtimes: state
            .runtimes
            .iter()
            .map(|(owner, runtime)| (owner.clone(), runtime.clone().without_private_keys()))
            .collect(),
        runtime: None,
        credentials: state.credentials.clone(),
        processed: state
            .processed
            .iter()
            .map(|(key, value)| PersistedProcessedCommand {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
        conflicts: state
            .conflicts
            .iter()
            .map(|(key, value)| PersistedConflict {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
        next_sequences: state.next_sequences.clone(),
    }
}

fn persist_snapshot(
    state: &PersistedExecutionState,
    data_dir: Option<&Path>,
) -> Result<(), NodeError> {
    let Some(data_dir) = data_dir else {
        return Ok(());
    };
    fs::create_dir_all(data_dir)?;
    let path = data_dir.join("execution-state.json");
    let temporary_path = path.with_extension("json.tmp");
    fs::write(&temporary_path, serde_json::to_string_pretty(state)?)?;
    set_private_file_permissions(&temporary_path)?;
    fs::rename(&temporary_path, &path)?;
    File::open(&path)?.sync_all()?;
    File::open(data_dir)?.sync_all()?;
    set_private_file_permissions(&path)
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedExecutionState {
    #[serde(default)]
    runtimes: BTreeMap<String, FakeWorkloadRuntime>,
    #[serde(default)]
    runtime: Option<FakeWorkloadRuntime>,
    #[serde(default)]
    credentials: BTreeMap<String, FakeCredentialStore>,
    processed: Vec<PersistedProcessedCommand>,
    #[serde(default)]
    conflicts: Vec<PersistedConflict>,
    next_sequences: BTreeMap<String, BTreeMap<WorkloadId, u64>>,
}

#[derive(Debug, Deserialize)]
struct LegacyPersistedExecutionState {
    #[serde(default)]
    runtime: Option<FakeWorkloadRuntime>,
    processed: HashMap<buzz_core::execution::CommandId, Vec<ExecutionReceipt>>,
    next_sequences: BTreeMap<WorkloadId, u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedProcessedCommand {
    key: JournalKey,
    value: ProcessedCommand,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedConflict {
    key: JournalKey,
    value: StoredEvents,
}

fn has_exact_p_tag(event: &Event, expected: &str) -> bool {
    let tags: Vec<_> = event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == "p")
        .filter_map(|tag| tag.content())
        .collect();
    tags.len() == 1 && tags[0] == expected
}

fn safe_error_for_validation(
    error: &buzz_core::execution::ExecutionValidationError,
) -> SafeErrorCode {
    use buzz_core::execution::ExecutionValidationError as Validation;
    match error {
        Validation::Expired => SafeErrorCode::Expired,
        _ => SafeErrorCode::InvalidCommand,
    }
}

fn required_env(name: &str) -> Result<String, NodeError> {
    std::env::var(name)
        .map(|value| value.trim().to_string())
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| NodeError::MissingEnvironment(name.to_string()))
}

fn parse_auth_tag(value: &str) -> Result<Tag, NodeError> {
    let values: Vec<String> = serde_json::from_str(value)
        .map_err(|error| NodeError::InvalidConfiguration(format!("BUZZ_AUTH_TAG: {error}")))?;
    Tag::parse(values)
        .map_err(|error| NodeError::InvalidConfiguration(format!("BUZZ_AUTH_TAG: {error}")))
}

fn set_private_file_permissions(path: &Path) -> Result<(), NodeError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

impl From<buzz_core::execution::ExecutionValidationError> for NodeError {
    fn from(error: buzz_core::execution::ExecutionValidationError) -> Self {
        Self::InvalidConfiguration(error.to_string())
    }
}

/// Assert the wire protocol version used by this crate remains explicit.
pub const fn protocol_version() -> u16 {
    EXECUTION_PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::execution::{CredentialRef, ExecutionCommand, WorkloadSpec};
    use chrono::Duration;
    use nostr::nips::nip44;
    use nostr::EventBuilder;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);
    const TEST_RELAY_AUTHORITY: &str = "relay.example";

    fn paired_owner_store(owner: &Keys, node: &NodeIdentity, data_dir: &Path) -> OwnerStore {
        let node_id = node.node_id().expect("node id");
        let attestation = ExecutionNodeAttestation::sign(owner, &node_id, TEST_RELAY_AUTHORITY)
            .expect("attestation");
        let mut owners = OwnerStore::default();
        owners
            .add_attestation(attestation, &node_id, TEST_RELAY_AUTHORITY, data_dir)
            .expect("pair owner");
        owners
    }

    #[test]
    fn owner_attestation_is_bound_to_the_active_relay() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let owners = paired_owner_store(&owner, &node, &dir);
        let node_id = node.node_id().expect("node id");
        let owner_pubkey = owner.public_key().to_hex();

        assert!(owners.contains_for_relay(&owner_pubkey, &node_id, TEST_RELAY_AUTHORITY));
        assert!(!owners.contains_for_relay(&owner_pubkey, &node_id, "another-relay.example"));
        let _ = fs::remove_dir_all(dir);
    }

    fn temp_dir() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let counter = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("buzz-node-test-{suffix}-{counter}"))
    }

    #[test]
    fn identity_is_stable_and_announcement_is_sanitized() {
        let dir = temp_dir();
        let first = NodeIdentity::load_or_create(&dir).expect("create identity");
        let second = NodeIdentity::load_or_create(&dir).expect("load identity");
        assert_eq!(
            first.node_id().expect("node id"),
            second.node_id().expect("node id")
        );

        let event = build_announcement(&first, "Example execution node").expect("announcement");
        let content: serde_json::Value = serde_json::from_str(&event.content).expect("json");
        assert_eq!(content["displayName"], "Example execution node");
        assert!(event.content.find("docker").is_none());
        assert!(event.content.find("privateKey").is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn desktop_pairing_payload_ignores_legacy_private_key_field() {
        let payload = serde_json::json!({
            "relayUrl": "wss://relay.example",
            "pubkey": "a".repeat(64),
            "nsec": "must-not-be-persisted"
        });
        let parsed = parse_desktop_pairing_payload(&payload.to_string()).expect("payload");
        assert_eq!(parsed.owner_pubkey, "a".repeat(64));
        assert!(!serde_json::to_string(&parsed)
            .expect("serialize")
            .contains("must-not-be-persisted"));
        let debug = format!("{parsed:?}");
        assert!(!debug.contains("must-not-be-persisted"));
        assert!(debug.contains("[redacted]"));
    }

    #[test]
    fn owner_store_deduplicates_and_validates_public_keys() {
        let dir = temp_dir();
        let mut store = OwnerStore::default();
        store.add(&"b".repeat(64), &dir).expect("add owner");
        store.add(&"b".repeat(64), &dir).expect("deduplicate owner");
        assert_eq!(store.owners(), &["b".repeat(64)]);
        assert!(store.add("not-a-key", &dir).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    fn deploy_command_event(
        owner: &Keys,
        node: &NodeIdentity,
        command: &ExecutionCommandEnvelope,
    ) -> Event {
        let plaintext = serde_json::to_string(command).expect("command JSON");
        let encrypted = nip44::encrypt(
            owner.secret_key(),
            &node.keys.public_key(),
            &plaintext,
            nip44::Version::V2,
        )
        .expect("encrypt command");
        EventBuilder::new(Kind::Custom(KIND_EXECUTION_NODE_COMMAND as u16), encrypted)
            .tags([Tag::parse(["p", &node.keys.public_key().to_hex()]).expect("p tag")])
            .sign_with_keys(owner)
            .expect("command event")
    }

    #[tokio::test]
    async fn encrypted_deploy_is_reconciled_once_and_receipts_are_terminal() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let owners = paired_owner_store(&owner, &node, &dir);
        let workload_id = WorkloadId::random();
        let workload = WorkloadSpec::agent(
            workload_id,
            "Research agent",
            "fake-runtime",
            Some("model".into()),
            Some("provider".into()),
            vec![CredentialRef::new("provider", "primary").expect("credential ref")],
        )
        .expect("workload");
        let now = Utc::now();
        let command = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::Deploy { workload },
        )
        .expect("command");
        let event = deploy_command_event(&owner, &node, &command);
        let controller = ExecutionController::default();

        let first = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &event,
                now + Duration::seconds(1),
            )
            .await
            .expect("first delivery");
        let second = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &event,
                now + Duration::seconds(2),
            )
            .await
            .expect("duplicate delivery");

        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 2);
        let runtime = controller.runtime().await;
        assert_eq!(runtime.workload_count(), 1);
        assert_eq!(runtime.deploy_invocations(), 1);
        for receipt_event in first {
            assert!(receipt_event.verify_id());
            assert!(receipt_event.verify_signature());
            let plaintext = nip44::decrypt(
                owner.secret_key(),
                &node.keys.public_key(),
                &receipt_event.content,
            )
            .expect("decrypt receipt");
            let receipt: ExecutionReceipt = serde_json::from_str(&plaintext).expect("receipt");
            assert_eq!(receipt.command_id, command.command_id());
            assert_eq!(receipt.node_id, command.node_id().clone());
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn persisted_command_state_blocks_duplicate_after_restart() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let owners = paired_owner_store(&owner, &node, &dir);
        let now = Utc::now();
        let command = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::Deploy {
                workload: WorkloadSpec::agent(
                    WorkloadId::random(),
                    "Research agent",
                    "fake-runtime",
                    None,
                    None,
                    Vec::new(),
                )
                .expect("workload"),
            },
        )
        .expect("command");
        let event = deploy_command_event(&owner, &node, &command);
        let first_controller = ExecutionController::load(&dir).expect("load controller");
        let first_receipts = first_controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &event,
                now + Duration::seconds(1),
            )
            .await
            .expect("first delivery");

        let restarted_controller = ExecutionController::load(&dir).expect("restart controller");
        let receipts = restarted_controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &event,
                now + Duration::seconds(2),
            )
            .await
            .expect("duplicate delivery after restart");

        assert_eq!(receipts.len(), 2);
        assert_eq!(receipts, first_receipts);
        let runtime = restarted_controller.runtime().await;
        assert_eq!(runtime.workload_count(), 1);
        assert_eq!(runtime.deploy_invocations(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn lifecycle_commands_update_durable_runtime_state() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let owners = paired_owner_store(&owner, &node, &dir);
        let workload_id = WorkloadId::random();
        let now = Utc::now();
        let workload = WorkloadSpec::agent(
            workload_id.clone(),
            "Lifecycle agent",
            "fake-runtime",
            None,
            None,
            Vec::new(),
        )
        .expect("workload");
        let command = |operation| {
            ExecutionCommandEnvelope::new(
                node.node_id().expect("node id"),
                now,
                now + Duration::minutes(5),
                operation,
            )
            .expect("command")
        };
        let commands = [
            ExecutionCommand::Deploy {
                workload: workload.clone(),
            },
            ExecutionCommand::Stop {
                workload_id: workload_id.clone(),
            },
            ExecutionCommand::Start {
                workload_id: workload_id.clone(),
            },
            ExecutionCommand::Restart {
                workload_id: workload_id.clone(),
            },
            ExecutionCommand::Remove {
                workload_id: workload_id.clone(),
            },
        ];

        let controller = ExecutionController::default();
        let mut sequences = Vec::new();
        for operation in commands {
            let envelope = command(operation);
            let receipts = controller
                .handle_command_event(
                    &node,
                    &owners,
                    TEST_RELAY_AUTHORITY,
                    &deploy_command_event(&owner, &node, &envelope),
                    now + Duration::seconds(1),
                )
                .await
                .expect("lifecycle command");
            let plaintext = nip44::decrypt(
                owner.secret_key(),
                &node.keys.public_key(),
                &receipts[1].content,
            )
            .expect("decrypt lifecycle receipt");
            sequences.push(serde_json::from_str::<ExecutionReceipt>(&plaintext).expect("receipt"));
        }

        assert!(sequences.iter().all(ExecutionReceipt::is_terminal));
        assert!(sequences
            .windows(2)
            .all(|pair| pair[1].sequence > pair[0].sequence));
        assert_eq!(controller.runtime().await.workload_count(), 0);
        let statuses = controller.workload_statuses().await;
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].lifecycle, WorkloadLifecycle::Removed);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn command_id_reuse_with_different_payload_is_rejected_and_cached() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let owners = paired_owner_store(&owner, &node, &dir);
        let now = Utc::now();
        let first = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::Deploy {
                workload: WorkloadSpec::agent(
                    WorkloadId::random(),
                    "Original",
                    "fake-runtime",
                    None,
                    None,
                    Vec::new(),
                )
                .expect("workload"),
            },
        )
        .expect("command");
        let mut conflicting = first.clone();
        if let ExecutionCommand::Deploy { workload } = &mut conflicting.command {
            workload.display_name = "Changed".into();
        }
        let controller = ExecutionController::default();
        controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &first),
                now + Duration::seconds(1),
            )
            .await
            .expect("first command");
        let first_conflict = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &conflicting),
                now + Duration::seconds(2),
            )
            .await
            .expect("conflict receipt");
        let second_conflict = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &conflicting),
                now + Duration::seconds(3),
            )
            .await
            .expect("cached conflict receipt");

        assert_eq!(first_conflict, second_conflict);
        let plaintext = nip44::decrypt(
            owner.secret_key(),
            &node.keys.public_key(),
            &first_conflict[0].content,
        )
        .expect("decrypt conflict");
        let receipt: ExecutionReceipt = serde_json::from_str(&plaintext).expect("receipt");
        assert_eq!(
            receipt.outcome,
            ReceiptOutcome::Rejected {
                error: SafeErrorCode::Conflict
            }
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn provider_auth_challenge_and_response_keep_secrets_off_node_projections() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let owners = paired_owner_store(&owner, &node, &dir);
        let workload_id = WorkloadId::random();
        let now = Utc::now();
        let deploy = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::Deploy {
                workload: WorkloadSpec::agent(
                    workload_id.clone(),
                    "Auth agent",
                    "fake-runtime",
                    None,
                    None,
                    Vec::new(),
                )
                .expect("workload"),
            },
        )
        .expect("deploy command");
        let controller = ExecutionController::load(&dir).expect("controller");
        controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &deploy),
                now,
            )
            .await
            .expect("deploy");

        let session = buzz_core::execution::ProviderAuthSession::new(
            workload_id.clone(),
            "anthropic",
            "auth-session",
            now + Duration::minutes(5),
        )
        .expect("session");
        let begin = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::AuthenticateProvider { session },
        )
        .expect("begin command");
        let challenge_events = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &begin),
                now,
            )
            .await
            .expect("challenge");
        let challenge_plaintext = nip44::decrypt(
            owner.secret_key(),
            &node.keys.public_key(),
            &challenge_events[1].content,
        )
        .expect("decrypt challenge");
        let challenge: ExecutionReceipt =
            serde_json::from_str(&challenge_plaintext).expect("challenge receipt");
        assert_eq!(challenge.outcome, ReceiptOutcome::Progress);
        assert!(matches!(
            challenge.detail,
            Some(ReceiptDetail::ProviderAuthChallenge { .. })
        ));

        let response = ProviderAuthResponse::new(
            workload_id.clone(),
            "auth-session",
            "secret-token-that-must-stay-local",
        )
        .expect("response");
        let submit = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::SubmitProviderAuthentication { response },
        )
        .expect("submit command");
        controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &submit),
                now,
            )
            .await
            .expect("submit");
        let persisted = fs::read_to_string(dir.join("execution-state.json")).expect("state");
        assert!(!persisted.contains("secret-token-that-must-stay-local"));
        assert!(persisted.contains("authenticated"));
        assert!(persisted.contains("anthropic"));

        let retry_session = buzz_core::execution::ProviderAuthSession::new(
            workload_id.clone(),
            "anthropic",
            "retry-session",
            now + Duration::minutes(5),
        )
        .expect("retry session");
        let retry_begin = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::AuthenticateProvider {
                session: retry_session,
            },
        )
        .expect("retry begin command");
        controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &retry_begin),
                now,
            )
            .await
            .expect("retry begin");
        let cancel = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::CancelProviderAuthentication {
                workload_id: workload_id.clone(),
                session_id: "retry-session".into(),
            },
        )
        .expect("cancel command");
        let cancel_events = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &cancel),
                now,
            )
            .await
            .expect("cancel");
        let cancel_plaintext = nip44::decrypt(
            owner.secret_key(),
            &node.keys.public_key(),
            &cancel_events[1].content,
        )
        .expect("decrypt cancel receipt");
        let cancel_receipt: ExecutionReceipt =
            serde_json::from_str(&cancel_plaintext).expect("cancel receipt");
        assert_eq!(cancel_receipt.outcome, ReceiptOutcome::Succeeded);

        let retry_response = ProviderAuthResponse::new(
            workload_id.clone(),
            "retry-session",
            "response-after-cancel",
        )
        .expect("retry response");
        let retry_submit = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::SubmitProviderAuthentication {
                response: retry_response,
            },
        )
        .expect("retry submit command");
        let retry_events = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &retry_submit),
                now,
            )
            .await
            .expect("retry submit");
        let retry_plaintext = nip44::decrypt(
            owner.secret_key(),
            &node.keys.public_key(),
            &retry_events[1].content,
        )
        .expect("decrypt retry receipt");
        let retry_receipt: ExecutionReceipt =
            serde_json::from_str(&retry_plaintext).expect("retry receipt");
        assert_eq!(
            retry_receipt.outcome,
            ReceiptOutcome::Failed {
                error: SafeErrorCode::AuthenticationFailed
            }
        );

        let expiring_session = buzz_core::execution::ProviderAuthSession::new(
            workload_id.clone(),
            "anthropic",
            "expiring-session",
            now + Duration::seconds(1),
        )
        .expect("expiring session");
        let expiring_begin = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::AuthenticateProvider {
                session: expiring_session,
            },
        )
        .expect("expiring begin command");
        controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &expiring_begin),
                now,
            )
            .await
            .expect("expiring begin");
        let expired_response = ProviderAuthResponse::new(
            workload_id.clone(),
            "expiring-session",
            "response-after-expiry",
        )
        .expect("expired response");
        let expired_submit = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::SubmitProviderAuthentication {
                response: expired_response,
            },
        )
        .expect("expired submit command");
        let expired_submit_events = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &expired_submit),
                now + Duration::seconds(2),
            )
            .await
            .expect("expired submit");
        let expired_submit_plaintext = nip44::decrypt(
            owner.secret_key(),
            &node.keys.public_key(),
            &expired_submit_events[1].content,
        )
        .expect("decrypt expired submit receipt");
        let expired_submit_receipt: ExecutionReceipt =
            serde_json::from_str(&expired_submit_plaintext).expect("expired submit receipt");
        assert_eq!(
            expired_submit_receipt.outcome,
            ReceiptOutcome::Failed {
                error: SafeErrorCode::AuthenticationFailed
            }
        );

        let expired_start_session = buzz_core::execution::ProviderAuthSession::new(
            workload_id,
            "anthropic",
            "already-expired-session",
            now + Duration::seconds(1),
        )
        .expect("expired start session");
        let expired = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::AuthenticateProvider {
                session: expired_start_session,
            },
        )
        .expect("expiring command");
        let expired_events = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &expired),
                now + Duration::seconds(2),
            )
            .await
            .expect("expired auth");
        let expired_plaintext = nip44::decrypt(
            owner.secret_key(),
            &node.keys.public_key(),
            &expired_events[0].content,
        )
        .expect("decrypt expired receipt");
        let expired_receipt: ExecutionReceipt =
            serde_json::from_str(&expired_plaintext).expect("expired receipt");
        assert_eq!(
            expired_receipt.outcome,
            ReceiptOutcome::Rejected {
                error: SafeErrorCode::Expired
            }
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn different_workloads_can_progress_concurrently() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let owners = paired_owner_store(&owner, &node, &dir);
        let now = Utc::now();
        let make_command = |name| {
            ExecutionCommandEnvelope::new(
                node.node_id().expect("node id"),
                now,
                now + Duration::minutes(5),
                ExecutionCommand::Deploy {
                    workload: WorkloadSpec::agent(
                        WorkloadId::random(),
                        name,
                        "fake-runtime",
                        None,
                        None,
                        Vec::new(),
                    )
                    .expect("workload"),
                },
            )
            .expect("command")
        };
        let first = make_command("first");
        let second = make_command("second");
        let first_event = deploy_command_event(&owner, &node, &first);
        let second_event = deploy_command_event(&owner, &node, &second);
        // Use durable state here so concurrent commands exercise the same
        // persistence path used by a real node, including the serialized
        // snapshot writer.
        let controller =
            ExecutionController::load_with_concurrency(&dir, 2).expect("load durable controller");

        let (first_result, second_result) = tokio::join!(
            controller.handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &first_event,
                now,
            ),
            controller.handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &second_event,
                now,
            ),
        );

        assert!(first_result.is_ok());
        assert!(second_result.is_ok());
        assert_eq!(controller.runtime().await.workload_count(), 2);
        let restarted = ExecutionController::load(&dir).expect("restart controller");
        assert_eq!(restarted.runtime().await.workload_count(), 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn legacy_execution_state_is_migrated_without_startup_failure() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).expect("data directory");
        fs::write(
            dir.join("execution-state.json"),
            serde_json::json!({
                "processed": {},
                "next_sequences": {}
            })
            .to_string(),
        )
        .expect("legacy state");

        ExecutionController::load(&dir).expect("migrate legacy state");
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn unpaired_command_never_reaches_the_runtime() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let now = Utc::now();
        let command = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::Deploy {
                workload: WorkloadSpec::agent(
                    WorkloadId::random(),
                    "Research agent",
                    "fake-runtime",
                    None,
                    None,
                    Vec::new(),
                )
                .expect("workload"),
            },
        )
        .expect("command");
        let event = deploy_command_event(&owner, &node, &command);
        let controller = ExecutionController::default();

        let receipts = controller
            .handle_command_event(
                &node,
                &OwnerStore::default(),
                TEST_RELAY_AUTHORITY,
                &event,
                now,
            )
            .await
            .expect("unpaired command is ignored");

        assert!(receipts.is_empty());
        assert_eq!(controller.runtime().await.workload_count(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn expired_command_is_rejected_without_runtime_side_effects() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let owners = paired_owner_store(&owner, &node, &dir);
        let issued_at = Utc::now() - Duration::minutes(2);
        let command = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            issued_at,
            issued_at + Duration::minutes(1),
            ExecutionCommand::Deploy {
                workload: WorkloadSpec::agent(
                    WorkloadId::random(),
                    "Expired agent",
                    "fake-runtime",
                    None,
                    None,
                    Vec::new(),
                )
                .expect("workload"),
            },
        )
        .expect("command");
        let event = deploy_command_event(&owner, &node, &command);
        let controller = ExecutionController::default();

        let receipts = controller
            .handle_command_event(&node, &owners, TEST_RELAY_AUTHORITY, &event, Utc::now())
            .await
            .expect("expired command receipt");
        let plaintext = nip44::decrypt(
            owner.secret_key(),
            &node.keys.public_key(),
            &receipts[0].content,
        )
        .expect("decrypt rejection");
        let receipt: ExecutionReceipt = serde_json::from_str(&plaintext).expect("receipt");

        assert_eq!(
            receipt.outcome,
            ReceiptOutcome::Rejected {
                error: SafeErrorCode::Expired
            }
        );
        assert_eq!(controller.runtime().await.workload_count(), 0);
        let _ = fs::remove_dir_all(dir);
    }
}
