//! Provider deploy payload construction, split from `agents.rs` (file-size
//! guard). `build_deploy_payload` gathers live state; `deploy_payload_json`
//! is the pure serialization half so payload completeness stays testable.

use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{load_personas, AgentDefinition, ManagedAgentRecord},
    relay::relay_ws_url_with_override,
};

/// Resolve the deploy-specific structured model/provider for a managed agent.
///
/// Deploy uses **live-persona-first** precedence so remote agents receive
/// current config after a persona update, without requiring delete+recreate.
/// Unlike local spawn (which re-snapshots the persona onto `record` at the
/// start of every spawn), provider start does not re-snapshot — so the
/// record may hold a stale snapshot while the linked persona has moved on.
///
/// Precedence: live-persona → record (snapshot fallback) → global.
/// Symmetric for both model and provider.
///
/// Exported `pub(crate)` for unit testing.
pub(crate) fn resolve_deploy_model_provider<'a>(
    record: &'a ManagedAgentRecord,
    personas: &'a [AgentDefinition],
    global: &'a crate::managed_agents::GlobalAgentConfig,
) -> (Option<&'a str>, Option<&'a str>) {
    let live_persona = record
        .persona_id
        .as_deref()
        .and_then(|pid| personas.iter().find(|p| p.id == pid));
    let model = live_persona
        .and_then(|p| p.model.as_deref())
        .or(record.model.as_deref())
        .or(global.model.as_deref());
    let provider = live_persona
        .and_then(|p| p.provider.as_deref())
        .or(record.provider.as_deref())
        .or(global.provider.as_deref());
    (model, provider)
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
pub(crate) fn build_deploy_payload(
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

    // Resolve the deploy-specific structured provider/model. Uses the deploy
    // resolver with live-persona → record → global precedence.
    let personas = load_personas(app).unwrap_or_default();
    let (effective_model, effective_provider) =
        resolve_deploy_model_provider(record, &personas, &global_config);
    let (effective_model, effective_provider) = (
        effective_model.map(str::to_string),
        effective_provider.map(str::to_string),
    );

    Ok(deploy_payload_json(
        record,
        crate::relay::effective_agent_relay_url(
            &record.relay_url,
            &relay_ws_url_with_override(state),
        ),
        effective_model,
        effective_provider,
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
    merged_env: std::collections::BTreeMap<String, String>,
) -> serde_json::Value {
    serde_json::json!({
        "name": &record.name,
        // Resolve the per-agent pin against the active workspace relay here:
        // this payload crosses the host boundary to a remote provider harness
        // that has no notion of the desktop's workspace, so the blank→workspace
        // fallback (otherwise applied at read-time in `effective_agent_relay_url`)
        // must be materialized into a concrete URL before serializing.
        "relay_url": relay_url,
        "private_key_nsec": &record.private_key_nsec,
        "auth_tag": &record.auth_tag,
        "agent_command": &record.agent_command,
        "agent_args": &record.agent_args,
        "system_prompt": &record.system_prompt,
        "model": effective_model,
        // Structured provider from the persona record. Providers that don't
        // yet read this field will fall back to env_vars or their own default
        // — no protocol break.
        "provider": effective_provider,
        "turn_timeout_seconds": record.turn_timeout_seconds,
        "idle_timeout_seconds": record.idle_timeout_seconds,
        "max_turn_duration_seconds": record.max_turn_duration_seconds,
        "parallelism": record.parallelism,
        // Inbound author gate. Providers that don't yet read these fall back
        // to the harness default (`owner-only`) — no protocol break.
        "respond_to": record.respond_to,
        "respond_to_allowlist": &record.respond_to_allowlist,
        // Merged persona + agent env vars. Providers that don't read this
        // field will simply ignore it — no protocol break.
        "env_vars": merged_env,
    })
}
