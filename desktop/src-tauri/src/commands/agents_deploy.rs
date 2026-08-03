//! Provider deploy payload construction, split from `agents.rs` (file-size
//! guard). `build_deploy_payload` gathers live state; `deploy_payload_json`
//! is the pure serialization half so payload completeness stays testable.

use tauri::AppHandle;

use crate::managed_agents::AgentDefinition;
use crate::{
    app_state::AppState,
    managed_agents::{
        load_personas, EffectiveHarnessDescriptor, GlobalAgentConfig, ManagedAgentRecord,
        ReplyPlacement,
    },
    relay::relay_ws_url_with_override,
};

/// Resolve the deploy-specific structured model/provider for a managed agent.
///
/// Delegates to the single effective-config resolver which enforces
/// definition-authoritative semantics for linked instances:
///   - **Linked:** definition → global. Stale record bytes are never consulted.
///   - **Definition-less:** instance → global.
///   - **Orphaned:** returns `(None, None)` — spawn is blocked elsewhere.
///
/// Both local spawn and deploy now use the same resolver, so they can never
/// disagree on what model/provider an agent runs with.
///
/// Exported `pub(crate)` for unit testing.
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

/// Resolve the global config and effective reply placement used by a provider
/// deploy. Keeping the disk-load result in this helper makes the deploy
/// boundary fail closed on malformed persisted config instead of silently
/// reverting to the historical `thread` mode.
pub(crate) fn resolve_deploy_config(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    global_config: Result<GlobalAgentConfig, String>,
) -> Result<(GlobalAgentConfig, ReplyPlacement), String> {
    let global_config = global_config?;
    let reply_placement = crate::managed_agents::resolve_effective_reply_placement(
        record,
        personas,
        global_config.reply_placement,
    )?;
    Ok((global_config, reply_placement))
}

/// Build the standard agent JSON payload for provider deploy calls.
///
/// Like local spawn, provider deploy re-reads the live persona and global
/// configuration, then carries the complete effective harness descriptor so
/// the provider does not have to duplicate desktop-side command, argument, or
/// environment resolution. The legacy top-level fields remain for protocol
/// compatibility; `launch` is the authoritative execution contract.
///
/// Fails closed when the private key is unavailable (keyring outage leaves
/// it empty after hydration): without this guard a provider deploy would
/// serialize `"private_key_nsec": ""` and launch the agent with no
/// identity — the same hazard the local spawn path refuses via
/// `spawn_key_refusal`.
pub(super) fn build_deploy_payload(
    app: &AppHandle,
    state: &AppState,
    record: &ManagedAgentRecord,
) -> Result<serde_json::Value, String> {
    // Fails closed when the private key is unavailable — same guard as local
    // spawn. Without this, a keyring outage would serialize `"private_key_nsec": ""`
    // and launch the agent with no identity.
    if let Some(err) = crate::managed_agents::spawn_key_refusal(record) {
        return Err(err);
    }

    let personas = load_personas(app).unwrap_or_default();
    let (global_config, reply_placement) = resolve_deploy_config(
        record,
        &personas,
        crate::managed_agents::load_global_agent_config(app),
    )?;

    // Merge global + persona + agent env_vars for provider deploy — the same
    // live-persona-under-overrides semantics as local spawn. Global env vars
    // are the lowest user-settable layer: global < persona < agent (last-wins
    // on key collision). Without this, provider-backed agents wouldn't receive
    // credentials saved on the persona or the agent itself.
    let global_env = global_config.env_vars.clone();
    let persona_env =
        crate::managed_agents::resolve_persona_env(app, record.persona_id.as_deref())?;
    // Merge: global < persona (persona wins over global).
    let global_persona_merged = crate::managed_agents::merged_user_env(&global_env, &persona_env);
    // Merge: global+persona < agent (agent wins over everything).
    let merged_env =
        crate::managed_agents::merged_user_env(&global_persona_merged, &record.env_vars);

    let cfg = crate::managed_agents::effective_config::resolve_effective_config(
        record,
        &personas,
        &global_config,
    )
    .require_resolved()?;
    let effective_model = cfg.model.value;
    let effective_provider = cfg.provider.value;
    let effective_prompt = cfg.system_prompt.value;
    let launch = crate::managed_agents::resolve_effective_harness_descriptor(
        record,
        &personas,
        &global_config,
    )?;
    let teams = crate::managed_agents::load_teams(app)?;
    let policy_env = crate::managed_agents::resolve_effective_launch_policy_env(
        record,
        &launch.command,
        &teams,
        effective_prompt.as_deref(),
        effective_model.as_deref(),
        reply_placement,
        true,
    );
    let owner_pubkey = Some(super::workspace_owner_hex(state)?);

    Ok(deploy_payload_json(
        record,
        crate::relay::effective_agent_relay_url(
            &record.relay_url,
            &relay_ws_url_with_override(state),
        ),
        effective_model,
        effective_provider,
        effective_prompt,
        reply_placement,
        merged_env,
        launch,
        policy_env,
        owner_pubkey,
    ))
}

/// Pure serialization half of [`build_deploy_payload`] — every field the
/// provider harness receives is deliberately listed here, so payload
/// completeness is testable without an `AppHandle`.
pub(super) fn deploy_payload_json(
    record: &ManagedAgentRecord,
    relay_url: String,
    effective_model: Option<String>,
    effective_provider: Option<String>,
    effective_prompt: Option<String>,
    reply_placement: ReplyPlacement,
    merged_env: std::collections::BTreeMap<String, String>,
    launch: EffectiveHarnessDescriptor,
    policy_env: std::collections::BTreeMap<String, String>,
    owner_pubkey: Option<String>,
) -> serde_json::Value {
    // The shared descriptor resolver already strips reserved keys while
    // layering user env, but enforce the same invariant at the provider wire
    // boundary so hand-built/legacy descriptors cannot smuggle a policy gate
    // into launch.env. Reply placement is transported in policy_env and is
    // emitted below as the single authoritative value.
    let mut launch_env = launch.env;
    launch_env.retain(|key, _| !crate::managed_agents::is_reserved_env_key(key));
    let mut launch_policy_env = policy_env;
    // This is the only reserved policy key. Enforce it again at the wire
    // boundary so a hand-built/legacy policy map cannot make the provider run
    // a mode different from the typed effective value.
    launch_policy_env.insert(
        "BUZZ_ACP_REPLY_PLACEMENT".to_string(),
        reply_placement.as_str().to_string(),
    );

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
        "reply_placement": reply_placement.as_str(),
        "turn_timeout_seconds": record.turn_timeout_seconds,
        "idle_timeout_seconds": record.idle_timeout_seconds,
        "max_turn_duration_seconds": record.max_turn_duration_seconds,
        "parallelism": record.parallelism,
        "respond_to": record.respond_to,
        "respond_to_allowlist": &record.respond_to_allowlist,
        "env_vars": &merged_env,
        // Provider launchers apply this desktop-resolved policy env to the
        // remote harness. It is separate from user env so a persisted
        // BUZZ_ACP_REPLY_PLACEMENT value cannot override the effective mode.
        "launch": {
            "command": launch.command,
            "args": launch.args,
            "env": launch_env,
            "policy_env": launch_policy_env,
            "owner_pubkey": owner_pubkey,
        },
    })
}
