//! Provider deploy payload construction, split from `agents.rs` (file-size
//! guard). The launch block is derived from the same effective descriptor and
//! policy helpers as local spawn so remote execution does not reimplement them.

use std::collections::BTreeMap;

use tauri::AppHandle;

#[cfg(test)]
use crate::managed_agents::AgentDefinition;
use crate::{
    app_state::AppState,
    managed_agents::{load_personas, ManagedAgentRecord},
    relay::relay_ws_url_with_override,
};

/// Resolve the deploy-specific structured model/provider for a managed agent.
#[cfg(test)]
pub(crate) fn resolve_deploy_model_provider(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    global: &crate::managed_agents::GlobalAgentConfig,
) -> (Option<String>, Option<String>) {
    crate::managed_agents::effective_config::resolve_effective_model_provider_pair(
        record, personas, global,
    )
    .unwrap_or((None, None))
}

/// Serialize the portable launch contract shared with provider-backed agents.
///
/// `descriptor.env` is the authoritative six-layer environment. Policy values
/// are deliberately separate because providers apply them below that layered
/// environment, preserving the local spawn's power-user override semantics.
pub(super) fn build_launch_block(
    record: &ManagedAgentRecord,
    descriptor: &crate::managed_agents::readiness::EffectiveHarnessDescriptor,
    teams: &[crate::managed_agents::TeamRecord],
    effective_prompt: Option<&str>,
    effective_model: Option<&str>,
    owner_pubkey: &str,
) -> serde_json::Value {
    use crate::managed_agents::{known_acp_runtime, resolve_session_title, SESSION_TITLE_ENV_VAR};

    let runtime = known_acp_runtime(&descriptor.command);
    let mut policy_env = BTreeMap::new();

    if let Some(runtime) = runtime {
        policy_env.extend(
            runtime
                .default_env
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string())),
        );
        if runtime.mcp_hooks {
            policy_env.insert("MCP_HOOK_SERVERS".into(), "*".into());
        }
    }
    policy_env.insert("BUZZ_ACP_RELAY_OBSERVER".into(), "true".into());
    policy_env.insert("BUZZ_ACP_LAZY_POOL".into(), "true".into());
    policy_env.insert("BUZZ_ACP_AGENTS".into(), record.parallelism.to_string());

    if let Some(value) = effective_prompt {
        policy_env.insert("BUZZ_ACP_SYSTEM_PROMPT".into(), value.to_string());
    }
    if let Some(value) = effective_model {
        policy_env.insert("BUZZ_ACP_MODEL".into(), value.to_string());
    }
    if let Some(value) = record.idle_timeout_seconds {
        policy_env.insert("BUZZ_ACP_IDLE_TIMEOUT".into(), value.to_string());
    }
    if let Some(value) = record.max_turn_duration_seconds {
        policy_env.insert("BUZZ_ACP_MAX_TURN_DURATION".into(), value.to_string());
    }
    if let Some(value) = resolve_session_title(record.display_name.as_deref(), &record.name) {
        policy_env.insert(SESSION_TITLE_ENV_VAR.into(), value);
    }
    if let Some(value) =
        crate::managed_agents::spawn_hash::effective_team_instructions(record, teams)
    {
        policy_env.insert("BUZZ_ACP_TEAM_INSTRUCTIONS".into(), value);
    }

    serde_json::json!({
        "command": descriptor.command,
        "args": descriptor.args,
        "env": descriptor.env,
        "policy_env": policy_env,
        "owner_pubkey": owner_pubkey,
    })
}

pub(super) fn ensure_remote_provider_supported(provider: Option<&str>) -> Result<(), String> {
    if provider.map(str::trim) == Some(crate::managed_agents::RELAY_MESH_PROVIDER_ID) {
        return Err(
            "shared-compute agents cannot be deployed remotely because the mesh endpoint is local to the desktop"
                .to_string(),
        );
    }
    Ok(())
}

/// Build the standard agent JSON payload for provider deploy calls.
pub(super) fn build_deploy_payload(
    app: &AppHandle,
    state: &AppState,
    record: &ManagedAgentRecord,
) -> Result<serde_json::Value, String> {
    if let Some(err) = crate::managed_agents::spawn_key_refusal(record) {
        return Err(err);
    }

    let global = crate::managed_agents::load_global_agent_config(app).unwrap_or_default();
    let personas = load_personas(app).unwrap_or_default();
    let teams = crate::managed_agents::load_teams(app).unwrap_or_default();
    let persona_env =
        crate::managed_agents::live_persona_env(&personas, record.persona_id.as_deref());
    let global_persona_env = crate::managed_agents::merged_user_env(&global.env_vars, &persona_env);
    let merged_user_env =
        crate::managed_agents::merged_user_env(&global_persona_env, &record.env_vars);
    let effective = crate::managed_agents::effective_config::resolve_effective_config(
        record, &personas, &global,
    )
    .require_resolved()?;

    ensure_remote_provider_supported(effective.provider.value.as_deref())?;

    let descriptor =
        crate::managed_agents::resolve_effective_harness_descriptor(record, &personas, &global)
            .map_err(|error| crate::managed_agents::user_facing_harness_error(&error))?;
    let owner_pubkey = super::workspace_owner_hex(state)?;
    let launch = build_launch_block(
        record,
        &descriptor,
        &teams,
        effective.system_prompt.value.as_deref(),
        effective.model.value.as_deref(),
        &owner_pubkey,
    );

    Ok(deploy_payload_json(
        record,
        crate::relay::effective_agent_relay_url(
            &record.relay_url,
            &relay_ws_url_with_override(state),
        ),
        effective.model.value,
        effective.provider.value,
        effective.system_prompt.value,
        merged_user_env,
        launch,
    ))
}

