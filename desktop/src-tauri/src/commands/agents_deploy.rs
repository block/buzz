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

/// Effective projection fields for the deploy payload — all derived from the
/// resolved descriptor and effective config so that the serialised payload and
/// the `launch` block are always internally consistent.
pub(super) struct DeployProjections {
    pub effective_model: Option<String>,
    pub effective_provider: Option<String>,
    pub effective_prompt: Option<String>,
    /// Effective parallelism derived from the same resolved `descriptor.command`
    /// as `launch.policy_env["BUZZ_ACP_AGENTS"]`.
    pub effective_parallelism: u32,
    /// Access fields projected from the same build policy that gates local starts.
    pub owner_only_access: bool,
}

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
    use crate::managed_agents::{
        known_acp_runtime, resolve_session_title, DISPLAY_NAME_ENV_VAR, SESSION_TITLE_ENV_VAR,
    };

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
    policy_env.insert(
        "BUZZ_ACP_AGENTS".into(),
        crate::managed_agents::acp_agents_value(&descriptor.command, record.parallelism),
    );

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
        policy_env.insert(SESSION_TITLE_ENV_VAR.into(), value.clone());
        policy_env.insert(DISPLAY_NAME_ENV_VAR.into(), value);
    }
    if let Some(value) =
        crate::managed_agents::spawn_snapshot::effective_team_instructions(record, teams)
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

    let effective_parallelism =
        crate::managed_agents::effective_parallelism(&descriptor.command, record.parallelism);

    Ok(deploy_payload_json(
        record,
        crate::relay::effective_agent_relay_url(
            &record.relay_url,
            &relay_ws_url_with_override(state),
        ),
        DeployProjections {
            effective_model: effective.model.value,
            effective_provider: effective.provider.value,
            effective_prompt: effective.system_prompt.value,
            effective_parallelism,
            owner_only_access: crate::managed_agents::owner_only_access_build(),
        },
        merged_user_env,
        launch,
    ))
}

