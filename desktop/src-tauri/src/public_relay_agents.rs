use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const PUBLIC_RELAY_AGENTS_REGISTRY_VERSION: u32 = 1;
const PUBLIC_RELAY_AGENTS_FILE: &str = "public-relay-agents.json";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicRelayAgentRegistry {
    version: u32,
    agents: Vec<PublicRelayAgentRegistration>,
}

/// Lifecycle state written by the local public Relay Agent provisioner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PublicRelayAgentState {
    Provisioning,
    Active,
    Failed,
}

/// Non-secret public Agent metadata projected into the Desktop app-data directory.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicRelayAgentRegistration {
    pub id: String,
    pub name: String,
    pub pubkey: String,
    pub channel_ids: Vec<String>,
    pub state: PublicRelayAgentState,
    pub enabled: bool,
}

fn public_relay_agents_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data dir: {error}"))?
        .join("agents")
        .join(PUBLIC_RELAY_AGENTS_FILE))
}

fn valid_agent_id(id: &str) -> bool {
    let mut chars = id.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn valid_hex_pubkey(pubkey: &str) -> bool {
    pubkey.len() == 64 && pubkey.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_registration(
    registration: &mut PublicRelayAgentRegistration,
    ids: &mut HashSet<String>,
    pubkeys: &mut HashSet<String>,
) -> Result<(), String> {
    registration.id = registration.id.trim().to_string();
    if registration.id.is_empty() {
        return Err("public relay agent id must not be empty".to_string());
    }
    if !valid_agent_id(&registration.id) {
        return Err(format!(
            "invalid public relay agent id '{}': expected lowercase slug",
            registration.id
        ));
    }
    if !ids.insert(registration.id.clone()) {
        return Err(format!(
            "duplicate public relay agent id: {}",
            registration.id
        ));
    }

    registration.name = registration.name.trim().to_string();
    if registration.name.is_empty() {
        return Err(format!(
            "public relay agent '{}' name must not be empty",
            registration.id
        ));
    }

    registration.pubkey = registration.pubkey.trim().to_ascii_lowercase();
    if !valid_hex_pubkey(&registration.pubkey) {
        return Err(format!(
            "public relay agent '{}' pubkey must be 64 hexadecimal characters",
            registration.id
        ));
    }
    if !pubkeys.insert(registration.pubkey.clone()) {
        return Err(format!(
            "duplicate public relay agent pubkey: {}",
            registration.pubkey
        ));
    }

    let mut channel_ids = HashSet::new();
    registration.channel_ids = registration
        .channel_ids
        .drain(..)
        .map(|channel_id| channel_id.trim().to_string())
        .filter(|channel_id| !channel_id.is_empty())
        .filter(|channel_id| channel_ids.insert(channel_id.clone()))
        .collect();
    if registration.channel_ids.is_empty() {
        return Err(format!(
            "public relay agent '{}' must declare at least one channel",
            registration.id
        ));
    }

    Ok(())
}

fn load_public_relay_agents_from_path(
    path: &Path,
) -> Result<Vec<PublicRelayAgentRegistration>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read public relay agent registry: {error}"))?;
    let mut registry: PublicRelayAgentRegistry = serde_json::from_str(&content)
        .map_err(|error| format!("failed to parse public relay agent registry: {error}"))?;
    if registry.version != PUBLIC_RELAY_AGENTS_REGISTRY_VERSION {
        return Err(format!(
            "unsupported public relay agent registry version {}",
            registry.version
        ));
    }

    let mut ids = HashSet::new();
    let mut pubkeys = HashSet::new();
    for registration in &mut registry.agents {
        validate_registration(registration, &mut ids, &mut pubkeys)?;
    }

    Ok(registry.agents)
}

/// Loads the fixed public Relay Agent registry projection for this Desktop app.
#[tauri::command]
pub fn list_public_relay_agents(
    app: AppHandle,
) -> Result<Vec<PublicRelayAgentRegistration>, String> {
    load_public_relay_agents_from_path(&public_relay_agents_path(&app)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_registry(path: &std::path::Path, value: &str) {
        std::fs::write(path, value).expect("write registry fixture");
    }

    #[test]
    fn missing_registry_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result =
            load_public_relay_agents_from_path(&dir.path().join("missing.json")).expect("load");

        assert!(result.is_empty());
    }

    #[test]
    fn valid_registry_normalizes_pubkey_and_deduplicates_channels() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("public-relay-agents.json");
        write_registry(
            &path,
            &format!(
                r#"{{
                  "version": 1,
                  "agents": [{{
                    "id": "product",
                    "name": "Product Agent",
                    "pubkey": "{}",
                    "channelIds": ["channel-a", "channel-a", "channel-b"],
                    "state": "active",
                    "enabled": true
                  }}]
                }}"#,
                "A".repeat(64),
            ),
        );

        let result = load_public_relay_agents_from_path(&path).expect("valid registry");

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pubkey, "a".repeat(64));
        assert_eq!(result[0].channel_ids, vec!["channel-a", "channel-b"]);
    }

    #[test]
    fn unsupported_version_fails_loudly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("public-relay-agents.json");
        write_registry(&path, r#"{"version":2,"agents":[]}"#);

        let error = load_public_relay_agents_from_path(&path).expect_err("version must fail");

        assert!(error.contains("unsupported public relay agent registry version 2"));
    }

    #[test]
    fn duplicate_id_and_pubkey_fail_loudly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("public-relay-agents.json");
        write_registry(
            &path,
            &format!(
                r#"{{
                  "version": 1,
                  "agents": [
                    {{
                      "id": "one",
                      "name": "One",
                      "pubkey": "{}",
                      "channelIds": ["channel-a"],
                      "state": "active",
                      "enabled": true
                    }},
                    {{
                      "id": "one",
                      "name": "Two",
                      "pubkey": "{}",
                      "channelIds": ["channel-b"],
                      "state": "active",
                      "enabled": true
                    }}
                  ]
                }}"#,
                "a".repeat(64),
                "b".repeat(64),
            ),
        );

        let id_error = load_public_relay_agents_from_path(&path).expect_err("duplicate id");
        assert!(id_error.contains("duplicate public relay agent id: one"));

        write_registry(
            &path,
            &format!(
                r#"{{
                  "version": 1,
                  "agents": [
                    {{
                      "id": "one",
                      "name": "One",
                      "pubkey": "{}",
                      "channelIds": ["channel-a"],
                      "state": "active",
                      "enabled": true
                    }},
                    {{
                      "id": "two",
                      "name": "Two",
                      "pubkey": "{}",
                      "channelIds": ["channel-b"],
                      "state": "active",
                      "enabled": true
                    }}
                  ]
                }}"#,
                "a".repeat(64),
                "a".repeat(64),
            ),
        );

        let pubkey_error = load_public_relay_agents_from_path(&path).expect_err("duplicate pubkey");
        assert!(pubkey_error.contains("duplicate public relay agent pubkey"));
    }

    #[test]
    fn empty_identity_fields_and_channels_are_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("public-relay-agents.json");
        write_registry(
            &path,
            &format!(
                r#"{{
                  "version": 1,
                  "agents": [{{
                    "id": " ",
                    "name": "Product Agent",
                    "pubkey": "{}",
                    "channelIds": [],
                    "state": "active",
                    "enabled": true
                  }}]
                }}"#,
                "a".repeat(64),
            ),
        );

        let error = load_public_relay_agents_from_path(&path).expect_err("invalid record");

        assert!(error.contains("id must not be empty"));
    }
}
