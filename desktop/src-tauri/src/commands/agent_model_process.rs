use std::{collections::BTreeMap, path::PathBuf};

use crate::managed_agents::{
    build_buzz_agent_provider_defaults, default_agent_workdir, known_acp_runtime,
    redact_env_values_in, AgentModelsResponse,
};

use super::agent_models::normalize_agent_models;

/// Run a lightweight `buzz-acp` helper subcommand (`models`, `complete`, ...)
/// against the configured agent and parse its JSON stdout.
///
/// `helper_args` is the full subcommand argv (e.g. `["models", "--json"]`).
/// Env layering mirrors runtime agent spawn: augmented PATH, runtime default
/// env, baked provider defaults, then user env last so it always wins.
pub(super) async fn run_acp_helper_subprocess(
    resolved_acp: PathBuf,
    agent_command: String,
    agent_args: Vec<String>,
    merged_env: BTreeMap<String, String>,
    helper_args: Vec<String>,
) -> Result<serde_json::Value, String> {
    let subcommand = helper_args
        .first()
        .cloned()
        .unwrap_or_else(|| "helper".into());
    // Clone the env map for redaction below — `merged_env` is moved
    // into the spawn_blocking closure and we still need the values to
    // scrub any user-supplied secrets that the child surfaces in stderr.
    let env_for_redaction = merged_env.clone();
    let subcommand_in_closure = subcommand.clone();

    // Use spawn_blocking because the desktop Tauri crate doesn't enable
    // tokio's `process` feature. std::process::Command is synchronous
    // but fine for a short-lived subprocess.
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
        cmd.args(&helper_args)
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
            .map_err(|e| format!("failed to spawn buzz-acp {subcommand_in_closure}: {e}"))
    })
    .await
    .map_err(|e| format!("buzz-acp {subcommand} task failed: {e}"))?
    .map_err(|e: String| e)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Scrub any user-supplied env values before surfacing stderr to
        // the frontend — persona/agent env_vars may carry API keys that
        // a failing child process echoed back.
        let stderr_redacted = redact_env_values_in(stderr.as_ref(), &env_for_redaction);
        return Err(format!(
            "buzz-acp {subcommand} failed (exit {}): {stderr_redacted}",
            output.status.code().unwrap_or(-1)
        ));
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("failed to parse buzz-acp {subcommand} JSON: {e}"))
}

pub(super) async fn run_agent_models_command(
    resolved_acp: PathBuf,
    agent_command: String,
    agent_args: Vec<String>,
    persisted_model: Option<String>,
    merged_env: BTreeMap<String, String>,
) -> Result<AgentModelsResponse, String> {
    let raw = run_acp_helper_subprocess(
        resolved_acp,
        agent_command,
        agent_args,
        merged_env,
        vec!["models".into(), "--json".into()],
    )
    .await?;

    Ok(normalize_agent_models(&raw, persisted_model))
}
