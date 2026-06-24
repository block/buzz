use super::ManagedAgentRecord;
pub use super::RelayMeshConfig;

pub const RELAY_MESH_API_BASE_URL: &str = "http://127.0.0.1:9337/v1";
pub const RELAY_MESH_API_KEY_PLACEHOLDER: &str = "buzz-mesh-local";

/// Resolve a record's relay-mesh config, typed field first.
///
/// Source of truth is the typed `record.relay_mesh` field. For records saved
/// before that field existed, fall back to detecting the relay-mesh preset
/// from `env_vars` (the legacy discriminator). New records carry the typed
/// field and need no env-var sniffing at all.
#[cfg(feature = "mesh-llm")]
pub fn relay_mesh_config(record: &ManagedAgentRecord) -> Option<RelayMeshConfig> {
    if let Some(config) = &record.relay_mesh {
        return Some(config.clone());
    }
    relay_mesh_model_id_from_env(record).map(|model_ref| RelayMeshConfig { model_ref })
}

/// Returns the relay-mesh model id for agents whose provider env points at the
/// local mesh client endpoint created by Buzz's relay-mesh preset.
///
/// Prefer [`relay_mesh_config`]; this remains as a convenience for call sites
/// that only need the model id.
#[cfg(feature = "mesh-llm")]
pub fn relay_mesh_model_id(record: &ManagedAgentRecord) -> Option<String> {
    relay_mesh_config(record).map(|config| config.model_ref)
}

