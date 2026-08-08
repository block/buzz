//! Durable registration for agent runtimes owned by another launcher.
//!
//! An external runtime is deliberately not a ManagedAgentRecord. It has no
//! private key, process handle, provider configuration, or start path in Buzz
//! Desktop. The registry is provenance and operating-contract data only:
//! registering an entry can never start a second executor.

use std::{fs, path::PathBuf};

use nostr::{FromBech32, PublicKey};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::storage::{atomic_write_json_restricted, backup_invalid_store, managed_agents_base_dir};

const EXTERNAL_RUNTIMES_FILE: &str = "external-runtimes.json";
const MAX_TEXT_LEN: usize = 256;
const MAX_CHANNELS: usize = 64;
const MAX_RATE_PER_MINUTE: u32 = 60;

fn default_true() -> bool {
    true
}

/// The external runner's durable, non-secret operating contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentRuntime {
    /// The public key of the already-running agent. Buzz never stores its
    /// nsec or any credential that could start it.
    pub agent_pubkey: String,
    /// Owner recovered from the verified NIP-OA attestation.
    pub owner_pubkey: String,
    pub name: String,
    pub purpose: String,
    pub deployment_scope: String,
    pub runner_owner: String,
    pub health_source: String,
    pub shutdown_path: String,
    pub allowed_channels: Vec<String>,
    /// Safety defaults are persisted with the registration so a later UI
    /// cannot silently reinterpret an external agent as an unrestricted bot.
    pub mention_only: bool,
    pub mention_filter: bool,
    pub rate_limit_per_minute: u32,
    pub retirement_date: String,
    /// Archiving preserves the register and history. It is not a relay
    /// membership revocation and does not claim that the runner stopped.
    pub archived: bool,
    /// The NIP-OA conditions are public provenance metadata. The reusable
    /// auth tag and its signature are intentionally not persisted here.
    pub attestation_conditions: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Input for the explicit external-runtime registration command.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterExternalAgentRuntimeRequest {
    pub agent_pubkey: String,
    /// A NIP-OA auth tag signed by the current workspace owner for
    /// agent_pubkey. It is verified and then discarded.
    pub owner_auth_tag: String,
    pub name: String,
    pub purpose: String,
    pub deployment_scope: String,
    pub runner_owner: String,
    pub health_source: String,
    pub shutdown_path: String,
    pub allowed_channels: Vec<String>,
    #[serde(default = "default_true")]
    pub mention_only: bool,
    #[serde(default = "default_true")]
    pub mention_filter: bool,
    pub rate_limit_per_minute: u32,
    pub retirement_date: String,
}

/// Why an external registration cannot be activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalRuntimeConflict {
    /// The same identity is already registered in this deployment scope.
    SameScope,
    /// The same identity is registered in another live deployment scope.
    DifferentScope { existing_scope: String },
}

impl ExternalRuntimeConflict {
    /// Render the high-signal duplicate warning shown to the owner.
    pub fn message(&self, agent_pubkey: &str) -> String {
        match self {
            Self::SameScope => format!(
                "external agent {agent_pubkey} is already registered in this deployment scope"
            ),
            Self::DifferentScope { existing_scope } => format!(
                "external agent {agent_pubkey} is already active in deployment scope '{existing_scope}'; one identity may have only one live runner scope"
            ),
        }
    }
}

/// Normalize a public key supplied as either hex or npub.
pub fn normalize_public_key(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} is required"));
    }
    let key = if trimmed.starts_with("npub1") {
        PublicKey::from_bech32(trimmed).map_err(|error| format!("invalid {label} npub: {error}"))?
    } else {
        PublicKey::from_hex(trimmed)
            .map_err(|error| format!("invalid {label} hex pubkey: {error}"))?
    };
    Ok(key.to_hex())
}

fn bounded_text(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} is required"));
    }
    if trimmed.chars().count() > MAX_TEXT_LEN {
        return Err(format!(
            "{label} is too long (maximum {MAX_TEXT_LEN} characters)"
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(format!("{label} contains a control character"));
    }
    Ok(trimmed.to_string())
}

fn normalize_channels(channels: &[String]) -> Result<Vec<String>, String> {
    if channels.is_empty() {
        return Err("at least one allowed channel is required".to_string());
    }
    if channels.len() > MAX_CHANNELS {
        return Err(format!(
            "too many allowed channels (maximum {MAX_CHANNELS})"
        ));
    }
    let mut out = Vec::with_capacity(channels.len());
    for channel in channels {
        let normalized = bounded_text(channel, "allowed channel")?;
        if !out.iter().any(|existing| existing == &normalized) {
            out.push(normalized);
        }
    }
    if out.is_empty() {
        return Err("at least one allowed channel is required".to_string());
    }
    Ok(out)
}

fn attestation_conditions(auth_tag: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(auth_tag)
        .map_err(|error| format!("owner auth tag is not valid JSON: {error}"))?;
    value
        .as_array()
        .and_then(|parts| parts.get(2))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "owner auth tag is missing its conditions".to_string())
}