/// Pure serialization half of [`build_deploy_payload`]. Legacy top-level fields
/// remain for display/bookkeeping; providers execute the resolved `launch` block.
pub(super) fn deploy_payload_json(
    record: &ManagedAgentRecord,
    relay_url: String,
    effective_model: Option<String>,
    effective_provider: Option<String>,
    effective_prompt: Option<String>,
    merged_env: BTreeMap<String, String>,
    launch: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "name": &record.name,
        "relay_url": relay_url,
        "private_key_nsec": &record.private_key_nsec,
        "auth_tag": &record.auth_tag,
        "agent_command": &record.agent_command,
        "agent_args": &record.agent_args,
        "system_prompt": effective_prompt,
        "model": effective_model,
        "provider": effective_provider,
        "turn_timeout_seconds": record.turn_timeout_seconds,
        "idle_timeout_seconds": record.idle_timeout_seconds,
        "max_turn_duration_seconds": record.max_turn_duration_seconds,
        "parallelism": record.parallelism,
        "respond_to": record.respond_to,
        "respond_to_allowlist": &record.respond_to_allowlist,
        "env_vars": merged_env,
        "launch": launch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::{readiness::EffectiveHarnessDescriptor, RespondTo, TeamRecord};

    fn record() -> ManagedAgentRecord {
        serde_json::from_value(serde_json::json!({
            "pubkey": "abcd1234",
            "name": "agent-handle",
            "display_name": "Agent\u{0000} Name",
            "private_key_nsec": "nsec1fake",
            "relay_url": "wss://relay.example",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "idle_timeout_seconds": 17,
            "max_turn_duration_seconds": 23,
            "parallelism": 4,
            "respond_to": RespondTo::OwnerOnly,
            "respond_to_allowlist": [],
            "team_id": "team-1",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    #[test]
    fn launch_block_preserves_descriptor_and_spawn_policy() {
        let record = record();
        let descriptor = EffectiveHarnessDescriptor {
            command: "goose".into(),
            args: vec!["acp".into()],
            env: BTreeMap::from([
                ("GOOSE_MODE".into(), "custom".into()),
                ("SECRET_FROM_PERSONA".into(), "secret".into()),
            ]),
        };
        let teams: Vec<TeamRecord> = serde_json::from_value(serde_json::json!([{
            "id": "team-1", "name": "Team", "instructions": "Coordinate", "persona_ids": [], "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        }])).unwrap();

        let launch = build_launch_block(
            &record,
            &descriptor,
            &teams,
            Some("prompt"),
            Some("model"),
            "owner-hex",
        );

        assert_eq!(launch["command"], "goose");
        assert_eq!(launch["args"], serde_json::json!(["acp"]));
        assert_eq!(launch["env"]["GOOSE_MODE"], "custom");
        // policy_env is applied first, so this default remains separate from
        // the descriptor value that wins in launch.env.
        assert_eq!(launch["policy_env"]["GOOSE_MODE"], "auto");
        assert_eq!(launch["policy_env"]["BUZZ_ACP_LAZY_POOL"], "true");
        assert_eq!(launch["policy_env"]["BUZZ_ACP_RELAY_OBSERVER"], "true");
        assert_eq!(
            launch["policy_env"]["BUZZ_ACP_TEAM_INSTRUCTIONS"],
            "Coordinate"
        );
        assert_eq!(launch["policy_env"]["BUZZ_ACP_SESSION_TITLE"], "Agent Name");
        assert_eq!(launch["policy_env"]["BUZZ_ACP_SYSTEM_PROMPT"], "prompt");
        assert_eq!(launch["policy_env"]["BUZZ_ACP_MODEL"], "model");
        assert_eq!(launch["policy_env"]["BUZZ_ACP_IDLE_TIMEOUT"], "17");
        assert_eq!(launch["policy_env"]["BUZZ_ACP_MAX_TURN_DURATION"], "23");
        assert_eq!(launch["policy_env"]["BUZZ_ACP_AGENTS"], "4");
        assert_eq!(launch["owner_pubkey"], "owner-hex");
    }
}
