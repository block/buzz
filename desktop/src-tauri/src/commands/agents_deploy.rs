//! Provider deploy payload construction, split from `agents.rs` (file-size
//! guard). `build_deploy_payload` gathers live state; `deploy_payload_json`
//! is the pure serialization half so payload completeness stays testable.

use tauri::AppHandle;

#[cfg(test)]
use crate::managed_agents::AgentDefinition;
use crate::{
    app_state::AppState,
    managed_agents::{load_personas, ManagedAgentRecord},
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

/// Build the standard agent JSON payload for provider deploy calls.
///
/// Like local spawn, provider deploy re-reads live persona env vars and
/// structured model/provider so remote agents receive current credentials
/// and the same authoritative values that local spawn derives from
/// `runtime_metadata_env_vars`. The only field still pinned is
/// `agent_command`/`agent_args` — those were captured at create time.
/// The only read-time resolution is `relay_url`: a blank pin resolves to
/// the active workspace relay here, matching the create-path contract.
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

    // Merge global + persona + agent env_vars for provider deploy — the same
    // live-persona-under-overrides semantics as local spawn. Global env vars
    // are the lowest user-settable layer: global < persona < agent (last-wins
    // on key collision). Without this, provider-backed agents wouldn't receive
    // credentials saved on the persona or the agent itself.
    let global_config = crate::managed_agents::load_global_agent_config(app).unwrap_or_default();
    let global_env = global_config.env_vars.clone();
    let persona_env =
        crate::managed_agents::resolve_persona_env(app, record.persona_id.as_deref())?;
    // Merge: global < persona (persona wins over global).
    let global_persona_merged = crate::managed_agents::merged_user_env(&global_env, &persona_env);
    // Merge: global+persona < agent (agent wins over everything).
    let merged_env =
        crate::managed_agents::merged_user_env(&global_persona_merged, &record.env_vars);

    let personas = load_personas(app).unwrap_or_default();
    let cfg = crate::managed_agents::effective_config::resolve_effective_config(
        record,
        &personas,
        &global_config,
    )
    .require_resolved()?;
    let effective_model = cfg.model.value;
    let effective_provider = cfg.provider.value;
    let effective_prompt = cfg.system_prompt.value;

    Ok(deploy_payload_json(
        record,
        crate::relay::effective_agent_relay_url(
            &record.relay_url,
            &relay_ws_url_with_override(state),
        ),
        effective_model,
        effective_provider,
        effective_prompt,
        merged_env,
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
    merged_env: std::collections::BTreeMap<String, String>,
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
    })
}