/// Build a validated registry record. This pure constructor is also the
/// command's security boundary and is intentionally independent of disk I/O.
pub fn build_external_agent_runtime(
    request: &RegisterExternalAgentRuntimeRequest,
    expected_owner_pubkey: &str,
    now: &str,
) -> Result<ExternalAgentRuntime, String> {
    let agent_pubkey = normalize_public_key(&request.agent_pubkey, "agent")?;
    let expected_owner_pubkey = normalize_public_key(expected_owner_pubkey, "workspace owner")?;
    let agent_compat = nostr::PublicKey::from_hex(&agent_pubkey)
        .map_err(|error| format!("invalid agent pubkey for attestation: {error}"))?;
    let attested_owner =
        buzz_sdk_pkg::nip_oa::verify_auth_tag(request.owner_auth_tag.trim(), &agent_compat)
            .map_err(|error| format!("owner auth tag verification failed: {error}"))?
            .to_hex();
    if attested_owner != expected_owner_pubkey {
        return Err("owner auth tag is not signed by the current workspace owner".to_string());
    }
    if !request.mention_only || !request.mention_filter {
        return Err(
            "external runtimes must start in mentions-only mode with the mention filter enabled"
                .to_string(),
        );
    }
    if !(1..=MAX_RATE_PER_MINUTE).contains(&request.rate_limit_per_minute) {
        return Err(format!(
            "rate limit must be between 1 and {MAX_RATE_PER_MINUTE} messages per minute"
        ));
    }

    Ok(ExternalAgentRuntime {
        agent_pubkey,
        owner_pubkey: attested_owner,
        name: bounded_text(&request.name, "agent name")?,
        purpose: bounded_text(&request.purpose, "agent purpose")?,
        deployment_scope: bounded_text(&request.deployment_scope, "deployment scope")?,
        runner_owner: bounded_text(&request.runner_owner, "runner owner")?,
        health_source: bounded_text(&request.health_source, "health source")?,
        shutdown_path: bounded_text(&request.shutdown_path, "shutdown path")?,
        allowed_channels: normalize_channels(&request.allowed_channels)?,
        mention_only: true,
        mention_filter: true,
        rate_limit_per_minute: request.rate_limit_per_minute,
        retirement_date: bounded_text(&request.retirement_date, "retirement date")?,
        archived: false,
        attestation_conditions: attestation_conditions(request.owner_auth_tag.trim())?,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    })
}

/// Find a live duplicate before any local or relay state is changed.
pub fn find_external_runtime_conflict(
    records: &[ExternalAgentRuntime],
    agent_pubkey: &str,
    deployment_scope: &str,
) -> Option<ExternalRuntimeConflict> {
    records
        .iter()
        .filter(|record| !record.archived && record.agent_pubkey == agent_pubkey)
        .map(|record| {
            if record.deployment_scope == deployment_scope {
                ExternalRuntimeConflict::SameScope
            } else {
                ExternalRuntimeConflict::DifferentScope {
                    existing_scope: record.deployment_scope.clone(),
                }
            }
        })
        .next()
}

/// The local, owner-only registry path. It is separate from
/// managed-agents.json so no existing start/reconcile code can accidentally
/// treat an external identity as a local executor.
pub fn external_runtimes_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join(EXTERNAL_RUNTIMES_FILE))
}

/// Load the owner-only external-runtime register from disk.
pub fn load_external_agent_runtimes(app: &AppHandle) -> Result<Vec<ExternalAgentRuntime>, String> {
    let path = external_runtimes_store_path(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read external runtime registry: {error}"))?;
    serde_json::from_str(&content).map_err(|error| {
        backup_invalid_store(&path);
        format!("failed to parse external runtime registry (preserved as .invalid): {error}")
    })
}

/// Persist the external-runtime register with deterministic ordering.
pub fn save_external_agent_runtimes(
    app: &AppHandle,
    records: &[ExternalAgentRuntime],
) -> Result<(), String> {
    let mut sorted = records.to_vec();
    sorted.sort_by(|left, right| {
        left.agent_pubkey
            .cmp(&right.agent_pubkey)
            .then_with(|| left.deployment_scope.cmp(&right.deployment_scope))
    });
    let payload = serde_json::to_vec_pretty(&sorted)
        .map_err(|error| format!("failed to serialize external runtime registry: {error}"))?;
    atomic_write_json_restricted(&external_runtimes_store_path(app)?, &payload)
}

/// The public body of the owner-signed kind:30177 provenance projection.
/// It carries only operating-contract fields; it never carries the NIP-OA auth
/// tag, nsec, provider config, or a process handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentRuntimeEventContent {
    pub schema_version: u8,
    pub owner_pubkey: String,
    pub name: String,
    pub purpose: String,
    pub deployment_scope: String,
    pub runner_owner: String,
    pub health_source: String,
    pub shutdown_path: String,
    pub allowed_channels: Vec<String>,
    pub mention_only: bool,
    pub mention_filter: bool,
    pub rate_limit_per_minute: u32,
    pub retirement_date: String,
    pub archived: bool,
    pub attestation_conditions: String,
}

