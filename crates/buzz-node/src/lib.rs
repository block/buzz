//! Runtime-neutral relay client for a standalone Buzz execution node.

use std::fs;
use std::path::{Path, PathBuf};

use buzz_core::execution::{
    ExecutionCapability, ExecutionNodeId, ExecutionNodeLifecycle, ExecutionNodeStatus,
    EXECUTION_PROTOCOL_VERSION,
};
use buzz_core::kind::KIND_EXECUTION_NODE_ANNOUNCEMENT;
use nostr::{Event, EventBuilder, Keys, Kind, Tag, ToBech32};
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
        [
            ExecutionCapability::Deploy,
            ExecutionCapability::Start,
            ExecutionCapability::Stop,
            ExecutionCapability::Restart,
            ExecutionCapability::Remove,
            ExecutionCapability::ProviderAuthentication,
        ],
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
}
