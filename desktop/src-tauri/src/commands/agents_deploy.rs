//! Provider deploy payload construction, split from `agents.rs` (file-size
//! guard). `build_deploy_payload` gathers live state; `deploy_payload_json`
//! is the pure serialization half so payload completeness stays testable.

use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{load_personas, ManagedAgentRecord},
    relay::relay_ws_url_with_override,
};

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

    // Merge persona env_vars + agent env_vars for provider deploy — the same
    // live-persona-under-overrides semantics as local spawn. Without this,
    // provider-backed agents wouldn't receive credentials saved on the persona
    // or the agent itself.
    let persona_env =
        crate::managed_agents::resolve_persona_env(app, record.persona_id.as_deref())?;
    let merged_env = crate::managed_agents::merged_user_env(&persona_env, &record.env_vars);

    // Resolve the persona's structured provider/model so the remote provider
    // receives the same authoritative values that local spawn derives from
    // `runtime_metadata_env_vars`. Without this, remote deploy would rely on
    // stale derived env copies in `env_vars` (or have no provider at all for
    // imported personas whose derived keys were filtered at import time).
    //
    // Precedence: persona field wins when non-blank; otherwise falls back to the
    // record's own field (same blank-normalization as the snapshot path). This
    // matches `persona_snapshot_with_agent_config_fallback` exactly — a blank
    // persona field must not wipe a record value that the user configured.
    let (effective_model, effective_provider) = if let Some(pid) = record.persona_id.as_deref() {
        let personas = load_personas(app).map_err(|e| {
            format!(
                "failed to load personas while building deploy payload for persona `{pid}`: {e}"
            )
        })?;
        let persona = personas
            .into_iter()
            .find(|p| p.id == pid)
            .ok_or_else(|| format!("persona `{pid}` not found while building deploy payload"))?;
        let fallback = crate::managed_agents::persona_events::persona_field_with_record_fallback;
        let model = fallback(persona.model.as_deref(), record.model.as_deref()); // fallback: record.model
        let provider = fallback(persona.provider.as_deref(), record.provider.as_deref()); // fallback: record.provider
        (model, provider)
    } else {
        (record.model.clone(), record.provider.clone())
    };

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
        // MCP toolset filter (BUZZ_TOOLSETS on the local spawn path).
        // Providers that don't yet read this fall back to their default.
        "mcp_toolsets": &record.mcp_toolsets,
        // Merged persona + agent env vars. Providers that don't read this
        // field will simply ignore it — no protocol break.
        "env_vars": merged_env,
    })
}