/// Project a register entry onto the public provenance event body.
pub fn external_runtime_event_content(
    record: &ExternalAgentRuntime,
) -> ExternalAgentRuntimeEventContent {
    ExternalAgentRuntimeEventContent {
        schema_version: 1,
        owner_pubkey: record.owner_pubkey.clone(),
        name: record.name.clone(),
        purpose: record.purpose.clone(),
        deployment_scope: record.deployment_scope.clone(),
        runner_owner: record.runner_owner.clone(),
        health_source: record.health_source.clone(),
        shutdown_path: record.shutdown_path.clone(),
        allowed_channels: record.allowed_channels.clone(),
        mention_only: record.mention_only,
        mention_filter: record.mention_filter,
        rate_limit_per_minute: record.rate_limit_per_minute,
        retirement_date: record.retirement_date.clone(),
        archived: record.archived,
        attestation_conditions: record.attestation_conditions.clone(),
    }
}

/// Building and signing this event never starts or contacts a runtime.
pub fn build_external_runtime_event(
    owner_keys: &nostr::Keys,
    record: &ExternalAgentRuntime,
) -> Result<nostr::Event, String> {
    if owner_keys.public_key().to_hex() != record.owner_pubkey {
        return Err("external runtime owner does not match signing identity".to_string());
    }
    // Keep the three mandatory kind:30177 projection fields present so older
    // clients can parse the event, while the external contract lives in its
    // namespaced extension object.
    let content = serde_json::json!({
        "name": record.name,
        "parallelism": 1,
        "respond_to": "owner-only",
        "respond_to_allowlist": [],
        "external_runtime": external_runtime_event_content(record),
    })
    .to_string();
    let d_tag = nostr::Tag::parse(["d", record.agent_pubkey.as_str()])
        .map_err(|error| format!("invalid external runtime d-tag: {error}"))?;
    nostr::EventBuilder::new(
        nostr::Kind::Custom(buzz_core_pkg::kind::KIND_MANAGED_AGENT as u16),
        content,
    )
    .tags([d_tag])
    .sign_with_keys(owner_keys)
    .map_err(|error| format!("failed to sign external runtime event: {error}"))
}