/// Legacy env-var discriminator: detects the relay-mesh preset purely from the
/// four preset env vars. Used as a fallback for records saved before the typed
/// `relay_mesh` field existed.
#[cfg(feature = "mesh-llm")]
fn relay_mesh_model_id_from_env(record: &ManagedAgentRecord) -> Option<String> {
    let base_url = record.env_vars.get("OPENAI_COMPAT_BASE_URL")?.trim();
    if base_url.trim_end_matches('/') != RELAY_MESH_API_BASE_URL {
        return None;
    }
    let provider = record.env_vars.get("BUZZ_AGENT_PROVIDER")?.trim();
    if provider != "openai" {
        return None;
    }
    let api_key = record.env_vars.get("OPENAI_COMPAT_API_KEY")?.trim();
    if api_key != RELAY_MESH_API_KEY_PLACEHOLDER {
        return None;
    }
    record
        .env_vars
        .get("OPENAI_COMPAT_MODEL")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::managed_agents::{BackendKind, RespondTo};

    fn fixture() -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: "p".into(),
            name: "n".into(),
            persona_id: None,
            private_key_nsec: "nsec1fake".into(),
            auth_tag: Some("tag".into()),
            relay_url: "ws://localhost:3000".into(),
            avatar_url: None,
            acp_command: "buzz-acp".into(),
            agent_command: "goose".into(),
            agent_command_override: None,
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: 320,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 1,
            system_prompt: None,
            model: None,
            mcp_toolsets: None,
            env_vars: BTreeMap::new(),
            start_on_app_launch: false,
            runtime_pid: None,
            backend: BackendKind::Local,
            backend_agent_id: None,
            provider_binary_path: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: "now".into(),
            updated_at: "now".into(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            respond_to: RespondTo::OwnerOnly,
            respond_to_allowlist: vec![],
            relay_mesh: None,
        }
    }

    #[cfg(feature = "mesh-llm")]
    #[test]
    fn relay_mesh_model_id_detects_mesh_preset_env() {
        let mut rec = fixture();
        rec.env_vars = BTreeMap::from([
            ("BUZZ_AGENT_PROVIDER".to_string(), "openai".to_string()),
            (
                "OPENAI_COMPAT_BASE_URL".to_string(),
                "http://127.0.0.1:9337/v1/".to_string(),
            ),
            ("OPENAI_COMPAT_MODEL".to_string(), "Qwen3".to_string()),
            (
                "OPENAI_COMPAT_API_KEY".to_string(),
                RELAY_MESH_API_KEY_PLACEHOLDER.to_string(),
            ),
        ]);

        assert_eq!(relay_mesh_model_id(&rec).as_deref(), Some("Qwen3"));
    }

    #[cfg(feature = "mesh-llm")]
    #[test]
    fn relay_mesh_model_id_ignores_non_mesh_openai_env() {
        let mut rec = fixture();
        rec.env_vars = BTreeMap::from([
            ("BUZZ_AGENT_PROVIDER".to_string(), "openai".to_string()),
            (
                "OPENAI_COMPAT_BASE_URL".to_string(),
                "https://api.openai.com/v1".to_string(),
            ),
            ("OPENAI_COMPAT_MODEL".to_string(), "gpt-5".to_string()),
        ]);

        assert_eq!(relay_mesh_model_id(&rec), None);
    }

    #[cfg(feature = "mesh-llm")]
    #[test]
    fn relay_mesh_model_id_ignores_user_openai_on_same_local_port() {
        let mut rec = fixture();
        rec.env_vars = BTreeMap::from([
            ("BUZZ_AGENT_PROVIDER".to_string(), "openai".to_string()),
            (
                "OPENAI_COMPAT_BASE_URL".to_string(),
                "http://127.0.0.1:9337/v1".to_string(),
            ),
            ("OPENAI_COMPAT_MODEL".to_string(), "Qwen3".to_string()),
            ("OPENAI_COMPAT_API_KEY".to_string(), "real-key".to_string()),
        ]);

        assert_eq!(relay_mesh_model_id(&rec), None);
    }

    #[cfg(feature = "mesh-llm")]
    #[test]
    fn typed_field_recognized_with_zero_env_vars() {
        // The whole point: a typed record needs no env-var sniffing to be
        // recognized as a relay-mesh agent.
        let mut rec = fixture();
        rec.relay_mesh = Some(RelayMeshConfig {
            model_ref: "Qwen3".to_string(),
        });
        assert!(rec.env_vars.is_empty());
        assert_eq!(
            relay_mesh_config(&rec),
            Some(RelayMeshConfig {
                model_ref: "Qwen3".to_string()
            })
        );
        assert_eq!(relay_mesh_model_id(&rec).as_deref(), Some("Qwen3"));
    }

    #[cfg(feature = "mesh-llm")]
    #[test]
    fn typed_field_wins_over_env_sniff() {
        // When both are present, the typed field is authoritative.
        let mut rec = fixture();
        rec.relay_mesh = Some(RelayMeshConfig {
            model_ref: "typed-model".to_string(),
        });
        rec.env_vars = BTreeMap::from([
            ("BUZZ_AGENT_PROVIDER".to_string(), "openai".to_string()),
            (
                "OPENAI_COMPAT_BASE_URL".to_string(),
                "http://127.0.0.1:9337/v1".to_string(),
            ),
            ("OPENAI_COMPAT_MODEL".to_string(), "env-model".to_string()),
            (
                "OPENAI_COMPAT_API_KEY".to_string(),
                RELAY_MESH_API_KEY_PLACEHOLDER.to_string(),
            ),
        ]);
        assert_eq!(relay_mesh_model_id(&rec).as_deref(), Some("typed-model"));
    }

    #[cfg(feature = "mesh-llm")]
    #[test]
    fn legacy_record_falls_back_to_env_sniff() {
        // Records saved before the typed field still resolve via env vars.
        let mut rec = fixture();
        rec.relay_mesh = None;
        rec.env_vars = BTreeMap::from([
            ("BUZZ_AGENT_PROVIDER".to_string(), "openai".to_string()),
            (
                "OPENAI_COMPAT_BASE_URL".to_string(),
                "http://127.0.0.1:9337/v1".to_string(),
            ),
            ("OPENAI_COMPAT_MODEL".to_string(), "Qwen3".to_string()),
            (
                "OPENAI_COMPAT_API_KEY".to_string(),
                RELAY_MESH_API_KEY_PLACEHOLDER.to_string(),
            ),
        ]);
        assert_eq!(relay_mesh_model_id(&rec).as_deref(), Some("Qwen3"));
    }
}
