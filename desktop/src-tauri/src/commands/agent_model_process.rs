use std::{collections::BTreeMap, path::PathBuf};

use crate::managed_agents::{
    build_buzz_agent_provider_defaults, default_agent_workdir, known_acp_runtime,
    redact_env_values_in, AgentModelsResponse,
};

use super::agent_models::normalize_agent_models;

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
        if let Some(ref path) = crate::managed_agents::login_shell_path() {
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