/// Read and verify an external-runtime extension from a relay event.
///
/// A kind:30177 event without the namespaced extension is a normal managed
/// agent and returns `Ok(None)`. An extension must be signed by its event
/// author and carry the requested agent identity in its `d` tag; otherwise it
/// is rejected before it can suppress a duplicate-scope warning.
pub fn external_runtime_projection_from_event(
    event: &nostr::Event,
    agent_pubkey: &str,
) -> Result<Option<ExternalAgentRuntimeEventContent>, String> {
    if event.kind.as_u16() as u32 != buzz_core_pkg::kind::KIND_MANAGED_AGENT {
        return Ok(None);
    }
    let d_tag = event.tags.iter().find_map(|tag| {
        let values: Vec<&str> = tag.as_slice().iter().map(|value| value.as_str()).collect();
        (values.first() == Some(&"d")).then(|| values.get(1).copied()).flatten()
    });
    if d_tag != Some(agent_pubkey) {
        return Ok(None);
    }
    event
        .verify()
        .map_err(|error| format!("external runtime provenance event failed signature verification: {error}"))?;
    let value: serde_json::Value = serde_json::from_str(event.content.as_ref())
        .map_err(|error| format!("external runtime provenance event is not JSON: {error}"))?;
    let Some(extension) = value.get("external_runtime") else {
        return Ok(None);
    };
    let projection: ExternalAgentRuntimeEventContent =
        serde_json::from_value(extension.clone()).map_err(|error| {
            format!("external runtime provenance extension is invalid: {error}")
        })?;
    if projection.owner_pubkey != event.pubkey.to_hex() {
        return Err("external runtime provenance owner does not match event author".to_string());
    }
    Ok(Some(projection))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::ToBech32;

    fn request(agent: &nostr::Keys, owner: &nostr::Keys) -> RegisterExternalAgentRuntimeRequest {
        let agent_compat = nostr::PublicKey::from_hex(&agent.public_key().to_hex()).unwrap();
        let owner_nsec = owner.secret_key().to_bech32().unwrap();
        let owner_compat = nostr::Keys::parse(&owner_nsec).unwrap();
        let auth =
            buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner_compat, &agent_compat, "").unwrap();
        RegisterExternalAgentRuntimeRequest {
            agent_pubkey: agent.public_key().to_bech32().unwrap(),
            owner_auth_tag: auth,
            name: "Kaya".to_string(),
            purpose: "External Hermes runner".to_string(),
            deployment_scope: "hetzner-hermes".to_string(),
            runner_owner: "hermes-gateway-selim-pro".to_string(),
            health_source: "systemd + relay presence".to_string(),
            shutdown_path: "systemctl stop hermes-gateway-selim-pro".to_string(),
            allowed_channels: vec!["approvals".to_string(), "alerts".to_string()],
            mention_only: true,
            mention_filter: true,
            rate_limit_per_minute: 12,
            retirement_date: "2027-01-01".to_string(),
        }
    }

    #[test]
    fn validates_owner_attestation_and_discards_auth_tag() {
        let owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let input = request(&agent, &owner);
        let record = build_external_agent_runtime(&input, &owner.public_key().to_hex(), "now")
            .expect("valid external registration");
        assert_eq!(record.agent_pubkey, agent.public_key().to_hex());
        assert_eq!(record.owner_pubkey, owner.public_key().to_hex());
        assert!(record.attestation_conditions.is_empty());
        let serialized = serde_json::to_string(&record).unwrap();
        assert!(!serialized.contains("auth_tag"));
        assert!(!serialized.contains("sig"));
    }

    #[test]
    fn rejects_owner_mismatch_and_unsafe_noise_defaults() {
        let owner = nostr::Keys::generate();
        let other_owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let input = request(&agent, &owner);
        assert!(
            build_external_agent_runtime(&input, &other_owner.public_key().to_hex(), "now")
                .unwrap_err()
                .contains("current workspace owner")
        );

        let mut unsafe_input = request(&agent, &owner);
        unsafe_input.mention_only = false;
        assert!(
            build_external_agent_runtime(&unsafe_input, &owner.public_key().to_hex(), "now")
                .unwrap_err()
                .contains("mentions-only")
        );
    }

    #[test]
    fn blocks_same_identity_across_live_scopes_but_ignores_archived_history() {
        let records = vec![ExternalAgentRuntime {
            agent_pubkey: "a".to_string(),
            owner_pubkey: "o".to_string(),
            name: "a".to_string(),
            purpose: "p".to_string(),
            deployment_scope: "old".to_string(),
            runner_owner: "r".to_string(),
            health_source: "h".to_string(),
            shutdown_path: "s".to_string(),
            allowed_channels: vec!["alerts".to_string()],
            mention_only: true,
            mention_filter: true,
            rate_limit_per_minute: 1,
            retirement_date: "today".to_string(),
            archived: false,
            attestation_conditions: String::new(),
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
        }];
        assert!(matches!(
            find_external_runtime_conflict(&records, "a", "old"),
            Some(ExternalRuntimeConflict::SameScope)
        ));
        assert!(matches!(
            find_external_runtime_conflict(&records, "a", "new"),
            Some(ExternalRuntimeConflict::DifferentScope { .. })
        ));
        let mut archived = records;
        archived[0].archived = true;
        assert!(find_external_runtime_conflict(&archived, "a", "new").is_none());
    }

    #[test]
    fn owner_signed_projection_carries_no_runtime_secret() {
        let owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let input = request(&agent, &owner);
        let record =
            build_external_agent_runtime(&input, &owner.public_key().to_hex(), "now").unwrap();
        let event = build_external_runtime_event(&owner, &record).unwrap();
        assert_eq!(
            event.kind.as_u16() as u32,
            buzz_core_pkg::kind::KIND_MANAGED_AGENT
        );
        assert!(event.content.contains("deploymentScope"));
        assert!(!event.content.contains("ownerAuthTag"));
        assert!(!event.content.contains("private"));
    }

    #[test]
    fn relay_projection_parser_rejects_wrong_identity_and_accepts_verified_extension() {
        let owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let input = request(&agent, &owner);
        let record =
            build_external_agent_runtime(&input, &owner.public_key().to_hex(), "now").unwrap();
        let event = build_external_runtime_event(&owner, &record).unwrap();
        let other_agent = "f".repeat(64);
        assert!(external_runtime_projection_from_event(&event, &other_agent)
            .unwrap()
            .is_none());
        let projection =
            external_runtime_projection_from_event(&event, &record.agent_pubkey).unwrap();
        assert_eq!(projection.unwrap().deployment_scope, "hetzner-hermes");
    }
}
