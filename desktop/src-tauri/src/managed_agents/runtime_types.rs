use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use url::{Host, Url};

use super::ManagedAgentProcess;

pub(crate) const RUNTIME_AUTHORITY_RECEIPT_VERSION: u8 = 1;

/// Canonicalize only URL syntax that cannot distinguish relay authorities.
///
/// In particular, loopback host spellings stay literal: relay tenancy and
/// lifecycle fences distinguish `localhost`, `127.*`, and `::1`. The shared
/// buzz-core normalizer predates that boundary and deliberately remains in use
/// by Bestie and migration consumers whose compatibility rules differ.
fn normalize_runtime_relay_url(raw: &str) -> Result<String, String> {
    let mut url = Url::parse(raw.trim()).map_err(|error| format!("invalid relay URL: {error}"))?;
    if !matches!(url.scheme(), "ws" | "wss") {
        return Err("relay URL scheme must be ws or wss".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("relay URL must not contain credentials".into());
    }
    if url.fragment().is_some() {
        return Err("relay URL must not contain a fragment".into());
    }

    let host = url
        .host()
        .ok_or_else(|| "relay URL must contain a host".to_string())?;
    if let Host::Domain(domain) = host {
        let lowercase = domain.to_ascii_lowercase();
        url.set_host(Some(&lowercase))
            .map_err(|_| "relay URL must contain a host".to_string())?;
    }

    let default_port = match url.scheme() {
        "ws" => Some(80),
        "wss" => Some(443),
        _ => None,
    };
    if url.port() == default_port {
        url.set_port(None)
            .map_err(|_| "relay URL scheme must be ws or wss".to_string())?;
    }
    let host = match url
        .host()
        .ok_or_else(|| "relay URL must contain a host".to_string())?
    {
        Host::Domain(domain) => domain.to_string(),
        Host::Ipv4(address) => address.to_string(),
        Host::Ipv6(address) => format!("[{address}]"),
    };
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    let path = if url.path() == "/" { "" } else { url.path() };
    let query = url
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    Ok(format!("{}://{host}{port}{path}{query}", url.scheme()))
}

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
            relay_url: normalize_runtime_relay_url(relay_url)?,
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
    /// Version 0 is an unversioned legacy receipt. Its lossy host/path rendering
    /// cannot prove pair authority; it is usable only for instance-wide cleanup.
    #[serde(default)]
    pub authority_version: u8,
    pub key: ManagedAgentRuntimeKey,
    pub pid: u32,
    pub desktop_instance_id: String,
    pub started_at: String,
}

impl ManagedAgentRuntimeReceipt {
    pub(crate) fn new(
        key: ManagedAgentRuntimeKey,
        pid: u32,
        desktop_instance_id: String,
        started_at: String,
    ) -> Self {
        Self {
            authority_version: RUNTIME_AUTHORITY_RECEIPT_VERSION,
            key,
            pid,
            desktop_instance_id,
            started_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(relay_url: &str) -> ManagedAgentRuntimeKey {
        ManagedAgentRuntimeKey::new("aa".repeat(32), relay_url).unwrap()
    }

    #[test]
    fn runtime_identity_preserves_distinct_loopback_authorities() {
        let localhost = key("ws://localhost:3000");
        let ipv4 = key("ws://127.0.0.1:3000");
        let other_ipv4 = key("ws://127.0.0.2:3000");
        let ipv6 = key("ws://[::1]:3000");

        assert_eq!(localhost.relay_url, "ws://localhost:3000");
        assert_eq!(ipv4.relay_url, "ws://127.0.0.1:3000");
        assert_eq!(other_ipv4.relay_url, "ws://127.0.0.2:3000");
        assert_eq!(ipv6.relay_url, "ws://[::1]:3000");
        assert_ne!(localhost, ipv4);
        assert_ne!(ipv4, other_ipv4);
        assert_ne!(ipv4, ipv6);
    }

    #[test]
    fn runtime_identity_preserves_paths_queries_and_meaningful_trailing_slashes() {
        assert_eq!(
            key(" WSS://Relay.Example:443/community/?mode=one ").relay_url,
            "wss://relay.example/community/?mode=one"
        );
        assert_ne!(
            key("wss://relay.example/community").relay_url,
            key("wss://relay.example/community/").relay_url
        );
        assert_eq!(
            key("wss://relay.example/?").relay_url,
            "wss://relay.example?"
        );
        assert_ne!(
            key("wss://relay.example").relay_url,
            key("wss://relay.example/?").relay_url
        );
    }

    #[test]
    fn runtime_identity_rejects_non_websocket_credentials_and_fragments() {
        for relay_url in [
            "https://relay.example",
            "wss://user@relay.example",
            "wss://relay.example/#",
            "wss://relay.example/#fragment",
        ] {
            assert!(ManagedAgentRuntimeKey::new("aa".repeat(32), relay_url).is_err());
        }
    }

    #[test]
    fn receipt_authority_version_distinguishes_new_and_unversioned_receipts() {
        let receipt = ManagedAgentRuntimeReceipt::new(
            key("ws://localhost:3000"),
            42,
            "instance".into(),
            "now".into(),
        );
        assert_eq!(receipt.authority_version, RUNTIME_AUTHORITY_RECEIPT_VERSION);

        let legacy: ManagedAgentRuntimeReceipt = serde_json::from_value(serde_json::json!({
            "key": receipt.key,
            "pid": 42,
            "desktopInstanceId": "instance",
            "startedAt": "now"
        }))
        .unwrap();
        assert_eq!(legacy.authority_version, 0);
    }
}