/// Pure serialization half of [`build_deploy_payload`]. Legacy top-level fields
/// remain for display/bookkeeping; providers execute the resolved `launch` block.
/// `projections.effective_parallelism` is pre-computed from the same resolved
/// descriptor as `launch.policy_env["BUZZ_ACP_AGENTS"]`. Access is projected from
/// the same compiled policy that gates local starts.
pub(super) fn deploy_payload_json(
    record: &ManagedAgentRecord,
    relay_url: String,
    projections: DeployProjections,
    merged_env: BTreeMap<String, String>,
    launch: serde_json::Value,
) -> serde_json::Value {
    let (respond_to, respond_to_allowlist) =
        crate::managed_agents::projected_access_with_policy(record, projections.owner_only_access);
    serde_json::json!({
        "name": &record.name,
        "relay_url": relay_url,
        "private_key_nsec": &record.private_key_nsec,
        "auth_tag": &record.auth_tag,
        "agent_command": &record.agent_command,
        "agent_args": &record.agent_args,
        "system_prompt": projections.effective_prompt,
        "model": projections.effective_model,
        "provider": projections.effective_provider,
        "turn_timeout_seconds": record.turn_timeout_seconds,
        "idle_timeout_seconds": record.idle_timeout_seconds,
        "max_turn_duration_seconds": record.max_turn_duration_seconds,
        // Legacy top-level field: projected from the same resolved descriptor as
        // launch.policy_env["BUZZ_ACP_AGENTS"] — the two are always consistent.
        "parallelism": projections.effective_parallelism,
        "respond_to": respond_to,
        "respond_to_allowlist": respond_to_allowlist,
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
        assert_eq!(launch["policy_env"]["BUZZ_ACP_DISPLAY_NAME"], "Agent Name");
        assert_eq!(launch["policy_env"]["BUZZ_ACP_SYSTEM_PROMPT"], "prompt");
        assert_eq!(launch["policy_env"]["BUZZ_ACP_MODEL"], "model");
        assert_eq!(launch["policy_env"]["BUZZ_ACP_IDLE_TIMEOUT"], "17");
        assert_eq!(launch["policy_env"]["BUZZ_ACP_MAX_TURN_DURATION"], "23");
        assert_eq!(launch["policy_env"]["BUZZ_ACP_AGENTS"], "4");
        assert_eq!(launch["owner_pubkey"], "owner-hex");
    }

    /// OpenClaw descriptor: `launch.policy_env["BUZZ_ACP_AGENTS"]` must be "5"
    /// even when the record's requested parallelism is 10. This is the direct
    /// `launch.policy_env` seam test — the executable contract for remote providers.
    #[test]
    fn launch_block_openclaw_over_cap_policy_env_is_capped() {
        let mut record = record();
        record.agent_command = "openclaw".into();
        record.parallelism = 10; // above the OpenClaw spawn-time cap
        let descriptor = EffectiveHarnessDescriptor {
            command: "openclaw".into(),
            args: vec![],
            env: BTreeMap::new(),
        };

        let launch = build_launch_block(&record, &descriptor, &[], None, None, "owner-hex");

        assert_eq!(
            launch["policy_env"]["BUZZ_ACP_AGENTS"],
            crate::managed_agents::parallelism::OPENCLAW_MAX_PARALLELISM.to_string(),
            "launch.policy_env[BUZZ_ACP_AGENTS] must be capped at {} for OpenClaw, not 10",
            crate::managed_agents::parallelism::OPENCLAW_MAX_PARALLELISM
        );
    }

    /// Uncapped harness (goose): `launch.policy_env["BUZZ_ACP_AGENTS"]` passes
    /// the requested value through unchanged.
    #[test]
    fn launch_block_goose_policy_env_is_not_capped() {
        let mut record = record();
        record.parallelism = 8;
        let descriptor = EffectiveHarnessDescriptor {
            command: "goose".into(),
            args: vec![],
            env: BTreeMap::new(),
        };

        let launch = build_launch_block(&record, &descriptor, &[], None, None, "owner-hex");

        assert_eq!(
            launch["policy_env"]["BUZZ_ACP_AGENTS"], "8",
            "goose: policy_env[BUZZ_ACP_AGENTS] must pass through requested value 8"
        );
    }

    /// deploy_payload_json: legacy top-level `parallelism` is the effective value
    /// derived from the descriptor, not `record.agent_command`.
    ///
    /// Stale-persona scenario: `record.agent_command` is "goose" (created before
    /// the user switched the persona to OpenClaw), but the live descriptor resolves
    /// OpenClaw. Both `launch.policy_env["BUZZ_ACP_AGENTS"]` and the legacy
    /// top-level `parallelism` must be the effective OpenClaw value (5), not the
    /// record's stale Goose identity (requested 10).
    #[test]
    fn deploy_payload_json_stale_goose_record_live_openclaw_descriptor_both_capped() {
        let mut record = record();
        // Stale agent_command from record creation — persona has since switched to OpenClaw.
        record.agent_command = "goose".into();
        record.parallelism = 10;
        // Resolved descriptor reflects the live persona (OpenClaw).
        let descriptor = EffectiveHarnessDescriptor {
            command: "openclaw".into(),
            args: vec![],
            env: BTreeMap::new(),
        };
        let cap = crate::managed_agents::parallelism::OPENCLAW_MAX_PARALLELISM;

        let launch = build_launch_block(&record, &descriptor, &[], None, None, "owner-hex");
        let effective_parallelism =
            crate::managed_agents::effective_parallelism(&descriptor.command, record.parallelism);
        let payload = deploy_payload_json(
            &record,
            "wss://relay.example".to_string(),
            DeployProjections {
                effective_model: None,
                effective_provider: None,
                effective_prompt: None,
                effective_parallelism,
                owner_only_access: false,
            },
            BTreeMap::new(),
            launch.clone(),
        );

        assert_eq!(
            launch["policy_env"]["BUZZ_ACP_AGENTS"],
            cap.to_string(),
            "launch.policy_env[BUZZ_ACP_AGENTS] must be capped at {cap} for live OpenClaw descriptor"
        );
        assert_eq!(
            payload["parallelism"], cap,
            "legacy top-level parallelism must match launch.policy_env — both must be {cap}"
        );
    }

    /// Inverse stale-persona scenario: `record.agent_command` is "openclaw"
    /// (created before the user switched the persona to Goose), but the live
    /// descriptor resolves Goose. Both projections must be the uncapped requested
    /// value (4), not the old OpenClaw cap.
    #[test]
    fn deploy_payload_json_stale_openclaw_record_live_goose_descriptor_both_uncapped() {
        let mut record = record();
        // Stale agent_command from record creation — persona has since switched to Goose.
        record.agent_command = "openclaw".into();
        record.parallelism = 4;
        // Resolved descriptor reflects the live persona (Goose).
        let descriptor = EffectiveHarnessDescriptor {
            command: "goose".into(),
            args: vec![],
            env: BTreeMap::new(),
        };

        let launch = build_launch_block(&record, &descriptor, &[], None, None, "owner-hex");
        let effective_parallelism =
            crate::managed_agents::effective_parallelism(&descriptor.command, record.parallelism);
        let payload = deploy_payload_json(
            &record,
            "wss://relay.example".to_string(),
            DeployProjections {
                effective_model: None,
                effective_provider: None,
                effective_prompt: None,
                effective_parallelism,
                owner_only_access: false,
            },
            BTreeMap::new(),
            launch.clone(),
        );

        assert_eq!(
            launch["policy_env"]["BUZZ_ACP_AGENTS"],
            "4",
            "launch.policy_env[BUZZ_ACP_AGENTS] must pass through requested 4 for live Goose descriptor"
        );
        assert_eq!(
            payload["parallelism"], 4,
            "legacy top-level parallelism must match launch.policy_env — both must be 4 (uncapped)"
        );
    }

    /// Explicit agent_command_override direction: record has an explicit override
    /// pinning OpenClaw while the persona default is Goose. The override wins
    /// via the descriptor — both projections must be capped at the OpenClaw limit.
    #[test]
    fn deploy_payload_json_explicit_openclaw_override_both_capped() {
        let mut record = record();
        // Explicit override: user pinned OpenClaw on this agent.
        record.agent_command_override = Some("openclaw".into());
        record.agent_command = "goose".into(); // persona default, overridden
        record.parallelism = 10;
        // Descriptor reflects the resolved override (OpenClaw wins).
        let descriptor = EffectiveHarnessDescriptor {
            command: "openclaw".into(),
            args: vec![],
            env: BTreeMap::new(),
        };
        let cap = crate::managed_agents::parallelism::OPENCLAW_MAX_PARALLELISM;

        let launch = build_launch_block(&record, &descriptor, &[], None, None, "owner-hex");
        let effective_parallelism =
            crate::managed_agents::effective_parallelism(&descriptor.command, record.parallelism);
        let payload = deploy_payload_json(
            &record,
            "wss://relay.example".to_string(),
            DeployProjections {
                effective_model: None,
                effective_provider: None,
                effective_prompt: None,
                effective_parallelism,
                owner_only_access: false,
            },
            BTreeMap::new(),
            launch.clone(),
        );

        assert_eq!(
            launch["policy_env"]["BUZZ_ACP_AGENTS"],
            cap.to_string(),
            "launch.policy_env[BUZZ_ACP_AGENTS] must be {cap} for explicit OpenClaw override"
        );
        assert_eq!(
            payload["parallelism"], cap,
            "legacy top-level parallelism must match launch.policy_env — both must be {cap}"
        );
    }
}
