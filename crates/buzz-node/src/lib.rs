#![deny(unsafe_code)]

//! Runtime-neutral relay client for a standalone Buzz execution node.

use std::collections::{BTreeMap, HashMap, HashSet};
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
    KIND_PRESENCE_UPDATE,
};
use chrono::{DateTime, Utc};
use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, PublicKey, Tag, ToBech32};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::{Mutex, Semaphore};
use tracing::warn;

pub mod substrate;

pub use substrate::docker::{DockerSubstrate, DockerSubstrateConfig};
pub use substrate::process::{ProcessSubstrate, ProcessSubstrateConfig};
pub use substrate::{InertSubstrate, Substrate, SubstrateError, WorkloadExit};

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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

    /// Reload the allowlist from disk and report whether it changed.
    ///
    /// `buzz-node pair` runs as a separate process and persists new owner
    /// attestations to `owners.json` while `buzz-node run` keeps serving. A
    /// running node polls with this method so a completed pairing takes
    /// effect — refreshed announcement and command authorization — without a
    /// restart. Returns `Ok(None)` when the on-disk contents match `self`.
    pub fn reload_if_changed(&self, data_dir: &Path) -> Result<Option<Self>, NodeError> {
        let latest = Self::load(data_dir)?;
        Ok((latest != *self).then_some(latest))
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

/// Build the ephemeral kind:20001 presence event a node publishes as its
/// liveness heartbeat.
///
/// The content is a bare status string (`"online"`, `"offline"`) — the same
/// shape members and managed agents publish — so the relay's presence handler
/// stores it in Redis (short TTL) and synthesizes it back on presence
/// queries. Ephemeral kinds are rejected by the relay's HTTP bridge, so the
/// event must be published over the node's WebSocket connection.
pub fn build_presence_event(identity: &NodeIdentity, status: &str) -> Result<Event, NodeError> {
    EventBuilder::new(Kind::Custom(KIND_PRESENCE_UPDATE as u16), status)
        .tags([])
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

/// Durable, owner-scoped ledger of workload intent.
///
/// The ledger is the node's persisted record of what a paired owner asked
/// for: admitted workload specifications, their last commanded lifecycle, and
/// sequenced removal tombstones. It deliberately stores only safe workload
/// data and never launches anything itself — making reality match an admitted
/// command is the [`Substrate`]'s job.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkloadLedger {
    workloads: BTreeMap<WorkloadId, LedgerWorkload>,
    /// Removal tombstones keyed by workload, valued by the node-assigned
    /// receipt sequence of the removal. A tombstone blocks deploys that
    /// cannot prove they were issued after the owner observed the removal
    /// (a stale or replayed deploy must not resurrect a removed workload),
    /// while a deploy carrying `supersedes_removal` at or above the recorded
    /// sequence clears the tombstone.
    #[serde(default)]
    removed_workloads: BTreeMap<WorkloadId, u64>,
    deploy_admissions: usize,
}

/// Encrypted local state proving a provider subscription has been authenticated.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CredentialStore {
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

impl CredentialStore {
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
struct LedgerWorkload {
    spec: WorkloadSpec,
    lifecycle: WorkloadLifecycle,
}

impl WorkloadLedger {
    fn without_private_keys(mut self) -> Self {
        for workload in self.workloads.values_mut() {
            workload.spec = workload.spec.clone().without_private_key();
        }
        self
    }

    /// Pure admissibility check for a deploy — no state is modified.
    ///
    /// A removal tombstone rejects the deploy unless the deploy proves it was
    /// issued after the owner observed the removal: `supersedes_removal` must
    /// be at or above the removal's node-assigned receipt sequence. A stale or
    /// replayed deploy from before the removal cannot carry that sequence,
    /// because the node had not assigned it yet. Legacy tombstones persisted
    /// without a sequence (recorded as `0`) are cleared by any new deploy.
    fn validate_deploy(
        &self,
        workload: &WorkloadSpec,
        supersedes_removal: Option<u64>,
    ) -> Result<(), SafeErrorCode> {
        if let Some(&removal_sequence) = self.removed_workloads.get(&workload.workload_id) {
            let supersedes = removal_sequence == 0
                || supersedes_removal.is_some_and(|sequence| sequence >= removal_sequence);
            if !supersedes {
                return Err(SafeErrorCode::Conflict);
            }
        }
        Ok(())
    }

    /// Pure admissibility check for start/stop/restart — no state is modified.
    fn validate_transition(&self, workload_id: &WorkloadId) -> Result<(), SafeErrorCode> {
        if self.workloads.contains_key(workload_id) {
            Ok(())
        } else {
            Err(SafeErrorCode::WorkloadNotFound)
        }
    }

    /// Pure admissibility check for a remove — no state is modified.
    fn validate_remove(&self, workload_id: &WorkloadId) -> Result<(), SafeErrorCode> {
        if self.workloads.contains_key(workload_id)
            || self.removed_workloads.contains_key(workload_id)
        {
            Ok(())
        } else {
            Err(SafeErrorCode::WorkloadNotFound)
        }
    }

    /// Whether the ledger currently admits this workload.
    pub fn contains(&self, workload_id: &WorkloadId) -> bool {
        self.workloads.contains_key(workload_id)
    }

    /// Return the durable, key-stripped spec of an admitted workload — what
    /// the substrate receives for start and restart.
    pub fn durable_spec(&self, workload_id: &WorkloadId) -> Option<WorkloadSpec> {
        self.workloads
            .get(workload_id)
            .map(|workload| workload.spec.clone())
    }

    /// Admit a deploy into the ledger. Callers run [`Self::validate_deploy`]
    /// and the substrate action first; this records the outcome.
    pub fn deploy(
        &mut self,
        workload: &WorkloadSpec,
        supersedes_removal: Option<u64>,
    ) -> Result<WorkloadLifecycle, SafeErrorCode> {
        self.validate_deploy(workload, supersedes_removal)?;
        self.removed_workloads.remove(&workload.workload_id);
        let is_new = !self.workloads.contains_key(&workload.workload_id);
        self.workloads.insert(
            workload.workload_id.clone(),
            LedgerWorkload {
                spec: workload.clone().without_private_key(),
                lifecycle: WorkloadLifecycle::Running,
            },
        );
        if is_new {
            self.deploy_admissions += 1;
        }
        Ok(WorkloadLifecycle::Running)
    }

    /// Record a started workload.
    pub fn start(&mut self, workload_id: &WorkloadId) -> Result<WorkloadLifecycle, SafeErrorCode> {
        self.transition(workload_id, WorkloadLifecycle::Running)
    }

    /// Record a stopped workload.
    pub fn stop(&mut self, workload_id: &WorkloadId) -> Result<WorkloadLifecycle, SafeErrorCode> {
        self.transition(workload_id, WorkloadLifecycle::Stopped)
    }

    /// Record a restarted workload.
    pub fn restart(
        &mut self,
        workload_id: &WorkloadId,
    ) -> Result<WorkloadLifecycle, SafeErrorCode> {
        self.transition(workload_id, WorkloadLifecycle::Running)
    }

    /// Remove an admitted workload and record its removal tombstone.
    pub fn remove(&mut self, workload_id: &WorkloadId) -> Result<WorkloadLifecycle, SafeErrorCode> {
        if self.workloads.remove(workload_id).is_some() {
            // The tombstone starts at sequence 0 and is stamped with the
            // removal's terminal receipt sequence via
            // `record_removal_sequence` before the state is persisted.
            self.removed_workloads
                .entry(workload_id.clone())
                .or_insert(0);
            return Ok(WorkloadLifecycle::Removed);
        }
        if self.removed_workloads.contains_key(workload_id) {
            return Ok(WorkloadLifecycle::Removed);
        }
        Err(SafeErrorCode::WorkloadNotFound)
    }

    /// Record that a workload body exited on its own.
    ///
    /// A clean exit means the body finished — the agent said goodbye and shut
    /// itself down — and maps to [`WorkloadLifecycle::Stopped`]; a non-zero
    /// exit maps to [`WorkloadLifecycle::Failed`]. Only a currently `Running`
    /// row transitions: exits observed after an explicit stop or removal are
    /// stale news. Returns the new lifecycle when the ledger changed.
    pub fn record_body_exit(
        &mut self,
        workload_id: &WorkloadId,
        clean: bool,
    ) -> Option<WorkloadLifecycle> {
        let workload = self.workloads.get_mut(workload_id)?;
        if workload.lifecycle != WorkloadLifecycle::Running {
            return None;
        }
        workload.lifecycle = if clean {
            WorkloadLifecycle::Stopped
        } else {
            WorkloadLifecycle::Failed
        };
        Some(workload.lifecycle)
    }

    /// Record the node-assigned receipt sequence of a successful removal on
    /// its tombstone, so later deploys can prove they supersede it. Keeps the
    /// highest sequence when a removal is repeated idempotently.
    fn record_removal_sequence(&mut self, workload_id: &WorkloadId, sequence: u64) {
        if let Some(removal_sequence) = self.removed_workloads.get_mut(workload_id) {
            *removal_sequence = (*removal_sequence).max(sequence);
        }
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

    /// Number of distinct workload rows currently admitted to the ledger.
    pub fn workload_count(&self) -> usize {
        self.workloads.len()
    }

    /// Number of first-time deploy admissions.
    pub fn deploy_admissions(&self) -> usize {
        self.deploy_admissions
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
        statuses.extend(self.removed_workloads.keys().filter_map(|workload_id| {
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
    substrate: Arc<dyn Substrate>,
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
    ledgers: BTreeMap<String, WorkloadLedger>,
    credentials: BTreeMap<String, CredentialStore>,
    processed: HashMap<JournalKey, ProcessedCommand>,
    conflicts: HashMap<JournalKey, StoredEvents>,
    /// Journal keys whose substrate action is currently running. A second
    /// command reusing the same command id (necessarily for a different
    /// workload — same-workload redelivery is serialized by the workload
    /// lock) is a conflicting reuse and is rejected instead of racing.
    in_flight: HashSet<JournalKey>,
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
    ///
    /// Library constructors default to the no-op [`InertSubstrate`]; attach a
    /// real substrate with [`Self::with_substrate`].
    pub fn with_concurrency(limit: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(ControllerState::default())),
            substrate: Arc::new(InertSubstrate),
            workload_locks: Arc::new(Mutex::new(HashMap::new())),
            concurrency: Arc::new(Semaphore::new(limit.max(1))),
            persist_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Replace the substrate that performs real side effects for workload
    /// commands. The durable ledger and idempotency journal are unaffected.
    pub fn with_substrate(mut self, substrate: Arc<dyn Substrate>) -> Self {
        self.substrate = substrate;
        self
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
        let state: PersistedExecutionState = serde_json::from_str(&fs::read_to_string(path)?)?;
        let controller = Self::with_concurrency(limit);
        *controller.state.try_lock().map_err(|_| {
            NodeError::InvalidConfiguration(
                "new execution controller was unexpectedly locked".into(),
            )
        })? = ControllerState {
            ledgers: state.ledgers,
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
            in_flight: HashSet::new(),
            next_sequences: state.next_sequences,
            data_dir: Some(data_dir.to_path_buf()),
        };
        Ok(controller)
    }

    /// Process one signed, owner-authorized command event and return its signed
    /// encrypted receipt events. Invalid or unauthorized events are ignored by
    /// the caller's subscription loop and never reach the ledger or substrate.
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

        // The substrate seam is asynchronous even when the substrate is inert. Yielding
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

        if state.in_flight.contains(&journal_key) {
            // The same command id is mid-execution for a different workload
            // (same-workload redelivery is serialized by the workload lock):
            // conflicting reuse, rejected without touching the substrate.
            let receipts = vec![next_receipt(
                &mut state,
                &journal_key.owner,
                &envelope,
                ReceiptOutcome::Rejected {
                    error: SafeErrorCode::Conflict,
                },
            )?];
            drop(state);
            return self.receipt_events(identity, &owner, receipts);
        }

        let rejection = if envelope.node_id() != &node_id {
            Some(vec![next_receipt(
                &mut state,
                &journal_key.owner,
                &envelope,
                ReceiptOutcome::Rejected {
                    error: SafeErrorCode::Unauthorized,
                },
            )?])
        } else if let Err(error) = envelope.validate_at(now) {
            Some(vec![next_receipt(
                &mut state,
                &journal_key.owner,
                &envelope,
                ReceiptOutcome::Rejected {
                    error: safe_error_for_validation(&error),
                },
            )?])
        } else {
            None
        };

        let receipts = match rejection {
            Some(receipts) => receipts,
            None => {
                // Execute drops the state lock around the substrate action;
                // the in-flight marker keeps the command id reserved while no
                // lock is held.
                state.in_flight.insert(journal_key.clone());
                drop(state);
                let executed = self
                    .execute(&journal_key.owner, &envelope, now, &identity.keys)
                    .await;
                state = self.state.lock().await;
                state.in_flight.remove(&journal_key);
                executed?
            }
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

    /// Inspect the first owner's workload ledger for diagnostics and tests.
    pub async fn ledger(&self) -> WorkloadLedger {
        let state = self.state.lock().await;
        state.ledgers.values().next().cloned().unwrap_or_default()
    }

    /// Return the current durable workload projection for node announcements.
    pub async fn workload_statuses(&self) -> Vec<buzz_core::execution::WorkloadStatus> {
        let state = self.state.lock().await;
        state
            .ledgers
            .iter()
            .flat_map(|(owner, ledger)| {
                ledger.statuses(state.next_sequences.get(owner).unwrap_or(&BTreeMap::new()))
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

impl ExecutionController {
    /// Execute one admitted command: ledger validation first (pure), then the
    /// substrate side effect, then — only on substrate success — the ledger
    /// mutation and its `Succeeded` receipt. A substrate failure earns a
    /// `Failed` receipt with a safe error code and leaves the ledger exactly
    /// as it was: a failed deploy records no running workload, a failed stop
    /// keeps the previous lifecycle.
    ///
    /// The state lock is released around the substrate action (which may wait
    /// on process shutdown); the per-workload lock held by the caller keeps
    /// same-workload commands strictly ordered while other workloads make
    /// progress.
    async fn execute(
        &self,
        owner: &str,
        envelope: &ExecutionCommandEnvelope,
        now: DateTime<Utc>,
        node_keys: &Keys,
    ) -> Result<Vec<ExecutionReceipt>, NodeError> {
        // Phase 1 — accepted receipt plus pure ledger validation.
        let mut state = self.state.lock().await;
        let accepted = next_receipt(&mut state, owner, envelope, ReceiptOutcome::Accepted)?;

        if matches!(
            &envelope.command,
            ExecutionCommand::AuthenticateProvider { .. }
                | ExecutionCommand::SubmitProviderAuthentication { .. }
                | ExecutionCommand::CancelProviderAuthentication { .. }
        ) {
            // Provider authentication is pure credential bookkeeping — no
            // substrate involvement — and completes under one lock hold.
            let (outcome, detail) =
                execute_provider_auth(&mut state, owner, envelope, now, node_keys);
            let terminal = next_receipt_with_detail(&mut state, owner, envelope, outcome, detail)?;
            return Ok(vec![accepted, terminal]);
        }

        let ledger = state.ledgers.entry(owner.to_string()).or_default();
        let validation = match &envelope.command {
            ExecutionCommand::Deploy {
                workload,
                supersedes_removal,
            } => ledger.validate_deploy(workload, *supersedes_removal),
            ExecutionCommand::Start { workload_id }
            | ExecutionCommand::Stop { workload_id }
            | ExecutionCommand::Restart { workload_id } => ledger.validate_transition(workload_id),
            ExecutionCommand::Remove { workload_id } => ledger.validate_remove(workload_id),
            _ => Ok(()),
        };
        // Start and restart hand the substrate the ledger's durable,
        // key-stripped spec; a passing validation guarantees it exists.
        let durable_spec = if validation.is_ok() {
            match &envelope.command {
                ExecutionCommand::Start { workload_id }
                | ExecutionCommand::Restart { workload_id } => ledger.durable_spec(workload_id),
                _ => None,
            }
        } else {
            None
        };
        if let Err(error) = validation {
            let terminal = next_receipt(
                &mut state,
                owner,
                envelope,
                ReceiptOutcome::Failed { error },
            )?;
            return Ok(vec![accepted, terminal]);
        }
        drop(state);

        // Phase 2 — the substrate side effect, outside the state lock.
        let missing_spec = || {
            SubstrateError::new(
                SafeErrorCode::WorkloadNotFound,
                "workload disappeared from the ledger mid-command",
            )
        };
        let substrate_result = match &envelope.command {
            ExecutionCommand::Deploy { workload, .. } => {
                self.substrate.deploy(owner, workload).await
            }
            ExecutionCommand::Start { .. } => match &durable_spec {
                Some(spec) => self.substrate.start(owner, spec).await,
                None => Err(missing_spec()),
            },
            ExecutionCommand::Stop { workload_id } => self.substrate.stop(owner, workload_id).await,
            ExecutionCommand::Restart { .. } => match &durable_spec {
                Some(spec) => self.substrate.restart(owner, spec).await,
                None => Err(missing_spec()),
            },
            ExecutionCommand::Remove { workload_id } => {
                self.substrate.remove(owner, workload_id).await
            }
            _ => Ok(()),
        };

        // Phase 3 — ledger mutation and the terminal receipt.
        let mut state = self.state.lock().await;
        let outcome = match substrate_result {
            Err(error) => {
                // The node-local diagnostic stays in the node log; only the
                // safe classification travels in the receipt.
                warn!(
                    workload = envelope.command.workload_id().as_str(),
                    code = ?error.code,
                    message = %error.message,
                    "substrate action failed"
                );
                ReceiptOutcome::Failed { error: error.code }
            }
            Ok(()) => {
                let ledger = state.ledgers.entry(owner.to_string()).or_default();
                let applied = match &envelope.command {
                    ExecutionCommand::Deploy {
                        workload,
                        supersedes_removal,
                    } => ledger.deploy(workload, *supersedes_removal),
                    ExecutionCommand::Start { workload_id } => ledger.start(workload_id),
                    ExecutionCommand::Stop { workload_id } => ledger.stop(workload_id),
                    ExecutionCommand::Restart { workload_id } => ledger.restart(workload_id),
                    ExecutionCommand::Remove { workload_id } => ledger.remove(workload_id),
                    _ => Ok(WorkloadLifecycle::Pending),
                };
                match applied {
                    Ok(_) => ReceiptOutcome::Succeeded,
                    Err(error) => ReceiptOutcome::Failed { error },
                }
            }
        };
        let terminal = next_receipt_with_detail(&mut state, owner, envelope, outcome, None)?;
        // The removal tombstone must remember the sequence of the removal's
        // terminal receipt: that is the sequence the owner observes (in the
        // receipt and in the announced workload status) and echoes back in a
        // deliberate redeploy to prove it is not a stale replay.
        if matches!(envelope.command, ExecutionCommand::Remove { .. })
            && matches!(terminal.outcome, ReceiptOutcome::Succeeded)
        {
            if let Some(ledger) = state.ledgers.get_mut(owner) {
                ledger.record_removal_sequence(envelope.command.workload_id(), terminal.sequence);
            }
        }
        Ok(vec![accepted, terminal])
    }

    /// Record a body exit observed by the substrate's exit channel.
    ///
    /// A clean self-exit transitions the ledger row to `Stopped`, a non-zero
    /// exit to `Failed`; the body is never respawned. Returns whether the
    /// durable ledger changed so callers can republish the node announcement.
    pub async fn record_workload_exit(&self, exit: &WorkloadExit) -> Result<bool, NodeError> {
        let workload_lock = self.workload_lock_for(&exit.owner, &exit.workload_id).await;
        let _workload_guard = workload_lock.lock().await;
        let mut state = self.state.lock().await;
        let Some(ledger) = state.ledgers.get_mut(&exit.owner) else {
            return Ok(false);
        };
        if ledger
            .record_body_exit(&exit.workload_id, exit.clean)
            .is_none()
        {
            return Ok(false);
        }
        drop(state);
        self.persist_current_state().await?;
        Ok(true)
    }

    /// Return the serialization lock for one owner-scoped workload.
    async fn workload_lock_for(&self, owner: &str, workload_id: &WorkloadId) -> Arc<Mutex<()>> {
        let mut locks = self.workload_locks.lock().await;
        locks
            .entry(WorkloadKey {
                owner: owner.to_string(),
                workload_id: workload_id.clone(),
            })
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }
}

/// Apply one provider-authentication command to the credential store.
fn execute_provider_auth(
    state: &mut ControllerState,
    owner: &str,
    envelope: &ExecutionCommandEnvelope,
    now: DateTime<Utc>,
    node_keys: &Keys,
) -> (ReceiptOutcome, Option<ReceiptDetail>) {
    let result = match &envelope.command {
        ExecutionCommand::AuthenticateProvider { session } => {
            let workload_exists = state
                .ledgers
                .entry(owner.to_string())
                .or_default()
                .contains(&session.workload_id);
            if workload_exists {
                state
                    .credentials
                    .entry(owner.to_string())
                    .or_default()
                    .begin(session)
                    .map(Some)
            } else {
                Err(SafeErrorCode::WorkloadNotFound)
            }
        }
        ExecutionCommand::SubmitProviderAuthentication { response } => state
            .credentials
            .entry(owner.to_string())
            .or_default()
            .submit(response, now, node_keys)
            .map(Some),
        ExecutionCommand::CancelProviderAuthentication {
            workload_id,
            session_id,
        } => state
            .credentials
            .entry(owner.to_string())
            .or_default()
            .cancel(workload_id, session_id)
            .map(|_| None),
        _ => Err(SafeErrorCode::InvalidCommand),
    };
    match result {
        Ok(detail)
            if matches!(
                &envelope.command,
                ExecutionCommand::AuthenticateProvider { .. }
            ) =>
        {
            (ReceiptOutcome::Progress, detail)
        }
        Ok(detail) => (ReceiptOutcome::Succeeded, detail),
        Err(error) => (ReceiptOutcome::Failed { error }, None),
    }
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
        ledgers: state
            .ledgers
            .iter()
            .map(|(owner, ledger)| (owner.clone(), ledger.clone().without_private_keys()))
            .collect(),
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

/// On-disk shape of `execution-state.json`: the durable ledger plus the
/// command idempotency journal. Substrate state is deliberately absent — it
/// is rebuilt from the ledger on restart (minus one-time launch keys, which
/// are never persisted).
#[derive(Debug, Serialize, Deserialize)]
struct PersistedExecutionState {
    #[serde(default)]
    ledgers: BTreeMap<String, WorkloadLedger>,
    #[serde(default)]
    credentials: BTreeMap<String, CredentialStore>,
    processed: Vec<PersistedProcessedCommand>,
    #[serde(default)]
    conflicts: Vec<PersistedConflict>,
    next_sequences: BTreeMap<String, BTreeMap<WorkloadId, u64>>,
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

    #[test]
    fn owner_store_reload_detects_pairing_written_by_another_process() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let running = OwnerStore::load(&dir).expect("initial load");
        assert!(running
            .reload_if_changed(&dir)
            .expect("reload unchanged store")
            .is_none());

        // Simulate `buzz-node pair` persisting a new attestation out-of-process.
        let owner = Keys::generate();
        let _ = paired_owner_store(&owner, &node, &dir);

        let refreshed = running
            .reload_if_changed(&dir)
            .expect("reload after pairing")
            .expect("pairing change detected");
        assert_eq!(refreshed.owners(), &[owner.public_key().to_hex()]);
        assert_eq!(refreshed.attestations().len(), 1);
        assert!(refreshed
            .reload_if_changed(&dir)
            .expect("reload refreshed store")
            .is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn owner_store_reload_treats_missing_file_as_unchanged_empty_store() {
        let dir = temp_dir();
        assert!(OwnerStore::default()
            .reload_if_changed(&dir)
            .expect("reload missing file")
            .is_none());
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
            ExecutionCommand::Deploy {
                supersedes_removal: None,
                workload: Box::new(workload),
            },
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
        let ledger = controller.ledger().await;
        assert_eq!(ledger.workload_count(), 1);
        assert_eq!(ledger.deploy_admissions(), 1);
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
                supersedes_removal: None,
                workload: Box::new(
                    WorkloadSpec::agent(
                        WorkloadId::random(),
                        "Research agent",
                        "fake-runtime",
                        None,
                        None,
                        Vec::new(),
                    )
                    .expect("workload"),
                ),
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
        let ledger = restarted_controller.ledger().await;
        assert_eq!(ledger.workload_count(), 1);
        assert_eq!(ledger.deploy_admissions(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn lifecycle_commands_update_durable_ledger_state() {
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
                supersedes_removal: None,
                workload: Box::new(workload.clone()),
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
        assert_eq!(controller.ledger().await.workload_count(), 0);
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
                supersedes_removal: None,
                workload: Box::new(
                    WorkloadSpec::agent(
                        WorkloadId::random(),
                        "Original",
                        "fake-runtime",
                        None,
                        None,
                        Vec::new(),
                    )
                    .expect("workload"),
                ),
            },
        )
        .expect("command");
        let mut conflicting = first.clone();
        if let ExecutionCommand::Deploy { workload, .. } = &mut conflicting.command {
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
                supersedes_removal: None,
                workload: Box::new(
                    WorkloadSpec::agent(
                        workload_id.clone(),
                        "Auth agent",
                        "fake-runtime",
                        None,
                        None,
                        Vec::new(),
                    )
                    .expect("workload"),
                ),
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
                    supersedes_removal: None,
                    workload: Box::new(
                        WorkloadSpec::agent(
                            WorkloadId::random(),
                            name,
                            "fake-runtime",
                            None,
                            None,
                            Vec::new(),
                        )
                        .expect("workload"),
                    ),
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
        assert_eq!(controller.ledger().await.workload_count(), 2);
        let restarted = ExecutionController::load(&dir).expect("restart controller");
        assert_eq!(restarted.ledger().await.workload_count(), 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn unpaired_command_never_reaches_the_ledger() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let now = Utc::now();
        let command = ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::Deploy {
                supersedes_removal: None,
                workload: Box::new(
                    WorkloadSpec::agent(
                        WorkloadId::random(),
                        "Research agent",
                        "fake-runtime",
                        None,
                        None,
                        Vec::new(),
                    )
                    .expect("workload"),
                ),
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
        assert_eq!(controller.ledger().await.workload_count(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    fn terminal_receipt(owner: &Keys, node: &NodeIdentity, events: &[Event]) -> ExecutionReceipt {
        let plaintext = nip44::decrypt(
            owner.secret_key(),
            &node.keys.public_key(),
            &events[1].content,
        )
        .expect("decrypt terminal receipt");
        serde_json::from_str(&plaintext).expect("terminal receipt")
    }

    #[tokio::test]
    async fn redeploy_after_observed_removal_succeeds_and_stale_deploy_stays_blocked() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let owners = paired_owner_store(&owner, &node, &dir);
        let workload_id = WorkloadId::random();
        let workload = WorkloadSpec::agent(
            workload_id.clone(),
            "Movable agent",
            "fake-runtime",
            None,
            None,
            Vec::new(),
        )
        .expect("workload");
        let now = Utc::now();
        let envelope = |command| {
            ExecutionCommandEnvelope::new(
                node.node_id().expect("node id"),
                now,
                now + Duration::minutes(5),
                command,
            )
            .expect("command")
        };
        let controller = ExecutionController::load(&dir).expect("controller");

        let deploy = envelope(ExecutionCommand::Deploy {
            supersedes_removal: None,
            workload: Box::new(workload.clone()),
        });
        controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &deploy),
                now,
            )
            .await
            .expect("initial deploy");

        let remove = envelope(ExecutionCommand::Remove {
            workload_id: workload_id.clone(),
        });
        let remove_events = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &remove),
                now,
            )
            .await
            .expect("remove");
        let removal = terminal_receipt(&owner, &node, &remove_events);
        assert_eq!(removal.outcome, ReceiptOutcome::Succeeded);

        // A deploy that cannot prove it observed the removal — no sequence, or
        // one from before the removal receipt — is a potential stale replay
        // and must stay in conflict.
        for supersedes_removal in [None, Some(removal.sequence - 1)] {
            let stale = envelope(ExecutionCommand::Deploy {
                supersedes_removal,
                workload: Box::new(workload.clone()),
            });
            let events = controller
                .handle_command_event(
                    &node,
                    &owners,
                    TEST_RELAY_AUTHORITY,
                    &deploy_command_event(&owner, &node, &stale),
                    now,
                )
                .await
                .expect("stale deploy handled");
            assert_eq!(
                terminal_receipt(&owner, &node, &events).outcome,
                ReceiptOutcome::Failed {
                    error: SafeErrorCode::Conflict
                }
            );
        }
        assert_eq!(controller.ledger().await.workload_count(), 0);

        // A deliberate redeploy echoes the removal receipt's sequence. Restart
        // the controller first so the sequenced tombstone is proven to
        // round-trip through persisted state.
        let restarted = ExecutionController::load(&dir).expect("restart controller");
        let redeploy = envelope(ExecutionCommand::Deploy {
            supersedes_removal: Some(removal.sequence),
            workload: Box::new(workload.clone()),
        });
        let events = restarted
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &redeploy),
                now,
            )
            .await
            .expect("redeploy");
        assert_eq!(
            terminal_receipt(&owner, &node, &events).outcome,
            ReceiptOutcome::Succeeded
        );
        assert_eq!(restarted.ledger().await.workload_count(), 1);
        let statuses = restarted.workload_statuses().await;
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].lifecycle, WorkloadLifecycle::Running);
        let _ = fs::remove_dir_all(dir);
    }

    /// Recording substrate with scriptable failures, proving the
    /// ledger-validation → substrate → ledger-mutation ordering.
    #[derive(Debug, Default)]
    struct ScriptedSubstrate {
        calls: std::sync::Mutex<Vec<String>>,
        failures: std::sync::Mutex<HashMap<String, SafeErrorCode>>,
    }

    impl ScriptedSubstrate {
        fn fail(&self, operation: &str, error: SafeErrorCode) {
            self.failures
                .lock()
                .expect("failures lock")
                .insert(operation.to_string(), error);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("calls lock").clone()
        }

        fn record(&self, operation: &str) -> Result<(), SubstrateError> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(operation.to_string());
            match self.failures.lock().expect("failures lock").get(operation) {
                Some(error) => Err(SubstrateError::new(*error, "scripted failure")),
                None => Ok(()),
            }
        }
    }

    #[async_trait::async_trait]
    impl Substrate for ScriptedSubstrate {
        async fn deploy(
            &self,
            _owner: &str,
            _workload: &WorkloadSpec,
        ) -> Result<(), SubstrateError> {
            self.record("deploy")
        }

        async fn start(
            &self,
            _owner: &str,
            _workload: &WorkloadSpec,
        ) -> Result<(), SubstrateError> {
            self.record("start")
        }

        async fn stop(
            &self,
            _owner: &str,
            _workload_id: &WorkloadId,
        ) -> Result<(), SubstrateError> {
            self.record("stop")
        }

        async fn restart(
            &self,
            _owner: &str,
            _workload: &WorkloadSpec,
        ) -> Result<(), SubstrateError> {
            self.record("restart")
        }

        async fn remove(
            &self,
            _owner: &str,
            _workload_id: &WorkloadId,
        ) -> Result<(), SubstrateError> {
            self.record("remove")
        }
    }

    fn agent_deploy_envelope(
        node: &NodeIdentity,
        workload: &WorkloadSpec,
    ) -> ExecutionCommandEnvelope {
        let now = Utc::now();
        ExecutionCommandEnvelope::new(
            node.node_id().expect("node id"),
            now,
            now + Duration::minutes(5),
            ExecutionCommand::Deploy {
                supersedes_removal: None,
                workload: Box::new(workload.clone()),
            },
        )
        .expect("command")
    }

    #[tokio::test]
    async fn substrate_refusal_fails_the_receipt_and_leaves_no_ledger_entry() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let owners = paired_owner_store(&owner, &node, &dir);
        let now = Utc::now();
        let workload = WorkloadSpec::agent(
            WorkloadId::random(),
            "Refused agent",
            "fake-runtime",
            None,
            None,
            Vec::new(),
        )
        .expect("workload");
        let deploy = agent_deploy_envelope(&node, &workload);
        let substrate = Arc::new(ScriptedSubstrate::default());
        substrate.fail("deploy", SafeErrorCode::RuntimeFailed);
        let controller = ExecutionController::new().with_substrate(substrate.clone());

        let events = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &deploy),
                now,
            )
            .await
            .expect("refused deploy still yields receipts");

        assert_eq!(
            terminal_receipt(&owner, &node, &events).outcome,
            ReceiptOutcome::Failed {
                error: SafeErrorCode::RuntimeFailed
            }
        );
        assert_eq!(substrate.calls(), vec!["deploy".to_string()]);
        assert_eq!(controller.ledger().await.workload_count(), 0);
        assert_eq!(controller.ledger().await.deploy_admissions(), 0);
        assert!(controller.workload_statuses().await.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn failed_stop_keeps_the_previous_lifecycle_and_start_failure_is_fail_closed() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let owners = paired_owner_store(&owner, &node, &dir);
        let now = Utc::now();
        let workload_id = WorkloadId::random();
        let workload = WorkloadSpec::agent(
            workload_id.clone(),
            "Sticky agent",
            "fake-runtime",
            None,
            None,
            Vec::new(),
        )
        .expect("workload");
        let substrate = Arc::new(ScriptedSubstrate::default());
        let controller = ExecutionController::new().with_substrate(substrate.clone());
        let envelope = |command| {
            ExecutionCommandEnvelope::new(
                node.node_id().expect("node id"),
                now,
                now + Duration::minutes(5),
                command,
            )
            .expect("command")
        };
        controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &agent_deploy_envelope(&node, &workload)),
                now,
            )
            .await
            .expect("deploy");

        // A substrate stop failure earns a Failed receipt and must leave the
        // ledger lifecycle exactly as it was.
        substrate.fail("stop", SafeErrorCode::RuntimeFailed);
        let stop_events = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(
                    &owner,
                    &node,
                    &envelope(ExecutionCommand::Stop {
                        workload_id: workload_id.clone(),
                    }),
                ),
                now,
            )
            .await
            .expect("stop attempt");
        assert_eq!(
            terminal_receipt(&owner, &node, &stop_events).outcome,
            ReceiptOutcome::Failed {
                error: SafeErrorCode::RuntimeFailed
            }
        );
        let statuses = controller.workload_statuses().await;
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].lifecycle, WorkloadLifecycle::Running);

        // A fail-closed start (e.g. the process substrate lost the launch key
        // across a node restart) surfaces its safe code the same way.
        substrate.fail("start", SafeErrorCode::RuntimeUnavailable);
        let start_events = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(
                    &owner,
                    &node,
                    &envelope(ExecutionCommand::Start {
                        workload_id: workload_id.clone(),
                    }),
                ),
                now,
            )
            .await
            .expect("start attempt");
        assert_eq!(
            terminal_receipt(&owner, &node, &start_events).outcome,
            ReceiptOutcome::Failed {
                error: SafeErrorCode::RuntimeUnavailable
            }
        );
        let statuses = controller.workload_statuses().await;
        assert_eq!(statuses[0].lifecycle, WorkloadLifecycle::Running);
        assert_eq!(
            substrate.calls(),
            vec![
                "deploy".to_string(),
                "stop".to_string(),
                "start".to_string()
            ]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn inadmissible_commands_never_reach_the_substrate() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let owners = paired_owner_store(&owner, &node, &dir);
        let now = Utc::now();
        let workload_id = WorkloadId::random();
        let workload = WorkloadSpec::agent(
            workload_id.clone(),
            "Tombstoned agent",
            "fake-runtime",
            None,
            None,
            Vec::new(),
        )
        .expect("workload");
        let substrate = Arc::new(ScriptedSubstrate::default());
        let controller = ExecutionController::new().with_substrate(substrate.clone());
        let envelope = |command| {
            ExecutionCommandEnvelope::new(
                node.node_id().expect("node id"),
                now,
                now + Duration::minutes(5),
                command,
            )
            .expect("command")
        };

        // Start of an unknown workload fails in pure validation.
        let events = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(
                    &owner,
                    &node,
                    &envelope(ExecutionCommand::Start {
                        workload_id: workload_id.clone(),
                    }),
                ),
                now,
            )
            .await
            .expect("start of unknown workload");
        assert_eq!(
            terminal_receipt(&owner, &node, &events).outcome,
            ReceiptOutcome::Failed {
                error: SafeErrorCode::WorkloadNotFound
            }
        );
        assert!(substrate.calls().is_empty());

        // A stale redeploy blocked by a removal tombstone also stays purely
        // in the ledger.
        controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &agent_deploy_envelope(&node, &workload)),
                now,
            )
            .await
            .expect("deploy");
        let remove_events = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(
                    &owner,
                    &node,
                    &envelope(ExecutionCommand::Remove {
                        workload_id: workload_id.clone(),
                    }),
                ),
                now,
            )
            .await
            .expect("remove");
        assert_eq!(
            terminal_receipt(&owner, &node, &remove_events).outcome,
            ReceiptOutcome::Succeeded
        );
        let stale = controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &agent_deploy_envelope(&node, &workload)),
                now,
            )
            .await
            .expect("stale deploy");
        assert_eq!(
            terminal_receipt(&owner, &node, &stale).outcome,
            ReceiptOutcome::Failed {
                error: SafeErrorCode::Conflict
            }
        );
        // Exactly one deploy and one remove hit the substrate; the stale
        // deploy was refused before any side effect.
        assert_eq!(
            substrate.calls(),
            vec!["deploy".to_string(), "remove".to_string()]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn self_exits_transition_the_ledger_without_receipts_or_respawns() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let owners = paired_owner_store(&owner, &node, &dir);
        let owner_hex = owner.public_key().to_hex();
        let now = Utc::now();
        let workload_id = WorkloadId::random();
        let workload = WorkloadSpec::agent(
            workload_id.clone(),
            "Self-reaping agent",
            "fake-runtime",
            None,
            None,
            Vec::new(),
        )
        .expect("workload");
        let substrate = Arc::new(ScriptedSubstrate::default());
        let controller = ExecutionController::load(&dir)
            .expect("controller")
            .with_substrate(substrate.clone());
        controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(&owner, &node, &agent_deploy_envelope(&node, &workload)),
                now,
            )
            .await
            .expect("deploy");

        // A clean self-exit: the agent finished and left. Stopped, no respawn.
        let changed = controller
            .record_workload_exit(&WorkloadExit {
                owner: owner_hex.clone(),
                workload_id: workload_id.clone(),
                clean: true,
            })
            .await
            .expect("record clean exit");
        assert!(changed);
        let statuses = controller.workload_statuses().await;
        assert_eq!(statuses[0].lifecycle, WorkloadLifecycle::Stopped);
        // The transition is durable across a node restart.
        let restarted = ExecutionController::load(&dir).expect("restarted controller");
        assert_eq!(
            restarted.workload_statuses().await[0].lifecycle,
            WorkloadLifecycle::Stopped
        );

        // A stale duplicate exit observation changes nothing.
        let changed = controller
            .record_workload_exit(&WorkloadExit {
                owner: owner_hex.clone(),
                workload_id: workload_id.clone(),
                clean: true,
            })
            .await
            .expect("record duplicate exit");
        assert!(!changed);

        // A crash exit maps to Failed.
        controller
            .handle_command_event(
                &node,
                &owners,
                TEST_RELAY_AUTHORITY,
                &deploy_command_event(
                    &owner,
                    &node,
                    &ExecutionCommandEnvelope::new(
                        node.node_id().expect("node id"),
                        now,
                        now + Duration::minutes(5),
                        ExecutionCommand::Start {
                            workload_id: workload_id.clone(),
                        },
                    )
                    .expect("start command"),
                ),
                now,
            )
            .await
            .expect("start");
        let changed = controller
            .record_workload_exit(&WorkloadExit {
                owner: owner_hex.clone(),
                workload_id: workload_id.clone(),
                clean: false,
            })
            .await
            .expect("record crash exit");
        assert!(changed);
        assert_eq!(
            controller.workload_statuses().await[0].lifecycle,
            WorkloadLifecycle::Failed
        );

        // Unknown owners and workloads are ignored.
        let changed = controller
            .record_workload_exit(&WorkloadExit {
                owner: "not-an-owner".into(),
                workload_id: WorkloadId::random(),
                clean: true,
            })
            .await
            .expect("record unknown exit");
        assert!(!changed);
        // The substrate saw only the explicit commands — never a respawn.
        assert_eq!(
            substrate.calls(),
            vec!["deploy".to_string(), "start".to_string()]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn removal_tombstones_round_trip_with_their_sequences() {
        let workload_id = WorkloadId::random();
        let workload = WorkloadSpec::agent(
            workload_id.clone(),
            "Round-trip agent",
            "fake-runtime",
            None,
            None,
            Vec::new(),
        )
        .expect("workload");
        let mut ledger = WorkloadLedger::default();
        ledger.deploy(&workload, None).expect("deploy");
        ledger.remove(&workload_id).expect("remove");
        ledger.record_removal_sequence(&workload_id, 4);

        let encoded = serde_json::to_value(&ledger).expect("serialize ledger");
        assert_eq!(encoded["removed_workloads"][workload_id.as_str()], 4);
        assert_eq!(encoded["deploy_admissions"], 1);
        let decoded: WorkloadLedger =
            serde_json::from_value(encoded).expect("decode sequenced ledger");
        assert_eq!(decoded.deploy_admissions(), 1);
        assert!(matches!(
            decoded.clone().deploy(&workload, None),
            Err(SafeErrorCode::Conflict)
        ));
        assert!(matches!(
            decoded.clone().deploy(&workload, Some(3)),
            Err(SafeErrorCode::Conflict)
        ));
        assert_eq!(
            decoded.clone().deploy(&workload, Some(4)),
            Ok(WorkloadLifecycle::Running)
        );
    }

    #[tokio::test]
    async fn expired_command_is_rejected_without_ledger_side_effects() {
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
                supersedes_removal: None,
                workload: Box::new(
                    WorkloadSpec::agent(
                        WorkloadId::random(),
                        "Expired agent",
                        "fake-runtime",
                        None,
                        None,
                        Vec::new(),
                    )
                    .expect("workload"),
                ),
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
        assert_eq!(controller.ledger().await.workload_count(), 0);
        let _ = fs::remove_dir_all(dir);
    }
}
