use crate::managed_agents::{
    discover_provider_candidates, invoke_provider, provider_discover_harnesses,
    provider_probe_models, validate_provider_config, BackendProviderInfo, ProviderFailure,
};

/// Resolve a frontend-supplied binary path to a canonical path that is
/// provably one of the discovered `buzz-backend-*` providers.
///
/// Every provider command must go through this. Without it, the `binaryPath`
/// argument is an arbitrary-execution primitive for a compromised frontend or
/// any process that can reach the IPC channel: the desktop would spawn
/// whatever it names and feed it the agent's private key.
/// The provider commands below return [`ProviderFailure`] rather than `String`
/// so a failure carrying a recovery keeps it all the way to the frontend, which
/// reads the serialized `{message, recovery?}` off `TauriInvokeError.payload`.
/// Every non-provider failure on these paths — a path that will not resolve, a
/// config that will not validate, a join that panicked — converts through
/// `.into()` and simply has no recovery.
fn resolve_discovered_provider(binary_path: &str) -> Result<std::path::PathBuf, String> {
    let canonical = std::path::PathBuf::from(binary_path)
        .canonicalize()
        .map_err(|e| format!("binary not found: {binary_path}: {e}"))?;
    let is_known = discover_provider_candidates()
        .iter()
        .any(|(_, p)| p.canonicalize().ok().as_ref() == Some(&canonical));
    if !is_known {
        return Err(format!(
            "binary '{binary_path}' is not a discovered buzz-backend-* provider"
        ));
    }
    Ok(canonical)
}

#[tauri::command]
pub async fn discover_backend_providers() -> Result<Vec<BackendProviderInfo>, String> {
    tokio::task::spawn_blocking(|| {
        discover_provider_candidates()
            .into_iter()
            .map(|(id, path)| BackendProviderInfo {
                id,
                binary_path: path.display().to_string(),
            })
            .collect()
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

#[tauri::command]
pub async fn probe_backend_provider(
    binary_path: String,
) -> Result<serde_json::Value, ProviderFailure> {
    let canonical = resolve_discovered_provider(&binary_path)?;
    // request_id is for provider-side logging — not validated in the response
    // (stdin→stdout is 1:1 per process invocation).
    let request = serde_json::json!({
        "op": "info",
        "request_id": uuid::Uuid::new_v4().to_string(),
    });
    tokio::task::spawn_blocking(move || {
        invoke_provider(&canonical, &request, std::time::Duration::from_secs(10))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// List the harnesses installed on the machine a provider deploys to.
///
/// The create dialog needs this for a remote agent because the local runtime
/// catalog describes THIS computer. The `command` of the entry the user picks
/// becomes the create-time `agentCommand` pin — the only channel by which the
/// harness choice reaches the host.
#[tauri::command]
pub async fn discover_provider_harnesses(
    binary_path: String,
    config: serde_json::Value,
) -> Result<serde_json::Value, ProviderFailure> {
    let canonical = resolve_discovered_provider(&binary_path)?;
    validate_provider_config(&config)?;
    tokio::task::spawn_blocking(move || provider_discover_harnesses(&canonical, &config))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Model catalog for one remote harness, normalized through the same
/// `normalize_agent_models` the local path uses so the model picker needs no
/// remote-specific rendering code.
#[tauri::command]
pub async fn probe_provider_models(
    binary_path: String,
    config: serde_json::Value,
    harness: serde_json::Value,
    env_vars: Option<std::collections::BTreeMap<String, String>>,
) -> Result<crate::managed_agents::AgentModelsResponse, ProviderFailure> {
    let canonical = resolve_discovered_provider(&binary_path)?;
    validate_provider_config(&config)?;
    let env_vars = env_vars.unwrap_or_default();
    let response = tokio::task::spawn_blocking(move || {
        provider_probe_models(&canonical, &config, &harness, &env_vars)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    let models_raw = response
        .get("models_raw")
        .ok_or("probe_models response is missing 'models_raw'")?;
    // `persisted_model: None` — this probes a harness during CREATE, before any
    // agent record exists to have persisted a selection.
    Ok(super::agent_models::normalize_agent_models(
        models_raw, None,
    ))
}
