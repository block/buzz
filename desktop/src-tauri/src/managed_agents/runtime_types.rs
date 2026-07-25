use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::ManagedAgentProcess;

/// Canonical identity of one managed-agent harness on one relay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentRuntimeKey {
    pub pubkey: String,
    pub relay_url: String,
}

impl ManagedAgentRuntimeKey {
    pub fn new(pubkey: impl Into<String>, relay_url: &str) -> Result<Self, String> {
        let pubkey = pubkey.into();
        if pubkey.len() != 64 || !pubkey.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("managed-agent pubkey must be 64 hexadecimal characters".into());
        }
        Ok(Self {
            pubkey: pubkey.to_ascii_lowercase(),
            relay_url: buzz_core_pkg::relay::normalize_relay_url(relay_url)
                .map_err(|error| error.to_string())?,
        })
    }

    /// Stable opaque identifier/path suffix derived only from canonical fields.
    pub fn runtime_id(&self) -> String {
        let relay_hash = hex::encode(Sha256::digest(self.relay_url.as_bytes()));
        format!("{}__{relay_hash}", self.pubkey)
    }
}

/// Validated routing target for one managed-agent harness.
///
/// `key` is canonical identity used for maps, receipts, status correlation, and
/// deduplication. `requested_relay_url` preserves the configured authority used
/// for network I/O; these may intentionally differ for loopback spellings whose
/// Host value selects different Buzz communities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedAgentRuntimeTarget {
    pub key: ManagedAgentRuntimeKey,
    pub requested_relay_url: String,
}

impl ManagedAgentRuntimeTarget {
    pub fn new(pubkey: impl Into<String>, relay_url: &str) -> Result<Self, String> {
        let requested_relay_url = relay_url.trim().to_string();
        let key = ManagedAgentRuntimeKey::new(pubkey, &requested_relay_url)?;
        Ok(Self {
            key,
            requested_relay_url,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAgentRuntimeLifecycle {
    Starting,
    Listening,
    Waking,
    Ready,
    Failed,
    Stopped,
}

#[derive(Debug)]
pub struct ManagedAgentPairRuntime {
    pub process: ManagedAgentProcess,
    pub lifecycle: ManagedAgentRuntimeLifecycle,
    pub error: Option<String>,
    /// Configured relay authority used by this process for network I/O. This is
    /// deliberately distinct from the canonical relay URL in its map key.
    pub requested_relay_url: String,
    /// Unpredictable identity for this exact harness generation. Lifecycle
    /// frames from prior processes are rejected even when the pair is live.
    pub start_nonce: String,
}

impl std::ops::Deref for ManagedAgentPairRuntime {
    type Target = ManagedAgentProcess;

    fn deref(&self) -> &Self::Target {
        &self.process
    }
}

impl std::ops::DerefMut for ManagedAgentPairRuntime {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.process
    }
}

impl ManagedAgentPairRuntime {
    pub fn starting(process: ManagedAgentProcess, requested_relay_url: String) -> Self {
        let start_nonce = process.start_nonce.clone();
        Self {
            process,
            lifecycle: ManagedAgentRuntimeLifecycle::Starting,
            error: None,
            requested_relay_url,
            start_nonce,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentRuntimeStatus {
    pub pubkey: String,
    pub relay_url: String,
    /// Configured relay authority used for network I/O. Live runtime rows keep
    /// this across status refreshes while `relay_url` remains canonical identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_relay_url: Option<String>,
    pub local_setup: bool,
    pub lifecycle: ManagedAgentRuntimeLifecycle,
    pub pid: Option<u32>,
    pub error: Option<String>,
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentRuntimeLifecycleObserverPayload {
    pub pubkey: String,
    pub relay_url: String,
    pub start_nonce: String,
    pub lifecycle: ManagedAgentRuntimeLifecycle,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentCommunityTarget {
    pub relay_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentRuntimeReceipt {
    pub key: ManagedAgentRuntimeKey,
    pub pid: u32,
    pub desktop_instance_id: String,
    pub started_at: String,
}
