//! Non-secret credential-persistence attestation for managed agents.
//!
//! External controllers that assign work to a named Buzz agent need to verify
//! — without ever reading key material — that the agent's credential is
//! durably held by the OS keyring, bound to exactly that agent, and not
//! sitting in the inline JSON fallback. This module produces a public,
//! deterministic attestation object for one managed agent.
//!
//! Guarantees, by construction:
//! - No secret ever enters this module: the builder takes only a boolean
//!   ("is an inline key present in the persisted record"), a keyring probe
//!   result, and public identity material. There is no field, parameter, or
//!   code path that carries the nsec.
//! - Fail closed: when the keyring is unreachable, or no credential can be
//!   located at all, the builder returns an error instead of guessing.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::secret_store::KeyringProbe;

/// Schema identifier for the v1 attestation object.
pub const AGENT_PERSISTENCE_ATTESTATION_SCHEMA_V1: &str =
    "buzz.desktop.exact_agent_credential_persistence.v1";

/// Where the agent's credential currently lives.
///
/// Extensible: additional backends (for example a secrets-provider protocol)
/// can be added as variants without breaking consumers, which are expected to
/// treat unknown strings as "not the backend I require".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistenceBackend {
    /// Credential is held by the OS keyring and absent from the JSON store.
    OsKeyring,
    /// Credential is serialized inline in the `0o600` JSON fallback file.
    InlineFile,
}

/// Public attestation of one managed agent's credential persistence state.
///
/// Field order is part of the hash contract: `attestation_hash` is the
/// SHA-256 of this struct serialized with `attestation_hash` set to the empty
/// string, so serialization must stay deterministic (serde struct-field
/// order, no maps).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPersistenceAttestation {
    pub schema_version: String,
    /// Agent identity pubkey (hex).
    pub agent_pubkey: String,
    pub persistence_backend: PersistenceBackend,
    /// True when the credential is inline in the JSON store rather than in
    /// the OS keyring. Always the negation of `persistence_backend ==
    /// os_keyring` in v1; kept explicit so consumers can gate on it directly.
    pub inline_fallback: bool,
    /// The agent record's configured parallelism (requested value).
    pub parallelism: u32,
    /// SHA-256 (hex) over the public identity material: the agent pubkey and
    /// its NIP-OA auth tag (empty string when the agent predates NIP-OA).
    pub public_identity_hash: String,
    /// SHA-256 (hex) of this attestation serialized with this field empty.
    pub attestation_hash: String,
    /// Desktop release identifier, e.g. `buzz-desktop@0.5.7`.
    pub stock_release_id: String,
    /// RFC 3339 timestamp of attestation issuance.
    pub issued_at: String,
}

/// Read-only observation of one agent's persisted credential state, collected
/// by [`crate::managed_agents::storage::observe_agent_credential_persistence`]
/// without migration side effects. Carries no secret: only presence booleans
/// and public identity material.
#[derive(Debug, Clone)]
pub(crate) struct CredentialPersistenceObservation {
    pub(crate) inline_key_present: bool,
    /// `None` when the build has no keyring backend (inline-only builds).
    pub(crate) keyring_probe: Option<KeyringProbe>,
    pub(crate) parallelism: u32,
    pub(crate) auth_tag: Option<String>,
}

/// Inputs to the pure attestation builder. Deliberately contains no secret:
/// callers report only whether an inline key is present, never its value.
#[derive(Debug, Clone)]
pub struct AttestationInputs<'a> {
    pub agent_pubkey: &'a str,
    /// The record's NIP-OA auth tag JSON, if the agent has one.
    pub auth_tag: Option<&'a str>,
    /// Whether the persisted record still carries an inline private key.
    pub inline_key_present: bool,
    /// Keyring probe for this agent's entry, or `None` when the build has no
    /// keyring backend at all (inline-only builds).
    pub keyring_probe: Option<KeyringProbe>,
    pub parallelism: u32,
    pub stock_release_id: &'a str,
    /// RFC 3339 issuance time, injected for determinism in tests.
    pub issued_at: &'a str,
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Hash of the public identity material. The auth tag is public NIP-OA JSON;
/// agents that predate NIP-OA hash the empty string in its place.
fn public_identity_hash(agent_pubkey: &str, auth_tag: Option<&str>) -> String {
    let mut material = String::with_capacity(agent_pubkey.len() + 1);
    material.push_str(agent_pubkey);
    material.push('\n');
    material.push_str(auth_tag.unwrap_or(""));
    sha256_hex(material.as_bytes())
}

/// Build the v1 attestation for one managed agent, or fail closed.
///
/// Errors (stable strings, suitable for surfacing to callers):
/// - `attestation_keyring_unreachable` — keyring backend exists but could not
///   be reached this boot and no inline key is present; presence cannot be
///   proven either way.
/// - `attestation_credential_missing` — no inline key and the keyring is
///   reachable but holds no entry for this agent.
pub fn build_agent_persistence_attestation(
    inputs: &AttestationInputs<'_>,
) -> Result<AgentPersistenceAttestation, String> {
    let (backend, inline_fallback) = if inputs.inline_key_present {
        (PersistenceBackend::InlineFile, true)
    } else {
        match inputs.keyring_probe {
            Some(KeyringProbe::Present) => (PersistenceBackend::OsKeyring, false),
            Some(KeyringProbe::ReachableButEmpty) | None => {
                return Err("attestation_credential_missing".to_string());
            }
            Some(KeyringProbe::Unreachable) => {
                return Err("attestation_keyring_unreachable".to_string());
            }
        }
    };

    let mut attestation = AgentPersistenceAttestation {
        schema_version: AGENT_PERSISTENCE_ATTESTATION_SCHEMA_V1.to_string(),
        agent_pubkey: inputs.agent_pubkey.to_string(),
        persistence_backend: backend,
        inline_fallback,
        parallelism: inputs.parallelism,
        public_identity_hash: public_identity_hash(inputs.agent_pubkey, inputs.auth_tag),
        attestation_hash: String::new(),
        stock_release_id: inputs.stock_release_id.to_string(),
        issued_at: inputs.issued_at.to_string(),
    };
    let preimage = serde_json::to_vec(&attestation)
        .map_err(|error| format!("attestation_serialize_failed: {error}"))?;
    attestation.attestation_hash = sha256_hex(&preimage);
    Ok(attestation)
}

/// Verify that `attestation.attestation_hash` matches its own payload.
/// External consumers can re-implement this from the schema; it is exposed
/// here so desktop tests and callers share one definition.
pub fn verify_attestation_hash(attestation: &AgentPersistenceAttestation) -> bool {
    let mut copy = attestation.clone();
    copy.attestation_hash = String::new();
    match serde_json::to_vec(&copy) {
        Ok(preimage) => sha256_hex(&preimage) == attestation.attestation_hash,
        Err(_) => false,
    }
}

#[cfg(test)]
#[path = "persistence_attestation_tests.rs"]
mod tests;
