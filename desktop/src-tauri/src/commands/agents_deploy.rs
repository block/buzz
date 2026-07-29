//! Provider deploy: payload construction and the deploy call itself, split
//! from `agents.rs` (file-size guard). `build_deploy_payload` gathers live
//! state; `deploy_payload_json` is the pure serialization half so payload
//! completeness stays testable; `deploy_to_provider` resolves the binary,
//! invokes it, and persists the outcome onto the record.

use tauri::AppHandle;

#[cfg(test)]
use crate::managed_agents::AgentDefinition;
use crate::{
    app_state::AppState,
    managed_agents::{
        discover_provider_candidates, load_managed_agents, load_personas, provider_deploy,
        resolve_provider_binary, save_managed_agents, ManagedAgentRecord, ProviderFailure,
    },
    relay::relay_ws_url_with_override,
    util::now_iso,
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
        BinariesToPush::from_env(),
    ))
}

/// The binaries this machine offers to install on a host that resolves none.
///
/// A pair rather than two loose arguments because they are resolved together
/// and read together by the provider, and because passing them in at all is
/// what keeps [`deploy_payload_json`] pure.
#[derive(Default)]
pub(super) struct BinariesToPush {
    pub buzz_acp: Option<String>,
    pub buzz_cli: Option<String>,
}

impl BinariesToPush {
    /// The dogfood seams as this process's environment currently reports them.
    ///
    /// Each var names a Linux binary on THIS machine; `buzz-backend-ssh`
    /// streams it to the host inside the deploy script and installs it to
    /// `~/.local/bin` when the host resolves none. Read at deploy time rather
    /// than captured at startup, so a developer can point one at a fresh build
    /// without restarting the app.
    ///
    /// Two vars rather than one because the two binaries are separately
    /// policed: a host with no `buzz-acp` cannot run an agent at all, while a
    /// host with no CLI runs one that simply cannot reply with
    /// `buzz messages send`.
    ///
    /// They are env vars and not settings because the durable answer is not a
    /// path at all: the release build should resolve the artifact for the
    /// host's platform by version, with no user-visible choice. Until that
    /// lands this is the whole surface.
    fn from_env() -> Self {
        Self {
            buzz_acp: binary_to_push("BUZZ_ACP_PUSH_BINARY"),
            buzz_cli: binary_to_push("BUZZ_CLI_PUSH_BINARY"),
        }
    }
}

/// Pure serialization half of [`build_deploy_payload`] — every field the
/// provider harness receives is deliberately listed here, so payload
/// completeness is testable without an `AppHandle`.
///
/// The push seams arrive as a parameter rather than being read from the
/// environment here: reading them inside would make this function's output a
/// property of the developer's shell (the seams are env vars a dogfooding
/// developer sets), which no test could pin down.
pub(super) fn deploy_payload_json(
    record: &ManagedAgentRecord,
    relay_url: String,
    effective_model: Option<String>,
    effective_provider: Option<String>,
    effective_prompt: Option<String>,
    merged_env: std::collections::BTreeMap<String, String>,
    binaries_to_push: BinariesToPush,
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
        // A path on THIS machine to a Linux `buzz-acp` the provider may install
        // on the host when the host has none. Serialized as `null` when unset —
        // a provider reading it with `as_str()` sees `None`, exactly as it does
        // for an absent key, so behavior is unchanged by default.
        "buzz_acp_binary": binaries_to_push.buzz_acp,
        // The same, for the `buzz` CLI. A local agent gets the CLI because the
        // desktop bundles it as a sidecar and prepends its directory to the
        // spawned harness's PATH; a remote agent is told by the very same system
        // prompt to reply with `buzz messages send`, so without this the remote
        // half of that contract is missing. Unlike `buzz-acp` its absence is a
        // warning on the provider side, never a failed deploy.
        "buzz_cli_binary": binaries_to_push.buzz_cli,
    })
}

/// A push seam's value, or `None` when unset or blank — blank being a var the
/// user cleared rather than a request to push an empty path.
fn binary_to_push(var: &str) -> Option<String> {
    std::env::var(var)
        .ok()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
}

/// Deploy an agent to a provider backend. Resolves the binary, calls deploy via
/// spawn_blocking, and persists the result (backend_agent_id or last_error).
///
/// Idempotency: calling deploy on an already-deployed agent sends the same payload
/// again. Providers are expected to handle this as an update-in-place or no-op —
/// the protocol does not include an explicit `undeploy` operation (deferred to v2).
///
/// Returns Ok(()) or a [`ProviderFailure`]; either way the record is updated
/// and saved first. The failure type is what keeps a recovery — a deploy onto a
/// tailnet wanting browser re-auth carries one — alive for callers that can
/// render it; each caller drops it explicitly where it cannot. The record's
/// `last_error` stays a plain string regardless: it is a post-mortem read long
/// after the fact, and an auth URL is a one-shot token that is stale by then.
pub(super) async fn deploy_to_provider(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    provider_id: &str,
    config: &serde_json::Value,
    agent_json: serde_json::Value,
    cached_binary_path: Option<&str>,
) -> Result<(), ProviderFailure> {
    // Resolve via discovered candidates only. Cached path must match BOTH
    // "is a discovered candidate" AND "belongs to this provider_id". A tampered
    // record cannot redirect deploys to a different provider's binary.
    let bin_path = cached_binary_path
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
        .map(|p| p.canonicalize().unwrap_or(p))
        .filter(|canonical| {
            discover_provider_candidates().iter().any(|(id, cp)| {
                id == provider_id && cp.canonicalize().ok().as_ref() == Some(canonical)
            })
        })
        .map_or_else(|| resolve_provider_binary(provider_id), Ok)?;

    let config_clone = config.clone();
    let deploy_result =
        tokio::task::spawn_blocking(move || provider_deploy(&bin_path, &agent_json, &config_clone))
            .await
            .map_err(|e| format!("spawn_blocking failed: {e}"))?;

    // Persist result under lock.
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(app)?;
    let rec = records
        .iter_mut()
        .find(|r| r.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))?;

    match deploy_result {
        Ok(backend_agent_id) => {
            rec.backend_agent_id = Some(backend_agent_id);
            rec.last_started_at = Some(now_iso());
            rec.updated_at = now_iso();
            rec.last_error = None;
        }
        Err(ref e) => {
            // The message only — see the `last_error` note above.
            rec.last_error = Some(e.message.clone());
            rec.updated_at = now_iso();
            save_managed_agents(app, &records)?;
            return Err(e.clone());
        }
    }
    save_managed_agents(app, &records)?;
    Ok(())
}
