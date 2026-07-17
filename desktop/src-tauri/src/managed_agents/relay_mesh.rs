#[cfg(feature = "mesh-llm")]
use super::{ManagedAgentRecord, RelayMeshConfig};

pub const RELAY_MESH_API_BASE_URL: &str = "http://127.0.0.1:9337/v1";
pub const RELAY_MESH_API_KEY_PLACEHOLDER: &str = "buzz-mesh-local";
pub const RELAY_MESH_PROVIDER_ID: &str = "relay-mesh";
pub const RELAY_MESH_AUTO_MODEL_ID: &str = "auto";

/// Translate the native Buzz shared compute provider into the OpenAI-compatible
/// transport understood by Buzz's agent runtimes. These are derived runtime
/// details, not user-owned agent configuration. Goose's native variable names
/// are added later by `apply_goose_runtime_env` when Goose is the runtime.
#[cfg(feature = "mesh-llm")]
pub fn apply_relay_mesh_env(
    env: &mut std::collections::BTreeMap<String, String>,
    provider: Option<&str>,
    model: Option<&str>,
) {
    if provider.map(str::trim) != Some(RELAY_MESH_PROVIDER_ID) {
        return;
    }
    let model = model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(RELAY_MESH_AUTO_MODEL_ID)
        .to_string();
    env.insert("BUZZ_AGENT_PROVIDER".to_string(), "openai".to_string());
    env.insert("BUZZ_AGENT_MODEL".to_string(), model.clone());
    env.insert(
        "OPENAI_COMPAT_BASE_URL".to_string(),
        RELAY_MESH_API_BASE_URL.to_string(),
    );
    env.insert("OPENAI_COMPAT_MODEL".to_string(), model);
    env.insert(
        "OPENAI_COMPAT_API_KEY".to_string(),
        RELAY_MESH_API_KEY_PLACEHOLDER.to_string(),
    );
    env.insert("OPENAI_COMPAT_API".to_string(), "chat".to_string());
    // Keep the requested response inside smaller local-model context windows,
    // and spend that budget on an answer/tool call instead of hidden reasoning.
    // Without both settings Qwen3 either fails the router's fit check at the
    // agent default (32K) or can consume a tight cap before serializing a tool.
    env.insert(
        "BUZZ_AGENT_MAX_OUTPUT_TOKENS".to_string(),
        "4096".to_string(),
    );
    env.insert("BUZZ_AGENT_THINKING_EFFORT".to_string(), "none".to_string());
}

/// Resolve a record's relay-mesh config, typed field first.
///
/// Source of truth is the typed `record.relay_mesh` field. For records saved
/// before that field existed, fall back to detecting the relay-mesh preset
/// from `env_vars` (the legacy discriminator). New records carry the typed
/// field and need no env-var sniffing at all.
#[cfg(feature = "mesh-llm")]
pub fn relay_mesh_config(record: &ManagedAgentRecord) -> Option<RelayMeshConfig> {
    if record.provider.as_deref() == Some(RELAY_MESH_PROVIDER_ID) {
        let model_ref = record
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(RELAY_MESH_AUTO_MODEL_ID)
            .to_string();
        return Some(RelayMeshConfig { model_ref });
    }
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

#[cfg(all(test, feature = "mesh-llm"))]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::managed_agents::{BackendKind, ManagedAgentRecord, RespondTo};

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
            env_vars: BTreeMap::new(),
            start_on_app_launch: false,
            auto_restart_on_config_change: true,
            runtime_pid: None,
            backend: BackendKind::Local,
            backend_agent_id: None,
            provider_binary_path: None,
            team_id: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            persona_source_version: None,
            provider: None,
            created_at: "now".into(),
            updated_at: "now".into(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: RespondTo::OwnerOnly,
            respond_to_allowlist: vec![],
            display_name: None,
            slug: None,
            runtime: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            definition_respond_to: None,
            definition_respond_to_allowlist: Vec::new(),
            definition_parallelism: None,
            relay_mesh: None,
        }
    }

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

    #[test]
    fn native_provider_uses_context_safe_non_reasoning_budget() {
        let mut env = BTreeMap::new();
        apply_relay_mesh_env(
            &mut env,
            Some(RELAY_MESH_PROVIDER_ID),
            Some(RELAY_MESH_AUTO_MODEL_ID),
        );

        assert_eq!(
            env.get("BUZZ_AGENT_MAX_OUTPUT_TOKENS").map(String::as_str),
            Some("4096")
        );
        assert_eq!(
            env.get("BUZZ_AGENT_THINKING_EFFORT").map(String::as_str),
            Some("none")
        );
    }

    #[test]
    fn bundled_goose_receives_the_native_shared_compute_transport() {
        let mut rec = fixture();
        rec.runtime = Some("goose".to_string());
        rec.provider = Some(RELAY_MESH_PROVIDER_ID.to_string());
        rec.model = Some("Qwen3".to_string());
        let runtime = crate::managed_agents::known_acp_runtime_exact("goose");

        let effective = crate::managed_agents::resolve_effective_agent_env(
            &rec,
            &[],
            runtime,
            &Default::default(),
        );

        assert_eq!(effective.effective_command, "goose-acp");
        assert_eq!(
            effective.env.get("GOOSE_PROVIDER").map(String::as_str),
            Some("openai")
        );
        assert_eq!(
            effective.env.get("GOOSE_MODEL").map(String::as_str),
            Some("Qwen3")
        );
        assert_eq!(
            effective.env.get("OPENAI_BASE_URL").map(String::as_str),
            Some(RELAY_MESH_API_BASE_URL)
        );
        assert_eq!(
            effective.env.get("OPENAI_API_KEY").map(String::as_str),
            Some(RELAY_MESH_API_KEY_PLACEHOLDER)
        );
        assert!(crate::managed_agents::agent_readiness(&effective).is_ready());
    }

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

    #[test]
    fn native_provider_fields_are_authoritative() {
        // The whole point: a typed record needs no env-var sniffing to be
        // recognized as a relay-mesh agent.
        let mut rec = fixture();
        rec.provider = Some(RELAY_MESH_PROVIDER_ID.to_string());
        rec.model = Some("Qwen3".to_string());
        assert!(rec.env_vars.is_empty());
        assert_eq!(
            relay_mesh_config(&rec),
            Some(RelayMeshConfig {
                model_ref: "Qwen3".to_string()
            })
        );
        assert_eq!(relay_mesh_model_id(&rec).as_deref(), Some("Qwen3"));
    }

    #[test]
    fn native_provider_fields_win_over_legacy_config() {
        let mut rec = fixture();
        rec.provider = Some(RELAY_MESH_PROVIDER_ID.to_string());
        rec.model = Some("native-model".to_string());
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
        assert_eq!(relay_mesh_model_id(&rec).as_deref(), Some("native-model"));
    }

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
