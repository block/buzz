//! Mesh owner identity for admission.
//!
//! Each machine gets a mesh-llm owner keypair (ed25519, distinct from the
//! Buzz/Nostr identity). The node presents a signed ownership attestation
//! binding `owner_id -> endpoint_id`, and serve nodes enforce an allowlist of
//! member owner ids (see `DesktopMeshRuntime::start`). The keystore lives at
//! mesh-llm's default path (`~/.mesh-llm/owner-keystore.json`) so a machine
//! has one owner identity whether mesh runs embedded in Buzz or standalone.

use std::path::PathBuf;
use std::sync::OnceLock;

use mesh_llm_host_runtime::crypto::{
    default_keystore_path, keystore_exists, load_keystore, save_keystore, OwnerKeypair,
};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct OwnerIdentity {
    pub keystore_path: PathBuf,
    pub owner_id: String,
    pub verifying_key_hex: String,
}

impl OwnerIdentity {
    /// Sign a Buzz-to-MeshLLM ownership binding. The member's Nostr signature
    /// authenticates the discovery event; this Ed25519 signature proves the
    /// advertised owner id is backed by the MeshLLM owner key itself.
    pub fn sign_member_binding(&self, member_pubkey: &str) -> anyhow::Result<String> {
        let keypair = load_keystore(&self.keystore_path, None).map_err(|error| {
            anyhow::anyhow!("failed to load mesh owner keystore for binding: {error}")
        })?;
        Ok(hex::encode(
            keypair.sign_bytes(&member_binding_bytes(member_pubkey)),
        ))
    }

    /// Sign the exact endpoint tokens advertised by this member. This prevents
    /// a holder of only the Nostr member key from reusing a valid owner binding
    /// while substituting an attacker-selected dial target.
    pub fn sign_member_endpoint_binding(
        &self,
        member_pubkey: &str,
        endpoint_tokens: &[String],
    ) -> anyhow::Result<String> {
        let keypair = load_keystore(&self.keystore_path, None).map_err(|error| {
            anyhow::anyhow!("failed to load mesh owner keystore for endpoint binding: {error}")
        })?;
        Ok(hex::encode(keypair.sign_bytes(
            &member_endpoint_binding_bytes(member_pubkey, endpoint_tokens),
        )))
    }
}

pub fn member_binding_bytes(member_pubkey: &str) -> Vec<u8> {
    format!(
        "buzz-mesh-owner-binding-v1:{}",
        member_pubkey.trim().to_ascii_lowercase()
    )
    .into_bytes()
}

/// Canonical bytes binding a member-associated node identity to the exact set
/// of endpoint tokens in its status event.
pub fn member_endpoint_binding_bytes(member_pubkey: &str, endpoint_tokens: &[String]) -> Vec<u8> {
    let mut endpoints = endpoint_tokens
        .iter()
        .map(|token| token.trim())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    endpoints.sort_unstable();
    endpoints.dedup();

    let mut digest = Sha256::new();
    for endpoint in endpoints {
        digest.update((endpoint.len() as u64).to_be_bytes());
        digest.update(endpoint.as_bytes());
    }
    format!(
        "buzz-mesh-owner-endpoint-binding-v1:{}:{}",
        member_pubkey.trim().to_ascii_lowercase(),
        hex::encode(digest.finalize())
    )
    .into_bytes()
}

/// Extract endpoint tokens from a status payload using the same canonical
/// field rules for publication and verification.
pub fn advertised_endpoint_tokens(payload: &serde_json::Value) -> Option<Vec<String>> {
    let Some(targets) = payload
        .get("serveTargets")
        .or_else(|| payload.get("serve_targets"))
    else {
        return Some(Vec::new());
    };
    let targets = targets.as_array()?;
    targets
        .iter()
        .map(|target| {
            target
                .get("endpointAddr")
                .or_else(|| target.get("endpoint_addr"))?
                .as_str()
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(ToString::to_string)
        })
        .collect()
}

fn owner_identity(path: PathBuf, keypair: &OwnerKeypair) -> OwnerIdentity {
    OwnerIdentity {
        owner_id: keypair.owner_id(),
        verifying_key_hex: hex::encode(keypair.verifying_key().as_bytes()),
        keystore_path: path,
    }
}

/// Load-or-generate the machine's mesh owner identity. Cached for the process
/// lifetime — the keystore is stable once created.
pub fn ensure_owner_identity() -> anyhow::Result<OwnerIdentity> {
    static CACHE: OnceLock<Result<OwnerIdentity, String>> = OnceLock::new();
    CACHE
        .get_or_init(|| ensure_owner_identity_uncached().map_err(|error| format!("{error:#}")))
        .clone()
        .map_err(|error| anyhow::anyhow!(error))
}

fn ensure_owner_identity_uncached() -> anyhow::Result<OwnerIdentity> {
    let path = default_keystore_path()
        .map_err(|error| anyhow::anyhow!("cannot resolve mesh owner keystore path: {error}"))?;
    if keystore_exists(&path) {
        let keypair = load_keystore(&path, None).map_err(|error| {
            anyhow::anyhow!(
                "failed to load mesh owner keystore at {}: {error}",
                path.display()
            )
        })?;
        return Ok(owner_identity(path, &keypair));
    }
    let keypair = OwnerKeypair::generate();
    save_keystore(&path, &keypair, None, false).map_err(|error| {
        anyhow::anyhow!(
            "failed to save mesh owner keystore at {}: {error}",
            path.display()
        )
    })?;
    Ok(owner_identity(path, &keypair))
}
