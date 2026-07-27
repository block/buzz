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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAgentRuntimeLifecycle {
    Starting,
    Listening,
    Waking,
    Ready,
    Unknown,
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
    /// Highest encrypted runtime observer sequence accepted for this process.
    /// Prevents delayed lifecycle or lease frames from regressing state.
    pub last_runtime_observer_seq: Option<u64>,
    /// Local monotonic deadline derived from the signed lease duration.
    ///
    /// Wall-clock timestamps remain in the status payload for operators, but
    /// expiry decisions use this deadline so system clock adjustments cannot
    /// revive or prematurely expire a runtime.
    pub lease_deadline: std::time::Instant,
    /// Signed source timestamp from the newest accepted runtime lease.
    pub last_observed_at: Option<String>,
    /// Signed expiry timestamp from the newest accepted runtime lease.
    pub lease_expires_at: Option<String>,
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
            last_runtime_observer_seq: None,
            // A newly spawned process gets one lease window to initialize its
            // relay observer. If no signed lease arrives, status degrades to
            // Unknown rather than claiming the process is offline.
            lease_deadline: std::time::Instant::now()
                + std::time::Duration::from_secs(
                    buzz_core_pkg::presence::MANAGED_AGENT_RUNTIME_LEASE_TTL_SECS,
                ),
            last_observed_at: None,
            lease_expires_at: None,
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
    pub observer_sequence: Option<u64>,
    pub last_observed_at: Option<String>,
    pub lease_expires_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentRuntimeLifecycleObserverPayload {
    pub pubkey: String,
    pub relay_url: String,
    pub start_nonce: String,
    pub observer_sequence: u64,
    pub source_timestamp: String,
    pub lifecycle: ManagedAgentRuntimeLifecycle,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentRuntimeLeaseObserverPayload {
    pub pubkey: String,
    pub relay_url: String,
    pub start_nonce: String,
    pub lease_epoch: String,
    pub observer_sequence: u64,
    pub source_timestamp: String,
    pub expires_at: String,
    pub last_error: Option<String>,
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
