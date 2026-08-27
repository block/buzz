use std::{collections::BTreeMap, path::PathBuf};

use crate::managed_agents::{
    build_buzz_agent_provider_defaults, default_agent_workdir, known_acp_runtime,
    redact_env_values_in, AgentModelInfo, AgentModelsResponse,
};

use super::agent_models::normalize_agent_models;

/// Live discovery through the agent binary: `buzz-agent models` dispatches
/// on `BUZZ_AGENT_PROVIDER` inside the agent, which owns every provider
/// transport, including ones the desktop cannot probe over plain HTTP. It
/// needs no configured model, so it also serves draft configs in the
/// create/edit dialog.
///
/// Returns `Ok(None)` when the runtime is not buzz-agent or the catalog is
/// empty, and callers fall through to their next discovery step. Errors
/// propagate so credential failures reach the dialog.
pub(super) async fn run_buzz_agent_native_models(
    agent_command: &str,
    resolved_agent: &str,
    merged_env: BTreeMap<String, String>,
    persisted_model: Option<String>,
) -> Result<Option<AgentModelsResponse>, String> {
    if known_acp_runtime(agent_command).map(|meta| meta.id) != Some("buzz-agent") {
        return Ok(None);
    }
    let env_for_redaction = merged_env.clone();
    let resolved_agent = resolved_agent.to_string();
    let output = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&resolved_agent);
        if let Some(home) = default_agent_workdir() {
            cmd.current_dir(home);
        }
        cmd.arg("models");
        // Mirror runtime spawn: internal builds may bake provider/model
        // defaults. User-provided env below still wins.
        build_buzz_agent_provider_defaults(&mut cmd);
        for (k, v) in &merged_env {
            cmd.env(k, v);
        }
        crate::util::configure_no_window(&mut cmd);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| format!("failed to spawn buzz-agent models: {e}"))
    })
    .await
    .map_err(|e| format!("model discovery task failed: {e}"))?
    .map_err(|e: String| e)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Scrub user-supplied env values (API keys) before surfacing stderr.
        let stderr_redacted = redact_env_values_in(stderr.as_ref(), &env_for_redaction);
        return Err(format!(
            "buzz-agent models failed (exit {}): {stderr_redacted}",
            output.status.code().unwrap_or(-1)
        ));
    }

    parse_native_models_output(&output.stdout, persisted_model)
}

/// Parse `buzz-agent models` stdout: a JSON array of `{"id","name"}`
/// objects. An empty array means the provider has no live catalog, returned
/// as `Ok(None)` so the caller falls through to its next discovery step.
pub(super) fn parse_native_models_output(
    stdout: &[u8],
    persisted_model: Option<String>,
) -> Result<Option<AgentModelsResponse>, String> {
    #[derive(serde::Deserialize)]
    struct NativeModel {
        id: String,
        name: Option<String>,
    }
    let raw: Vec<NativeModel> = serde_json::from_slice(stdout)
        .map_err(|e| format!("failed to parse buzz-agent models JSON: {e}"))?;
    if raw.is_empty() {
        return Ok(None);
    }
    Ok(Some(AgentModelsResponse {
        agent_name: "buzz-agent".to_string(),
        agent_version: "native-models".to_string(),
        models: raw
            .into_iter()
            .map(|m| AgentModelInfo {
                id: m.id,
                name: m.name,
                description: None,
            })
            .collect(),
        agent_default_model: None,
        selected_model: persisted_model,
        supports_switching: true,
    }))
}

pub(super) async fn run_agent_models_command(
    resolved_acp: PathBuf,
    agent_command: String,
    agent_args: Vec<String>,
    persisted_model: Option<String>,
    merged_env: BTreeMap<String, String>,
) -> Result<AgentModelsResponse, String> {
    // Clone the env map for redaction below — `merged_env` is moved
    // into the spawn_blocking closure and we still need the values to
    // scrub any user-supplied secrets that the child surfaces in stderr.
    let env_for_redaction = merged_env.clone();

    // Use spawn_blocking because the desktop Tauri crate doesn't enable
    // tokio's `process` feature. std::process::Command is synchronous
    // but fine for a short-lived subprocess (~2-5s).
    let output = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&resolved_acp);
        if let Some(home) = default_agent_workdir() {
            cmd.current_dir(home);
        }
        // Inject the same augmented PATH used for launched agents and CLI
        // probes: managed Node/npm dirs, exe-parent sidecars, login-shell
        // PATH, and (Windows) the inherited process PATH. login_shell_path()
        // alone is always None on Windows, which left this child with no
        // managed Node dirs — the ACP adapter's `.cmd` shims then failed with
        // `'node' is not recognized` and the model dropdown stayed empty.
        if let Some(ref path) = crate::managed_agents::readiness::cli_probe::augmented_path() {
            cmd.env("PATH", path);
        }
        cmd.arg("models")
            .arg("--json")
            .env("BUZZ_ACP_AGENT_COMMAND", &agent_command)
            .env("BUZZ_ACP_AGENT_ARGS", agent_args.join(","));
        if let Some(meta) = known_acp_runtime(&agent_command) {
            for (key, value) in meta.default_env {
                if std::env::var(key).is_err() {
                    cmd.env(key, value);
                }
            }
        }
        // Mirror runtime spawn: internal builds may bake provider/model
        // defaults. User-provided env below still wins.
        build_buzz_agent_provider_defaults(&mut cmd);
        // User env layering — written LAST so it overrides any Buzz-set env above.
        for (k, v) in &merged_env {
            cmd.env(k, v);
        }
        crate::managed_agents::configure_runtime_cli(&mut cmd, known_acp_runtime(&agent_command));
        crate::util::configure_no_window(&mut cmd);
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| format!("failed to spawn buzz-acp models: {e}"))
    })
    .await
    .map_err(|e| format!("model discovery task failed: {e}"))?
    .map_err(|e: String| e)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Scrub any user-supplied env values before surfacing stderr to
        // the frontend — persona/agent env_vars may carry API keys that
        // a failing child process echoed back.
        let stderr_redacted = redact_env_values_in(stderr.as_ref(), &env_for_redaction);
        return Err(format!(
            "buzz-acp models failed (exit {}): {stderr_redacted}",
            output.status.code().unwrap_or(-1)
        ));
    }

    let raw: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("failed to parse model JSON: {e}"))?;

    Ok(normalize_agent_models(&raw, persisted_model))
}
