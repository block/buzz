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

/// Canonical process identity paired with the exact relay URL used for I/O.
///
/// `normalize_relay_url` rewrites `localhost` to `127.0.0.1` so that one agent on
/// one relay always hashes to the same runtime id. That canonical form is right
/// for identity and wrong for connecting: the relay derives the community from
/// the request host, so `localhost:3000` and `127.0.0.1:3000` are *different
/// tenants*. Connecting on the canonicalized host lands the agent in an empty
/// community, where it discovers no channels and sits idle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedAgentRuntimeTarget {
    pub key: ManagedAgentRuntimeKey,
    pub connection_url: String,
}

impl ManagedAgentRuntimeTarget {
    pub fn new(pubkey: impl Into<String>, relay_url: &str) -> Result<Self, String> {
        let connection_url = relay_url.trim().trim_end_matches('/').to_string();
        let key = ManagedAgentRuntimeKey::new(pubkey, &connection_url)?;
        Ok(Self {
            connection_url,
            key,
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
    pub fn starting(process: ManagedAgentProcess) -> Self {
        let start_nonce = process.start_nonce.clone();
        Self {
            process,
            lifecycle: ManagedAgentRuntimeLifecycle::Starting,
            error: None,
            start_nonce,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentRuntimeStatus {
    pub pubkey: String,
    pub relay_url: String,
    /// Exact descriptor URL echoed only by reconcile result rows so callers can
    /// correlate a canonical response without normalizing on the frontend.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localhost_connection_authority_is_not_replaced_by_identity_canonicalization() {
        let target =
            ManagedAgentRuntimeTarget::new("aa".repeat(32), "ws://localhost:3000/").unwrap();

        // Identity stays canonical so the runtime id is stable across spellings.
        assert_eq!(target.key.relay_url, "ws://127.0.0.1:3000");
        // The connection keeps the authority the operator configured, because the
        // relay resolves the community from that host.
        assert_eq!(target.connection_url, "ws://localhost:3000");
    }

    #[test]
    fn explicit_loopback_authority_is_preserved_verbatim() {
        let target =
            ManagedAgentRuntimeTarget::new("bb".repeat(32), "ws://127.0.0.1:3000").unwrap();

        assert_eq!(target.key.relay_url, "ws://127.0.0.1:3000");
        assert_eq!(target.connection_url, "ws://127.0.0.1:3000");
    }

    #[test]
    fn invalid_pubkey_is_rejected() {
        assert!(ManagedAgentRuntimeTarget::new("../not-a-key", "ws://localhost:3000").is_err());
    }
}
