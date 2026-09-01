use serde::{Deserialize, Serialize};
use tokio::time::{timeout, Duration};

const PROFILE_INVENTORY_SCHEMA: &str = "hermes-profile-list/v1";
const MAX_PROFILE_COUNT: usize = 256;
const MAX_PROFILE_NAME_CHARS: usize = 64;
const MAX_DISPLAY_NAME_CHARS: usize = 128;
const MAX_DESCRIPTION_CHARS: usize = 2_000;
const MAX_MODEL_CHARS: usize = 256;
const MAX_PROVIDER_CHARS: usize = 128;
const MAX_ALIAS_CHARS: usize = 64;
const MAX_DISTRIBUTION_CHARS: usize = 256;
const MAX_INVENTORY_BYTES: usize = 1_048_576;
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Deserialize)]
struct RawProfileInventory {
    schema: String,
    active_profile: String,
    profiles: Vec<RawHermesProfile>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawHermesProfile {
    name: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    description_auto: bool,
    #[serde(default)]
    is_default: bool,
    #[serde(default)]
    active: bool,
    model: Option<String>,
    provider: Option<String>,
    #[serde(default)]
    gateway_running: bool,
    alias: Option<String>,
    distribution: Option<RawHermesProfileDistribution>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawHermesProfileDistribution {
    name: Option<String>,
    version: Option<String>,
    source: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HermesProfileInventory {
    pub active_profile: String,
    pub profiles: Vec<HermesProfileInfo>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HermesProfileInfo {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub description_auto: bool,
    pub is_default: bool,
    pub active: bool,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub gateway_running: bool,
    pub alias: Option<String>,
    pub distribution: Option<HermesProfileDistribution>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HermesProfileDistribution {
    pub name: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
}

fn valid_profile_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
        && name.chars().count() <= MAX_PROFILE_NAME_CHARS
}

fn validate_text(value: &str, field: &str, max_chars: usize) -> Result<(), String> {
    if value.chars().count() > max_chars {
        return Err(format!(
            "Hermes profile inventory field {field} exceeds {max_chars} characters"
        ));
    }
    if value.contains('\0') {
        return Err(format!(
            "Hermes profile inventory field {field} contains a NUL byte"
        ));
    }
    Ok(())
}

fn validate_optional_text(
    value: Option<&str>,
    field: &str,
    max_chars: usize,
) -> Result<(), String> {
    if let Some(value) = value {
        validate_text(value, field, max_chars)?;
    }
    Ok(())
}

pub(crate) fn parse_profile_inventory(input: &str) -> Result<HermesProfileInventory, String> {
    if input.len() > MAX_INVENTORY_BYTES {
        return Err("Hermes profile inventory exceeds the 1 MiB limit".to_string());
    }
    let raw: RawProfileInventory = serde_json::from_str(input)
        .map_err(|error| format!("Hermes profile inventory is invalid JSON: {error}"))?;
    if raw.schema != PROFILE_INVENTORY_SCHEMA {
        return Err(format!(
            "unsupported Hermes profile inventory schema {:?}",
            raw.schema
        ));
    }
    if raw.profiles.len() > MAX_PROFILE_COUNT {
        return Err(format!(
            "Hermes profile inventory contains {} rows; maximum is {MAX_PROFILE_COUNT}",
            raw.profiles.len()
        ));
    }
    if !valid_profile_name(&raw.active_profile) {
        return Err("Hermes active profile has an invalid canonical name".to_string());
    }

    let mut seen = std::collections::HashSet::new();
    let mut profiles = Vec::with_capacity(raw.profiles.len());
    for profile in raw.profiles {
        if !valid_profile_name(&profile.name) {
            return Err(format!(
                "Hermes profile has an invalid canonical name: {:?}",
                profile.name
            ));
        }
        if !seen.insert(profile.name.clone()) {
            return Err(format!(
                "Hermes profile inventory contains duplicate profile {:?}",
                profile.name
            ));
        }
        validate_text(
            &profile.display_name,
            "display_name",
            MAX_DISPLAY_NAME_CHARS,
        )?;
        validate_text(&profile.description, "description", MAX_DESCRIPTION_CHARS)?;
        validate_optional_text(profile.model.as_deref(), "model", MAX_MODEL_CHARS)?;
        validate_optional_text(profile.provider.as_deref(), "provider", MAX_PROVIDER_CHARS)?;
        validate_optional_text(profile.alias.as_deref(), "alias", MAX_ALIAS_CHARS)?;

        let distribution = profile
            .distribution
            .map(|distribution| {
                validate_optional_text(
                    distribution.name.as_deref(),
                    "distribution.name",
                    MAX_DISTRIBUTION_CHARS,
                )?;
                validate_optional_text(
                    distribution.version.as_deref(),
                    "distribution.version",
                    MAX_DISTRIBUTION_CHARS,
                )?;
                validate_optional_text(
                    distribution.source.as_deref(),
                    "distribution.source",
                    MAX_DISTRIBUTION_CHARS,
                )?;
                Ok::<_, String>(HermesProfileDistribution {
                    name: distribution.name,
                    version: distribution.version,
                    source: distribution.source,
                })
            })
            .transpose()?;

        profiles.push(HermesProfileInfo {
            name: profile.name,
            display_name: profile.display_name,
            description: profile.description,
            description_auto: profile.description_auto,
            is_default: profile.is_default,
            active: profile.active,
            model: profile.model,
            provider: profile.provider,
            gateway_running: profile.gateway_running,
            alias: profile.alias,
            distribution,
        });
    }

    if !profiles
        .iter()
        .any(|profile| profile.name == raw.active_profile)
    {
        return Err("Hermes active profile is absent from the inventory".to_string());
    }

    Ok(HermesProfileInventory {
        active_profile: raw.active_profile,
        profiles,
    })
}

#[tauri::command]
pub async fn discover_hermes_profiles() -> Result<HermesProfileInventory, String> {
    let command = super::discovery::resolve_command("hermes")
        .ok_or_else(|| "Hermes is not installed or is not available on PATH".to_string())?;
    let mut process = tokio::process::Command::new(command);
    process
        .args(["profile", "list", "--json"])
        .kill_on_drop(true);
    let output = timeout(DISCOVERY_TIMEOUT, process.output())
        .await
        .map_err(|_| "Hermes profile discovery timed out after 15 seconds".to_string())?
        .map_err(|error| format!("failed to start Hermes profile discovery: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let excerpt = stderr.trim().chars().take(1_000).collect::<String>();
        return Err(format!(
            "Hermes profile discovery failed (exit {}): {}",
            output
                .status
                .code()
                .map_or_else(|| "signal".to_string(), |code| code.to_string()),
            if excerpt.is_empty() {
                "no diagnostic output"
            } else {
                excerpt.as_str()
            }
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "Hermes profile discovery returned non-UTF-8 output".to_string())?;
    parse_profile_inventory(&stdout)
}

#[cfg(test)]
mod tests {
    use super::parse_profile_inventory;

    #[test]
    fn parses_versioned_path_free_profile_inventory() {
        let inventory = parse_profile_inventory(
            r#"{
                "schema":"hermes-profile-list/v1",
                "active_profile":"default",
                "profiles":[
                  {
                    "name":"default",
                    "display_name":"",
                    "description":"",
                    "description_auto":false,
                    "is_default":true,
                    "active":true,
                    "model":null,
                    "provider":null,
                    "gateway_running":true,
                    "alias":null,
                    "distribution":null
                  },
                  {
                    "name":"jake",
                    "display_name":"Jake",
                    "description":"Implementation agent",
                    "description_auto":false,
                    "is_default":false,
                    "active":false,
                    "model":"x-ai/grok-4.6",
                    "provider":"openrouter",
                    "gateway_running":false,
                    "alias":"jake",
                    "distribution":null,
                    "path":"/must/not/cross/the/boundary"
                  }
                ]
            }"#,
        )
        .expect("valid inventory");

        assert_eq!(inventory.active_profile, "default");
        assert_eq!(inventory.profiles.len(), 2);
        let profile = &inventory.profiles[1];
        assert_eq!(profile.name, "jake");
        assert_eq!(profile.display_name, "Jake");
        assert_eq!(profile.model.as_deref(), Some("x-ai/grok-4.6"));
        let serialized = serde_json::to_string(profile).expect("serializes");
        assert!(!serialized.contains("/must/not"));
        assert!(!serialized.contains("path"));
    }

    #[test]
    fn rejects_unknown_schema_and_invalid_profile_names() {
        assert!(parse_profile_inventory(
            r#"{"schema":"future/v9","active_profile":"default","profiles":[]}"#
        )
        .is_err());
        assert!(parse_profile_inventory(
            r#"{"schema":"hermes-profile-list/v1","active_profile":"default","profiles":[{"name":"../escape","display_name":"","description":"","description_auto":false,"is_default":false,"active":false,"model":null,"provider":null,"gateway_running":false,"alias":null,"distribution":null}]}"#
        )
        .is_err());
    }

    #[test]
    fn rejects_unbounded_inventory_and_oversized_text() {
        let rows = (0..257)
            .map(|index| {
                serde_json::json!({
                    "name": format!("agent-{index}"),
                    "display_name": "",
                    "description": "",
                    "description_auto": false,
                    "is_default": false,
                    "active": false,
                    "model": null,
                    "provider": null,
                    "gateway_running": false,
                    "alias": null,
                    "distribution": null
                })
            })
            .collect::<Vec<_>>();
        let payload = serde_json::json!({
            "schema": "hermes-profile-list/v1",
            "active_profile": "default",
            "profiles": rows
        });
        assert!(parse_profile_inventory(&payload.to_string()).is_err());

        let oversized = serde_json::json!({
            "schema": "hermes-profile-list/v1",
            "active_profile": "default",
            "profiles": [{
                "name": "default",
                "display_name": "",
                "description": "",
                "description_auto": false,
                "is_default": true,
                "active": true,
                "model": null,
                "provider": null,
                "gateway_running": false,
                "alias": null,
                "distribution": null
            }, {
                "name": "jake",
                "display_name": "x".repeat(129),
                "description": "",
                "description_auto": false,
                "is_default": false,
                "active": false,
                "model": null,
                "provider": null,
                "gateway_running": false,
                "alias": null,
                "distribution": null
            }]
        });
        assert!(parse_profile_inventory(&oversized.to_string()).is_err());
    }
}
