//! Runtime-neutral relay client for a standalone Buzz execution node.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use buzz_core::execution::{
    ExecutionCapability, ExecutionCommand, ExecutionCommandEnvelope, ExecutionNodeId,
    ExecutionNodeLifecycle, ExecutionNodeStatus, ExecutionReceipt, ReceiptOutcome, SafeErrorCode,
    WorkloadId, WorkloadLifecycle, WorkloadSpec, EXECUTION_PROTOCOL_VERSION,
};
use buzz_core::kind::{
    KIND_EXECUTION_NODE_ANNOUNCEMENT, KIND_EXECUTION_NODE_COMMAND, KIND_EXECUTION_NODE_RECEIPT,
};
use chrono::{DateTime, Utc};
use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, PublicKey, Tag, ToBech32};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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

        Ok(Self {
            relay_url,
            data_dir,
            display_name,
            auth_tag,
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

    /// Check whether an owner public key is paired.
    pub fn contains(&self, owner: &str) -> bool {
        self.owners.iter().any(|candidate| candidate == owner)
    }

    /// Return paired owner identities without exposing any private material.
    pub fn owners(&self) -> &[String] {
        &self.owners
    }
}

/// Build the safe replaceable announcement published by a node.
pub fn build_announcement(identity: &NodeIdentity, display_name: &str) -> Result<Event, NodeError> {
    let node_id = identity.node_id()?;
    let status = ExecutionNodeStatus::new(
        node_id.clone(),
        display_name,
        ExecutionNodeLifecycle::Ready,
        [ExecutionCapability::Deploy],
    )?;
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
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopPairingPayload {
    /// Workspace owner identity. The legacy `pubkey` name is accepted too.
    #[serde(alias = "pubkey")]
    pub owner_pubkey: String,
    /// Relay to use after pairing.
    pub relay_url: String,
}

/// Parse a Desktop pairing payload without retaining the private key it may contain.
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
#[derive(Debug, Default)]
pub struct FakeWorkloadRuntime {
    workloads: BTreeMap<WorkloadId, WorkloadSpec>,
    deploy_invocations: usize,
}

impl FakeWorkloadRuntime {
    /// Reconcile a deploy request into the fake runtime.
    pub fn deploy(&mut self, workload: &WorkloadSpec) -> Result<WorkloadLifecycle, SafeErrorCode> {
        let is_new = !self.workloads.contains_key(&workload.workload_id);
        self.workloads
            .insert(workload.workload_id.clone(), workload.clone());
        if is_new {
            self.deploy_invocations += 1;
        }
        Ok(WorkloadLifecycle::Running)
    }

    /// Number of distinct workload rows currently known to the runtime.
    pub fn workload_count(&self) -> usize {
        self.workloads.len()
    }

    /// Number of first-time deploy reconciliations.
    pub fn deploy_invocations(&self) -> usize {
        self.deploy_invocations
    }
}

/// Node-side command processor for the first encrypted execution slice.
#[derive(Debug, Default)]
pub struct ExecutionController {
    runtime: FakeWorkloadRuntime,
    processed: HashMap<buzz_core::execution::CommandId, Vec<ExecutionReceipt>>,
    next_sequences: BTreeMap<WorkloadId, u64>,
    data_dir: Option<PathBuf>,
}

impl ExecutionController {
    /// Load command idempotency state from a node data directory.
    pub fn load(data_dir: &Path) -> Result<Self, NodeError> {
        let path = data_dir.join("execution-state.json");
        if !path.exists() {
            return Ok(Self {
                data_dir: Some(data_dir.to_path_buf()),
                ..Self::default()
            });
        }
        let state: PersistedExecutionState = serde_json::from_str(&fs::read_to_string(path)?)?;
        Ok(Self {
            runtime: FakeWorkloadRuntime::default(),
            processed: state.processed,
            next_sequences: state.next_sequences,
            data_dir: Some(data_dir.to_path_buf()),
        })
    }

    /// Process one signed, owner-authorized command event and return its signed
    /// encrypted receipt events. Invalid or unauthorized events are ignored by
    /// the caller's subscription loop and never reach the fake runtime.
    pub fn handle_command_event(
        &mut self,
        identity: &NodeIdentity,
        owners: &OwnerStore,
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

        let owner = event.pubkey;
        let owner_hex = owner.to_hex();
        if !owners.contains(&owner_hex) {
            return Ok(Vec::new());
        }
        let node_id = identity.node_id()?;
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

        if let Some(previous) = self.processed.get(&envelope.command_id()).cloned() {
            return self.receipt_events(identity, &owner, previous);
        }

        let receipts = if envelope.node_id() != &node_id {
            vec![self.rejected_receipt(&envelope, SafeErrorCode::Unauthorized)?]
        } else if let Err(error) = envelope.validate_at(now) {
            vec![self.rejected_receipt(&envelope, safe_error_for_validation(&error))?]
        } else {
            self.execute(&envelope)?
        };

        self.processed
            .insert(envelope.command_id(), receipts.clone());
        self.persist()?;
        self.receipt_events(identity, &owner, receipts)
    }

    /// Inspect the fake runtime for diagnostics and tests.
    pub fn runtime(&self) -> &FakeWorkloadRuntime {
        &self.runtime
    }

    fn execute(
        &mut self,
        envelope: &ExecutionCommandEnvelope,
    ) -> Result<Vec<ExecutionReceipt>, NodeError> {
        let accepted = self.next_receipt(envelope, ReceiptOutcome::Accepted)?;
        let outcome = match &envelope.command {
            ExecutionCommand::Deploy { workload } => match self.runtime.deploy(workload) {
                Ok(_) => ReceiptOutcome::Succeeded,
                Err(error) => ReceiptOutcome::Failed { error },
            },
            _ => ReceiptOutcome::Failed {
                error: SafeErrorCode::Unsupported,
            },
        };
        Ok(vec![accepted, self.next_receipt(envelope, outcome)?])
    }

    fn rejected_receipt(
        &mut self,
        envelope: &ExecutionCommandEnvelope,
        error: SafeErrorCode,
    ) -> Result<ExecutionReceipt, NodeError> {
        self.next_receipt(envelope, ReceiptOutcome::Rejected { error })
    }

    fn next_receipt(
        &mut self,
        envelope: &ExecutionCommandEnvelope,
        outcome: ReceiptOutcome,
    ) -> Result<ExecutionReceipt, NodeError> {
        let workload_id = envelope.command.workload_id().clone();
        let sequence = self.next_sequences.entry(workload_id.clone()).or_insert(0);
        *sequence += 1;
        ExecutionReceipt::for_command(envelope, workload_id, *sequence, outcome)
            .map_err(|error| NodeError::InvalidCommand(error.to_string()))
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

    fn persist(&self) -> Result<(), NodeError> {
        let Some(data_dir) = &self.data_dir else {
            return Ok(());
        };
        fs::create_dir_all(data_dir)?;
        let state = PersistedExecutionState {
            processed: self.processed.clone(),
            next_sequences: self.next_sequences.clone(),
        };
        let path = data_dir.join("execution-state.json");
        fs::write(&path, serde_json::to_string_pretty(&state)?)?;
        set_private_file_permissions(&path)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedExecutionState {
    processed: HashMap<buzz_core::execution::CommandId, Vec<ExecutionReceipt>>,
    next_sequences: BTreeMap<WorkloadId, u64>,
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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("buzz-node-test-{suffix}"))
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

        let event = build_announcement(&first, "Onkie server").expect("announcement");
        let content: serde_json::Value = serde_json::from_str(&event.content).expect("json");
        assert_eq!(content["displayName"], "Onkie server");
        assert!(event.content.find("docker").is_none());
        assert!(event.content.find("privateKey").is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn desktop_pairing_payload_ignores_legacy_private_key_field() {
        let payload = serde_json::json!({
            "relayUrl": "ws://localhost:3000",
            "pubkey": "a".repeat(64),
            "nsec": "must-not-be-persisted"
        });
        let parsed = parse_desktop_pairing_payload(&payload.to_string()).expect("payload");
        assert_eq!(parsed.owner_pubkey, "a".repeat(64));
        assert!(!serde_json::to_string(&parsed)
            .expect("serialize")
            .contains("must-not-be-persisted"));
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

    #[test]
    fn encrypted_deploy_is_reconciled_once_and_receipts_are_terminal() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let mut owners = OwnerStore::default();
        owners
            .add(&owner.public_key().to_hex(), &dir)
            .expect("pair owner");
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
        let mut controller = ExecutionController::default();

        let first = controller
            .handle_command_event(&node, &owners, &event, now + Duration::seconds(1))
            .expect("first delivery");
        let second = controller
            .handle_command_event(&node, &owners, &event, now + Duration::seconds(2))
            .expect("duplicate delivery");

        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 2);
        assert_eq!(controller.runtime().workload_count(), 1);
        assert_eq!(controller.runtime().deploy_invocations(), 1);
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

    #[test]
    fn persisted_command_state_blocks_duplicate_after_restart() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let mut owners = OwnerStore::default();
        owners
            .add(&owner.public_key().to_hex(), &dir)
            .expect("pair owner");
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
        let mut first_controller = ExecutionController::load(&dir).expect("load controller");
        first_controller
            .handle_command_event(&node, &owners, &event, now + Duration::seconds(1))
            .expect("first delivery");

        let mut restarted_controller = ExecutionController::load(&dir).expect("restart controller");
        let receipts = restarted_controller
            .handle_command_event(&node, &owners, &event, now + Duration::seconds(2))
            .expect("duplicate delivery after restart");

        assert_eq!(receipts.len(), 2);
        assert_eq!(restarted_controller.runtime().workload_count(), 0);
        assert_eq!(restarted_controller.runtime().deploy_invocations(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unpaired_command_never_reaches_the_runtime() {
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
        let mut controller = ExecutionController::default();

        let receipts = controller
            .handle_command_event(&node, &OwnerStore::default(), &event, now)
            .expect("unpaired command is ignored");

        assert!(receipts.is_empty());
        assert_eq!(controller.runtime().workload_count(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn expired_command_is_rejected_without_runtime_side_effects() {
        let dir = temp_dir();
        let node = NodeIdentity::load_or_create(&dir).expect("node identity");
        let owner = Keys::generate();
        let mut owners = OwnerStore::default();
        owners
            .add(&owner.public_key().to_hex(), &dir)
            .expect("pair owner");
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
        let mut controller = ExecutionController::default();

        let receipts = controller
            .handle_command_event(&node, &owners, &event, Utc::now())
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
        assert_eq!(controller.runtime().workload_count(), 0);
        let _ = fs::remove_dir_all(dir);
    }
}
