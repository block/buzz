//! Temporary (ephemeral) managed-agent spawn/destroy Tauri commands.
//!
//! Kept out of `agents.rs` for the desktop file-size ratchet.

use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    commands::agents::{create_managed_agent, delete_managed_agent, workspace_owner_hex},
    managed_agents::{
        load_managed_agents, load_personas, BackendKind, CreateManagedAgentRequest,
        CreateManagedAgentResponse, DestroyTempManagedAgentRequest, SpawnTempManagedAgentRequest,
    },
};

/// Spawn a temporary task agent for an orchestrator (no owner form).
/// Inherits parent effective harness/model/provider/env; sets respondTo=allowlist [owner, parent].
#[tauri::command]
pub async fn spawn_temp_managed_agent(
    input: SpawnTempManagedAgentRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CreateManagedAgentResponse, String> {
    let name = input.name.trim().to_string();
    let system_prompt = input.system_prompt.trim().to_string();
    if system_prompt.is_empty() {
        return Err("system prompt is required".to_string());
    }
    let channel_id = input.channel_id.trim().to_string();
    uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let parent_pubkey = input.parent_agent_pubkey.trim().to_ascii_lowercase();
    if parent_pubkey.len() != 64 || !parent_pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("parent_agent_pubkey must be 64-char hex".to_string());
    }

    let owner_hex = workspace_owner_hex(&state)?;
    let ttl = crate::managed_agents::ephemeral::clamp_temp_ttl_secs(input.ttl_seconds);
    let expires_at = crate::managed_agents::ephemeral::expires_at_from_ttl_secs(ttl);

    // Snapshot parent *effective* harness/config under the store lock, then create.
    // Raw record fields are wrong when the parent inherits persona runtime/credentials.
    let (model, provider, agent_command, agent_args, env_vars) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = load_managed_agents(&app)?;
        let global = crate::managed_agents::load_global_agent_config(&app).unwrap_or_default();
        crate::managed_agents::ephemeral::assert_can_spawn_temp(
            &global,
            &records,
            &parent_pubkey,
            &name,
        )?;
        let parent =
            crate::managed_agents::ephemeral::find_parent(&records, &parent_pubkey)?.clone();
        let personas = load_personas(&app).unwrap_or_default();
        let effective = crate::managed_agents::effective_config::resolve_effective_config(
            &parent, &personas, &global,
        );
        let (model, provider) = match effective {
            crate::managed_agents::effective_config::EffectiveConfigResult::Resolved(cfg) => {
                (cfg.model.value, cfg.provider.value)
            }
            crate::managed_agents::effective_config::EffectiveConfigResult::OrphanedInstance {
                ..
            } => {
                return Err(
                    "parent agent is orphaned (missing persona) and cannot spawn temps".to_string(),
                );
            }
        };
        let descriptor = crate::managed_agents::resolve_effective_harness_descriptor(
            &parent, &personas, &global,
        )?;
        (
            model,
            provider,
            descriptor.command,
            descriptor.args,
            // Full layered env (persona credentials included) so the definition-less
            // temp still boots. Agent layer wins on re-layer at spawn.
            descriptor.env,
        )
    };

    let create_input = CreateManagedAgentRequest {
        name,
        persona_id: None,
        team_id: None,
        relay_url: None,
        acp_command: None,
        agent_command: Some(agent_command),
        harness_override: true,
        agent_args,
        mcp_command: None,
        turn_timeout_seconds: None,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: None,
        system_prompt: Some(system_prompt),
        avatar_url: None,
        model,
        provider,
        env_vars,
        spawn_after_create: true,
        start_on_app_launch: false,
        backend: BackendKind::Local,
        respond_to: Some(crate::managed_agents::RespondTo::Allowlist),
        respond_to_allowlist: vec![owner_hex, parent_pubkey.clone()],
        relay_mesh: None,
        ephemeral: true,
        parent_agent_pubkey: Some(parent_pubkey),
        expires_at: Some(expires_at),
        channel_id: Some(channel_id),
        temp_spawn_canonical: true,
    };
    create_managed_agent(create_input, app, state).await
}

/// Destroy a temp agent when the parent requests it (reuses delete path).
#[tauri::command]
pub async fn destroy_temp_managed_agent(
    input: DestroyTempManagedAgentRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let parent = input.parent_agent_pubkey.trim().to_ascii_lowercase();
    let name_or_pk = input.agent_name.trim();
    if name_or_pk.is_empty() {
        return Err("agent name is required".to_string());
    }
    let target_pubkey = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = load_managed_agents(&app)?;
        let target = records
            .iter()
            .find(|r| {
                r.name.eq_ignore_ascii_case(name_or_pk) || r.pubkey.eq_ignore_ascii_case(name_or_pk)
            })
            .ok_or_else(|| format!("temp agent '{name_or_pk}' not found"))?;
        crate::managed_agents::ephemeral::assert_parent_can_destroy(
            &records,
            &target.pubkey,
            &parent,
        )?;
        target.pubkey.clone()
    };
    delete_managed_agent(target_pubkey, None, app).await
}

/// Owner kill-switch: archive every live temp agent on this Desktop.
#[tauri::command]
pub async fn kill_all_temp_agents(app: AppHandle) -> Result<usize, String> {
    sweep_temp_agents_inner(app, true).await
}

/// Sweep expired temp agents (Desktop expiry path; call on boot + timer).
#[tauri::command]
pub async fn sweep_expired_temp_agents(app: AppHandle) -> Result<usize, String> {
    sweep_temp_agents_inner(app, false).await
}

async fn sweep_temp_agents_inner(app: AppHandle, kill_all: bool) -> Result<usize, String> {
    use tauri::Manager;
    let pubkeys = {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = load_managed_agents(&app)?;
        crate::managed_agents::ephemeral::temps_to_sweep(&records, kill_all, chrono::Utc::now())
    };
    let mut deleted = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for pubkey in pubkeys {
        match delete_managed_agent(pubkey.clone(), None, app.clone()).await {
            Ok(()) => deleted += 1,
            Err(e) => errors.push(format!("{pubkey}: {e}")),
        }
    }
    if !errors.is_empty() {
        return Err(format!(
            "deleted {deleted} temp agent(s); {} failed: {}",
            errors.len(),
            errors.join("; ")
        ));
    }
    Ok(deleted)
}

/// Periodic grant-off kill-all / expiry sweep (started from Desktop setup).
pub fn spawn_temp_agent_lifecycle_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        use std::time::Duration;
        loop {
            let grant_on = crate::managed_agents::load_global_agent_config(&app)
                .unwrap_or_default()
                .allow_temp_agent_spawn;
            let result = if grant_on {
                sweep_expired_temp_agents(app.clone()).await
            } else {
                kill_all_temp_agents(app.clone()).await
            };
            if let Err(e) = result {
                eprintln!("buzz-desktop: temp-agent sweep: {e}");
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}
