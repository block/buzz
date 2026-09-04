use std::{collections::BTreeMap, path::PathBuf};

use crate::managed_agents::{
    build_buzz_agent_provider_defaults, default_agent_workdir, known_acp_runtime,
    redact_env_values_in, resolve_command, AgentModelsResponse,
};

use super::agent_models::normalize_agent_models;

/// WSL env decision for the `models --json` subprocess, mirroring the spawn
/// path in `runtime.rs`.
///
/// When the agent command cannot be resolved on the host PATH but discovery
/// located it inside the default WSL distribution, the in-distro path is
/// handed to `buzz-acp` through `BUZZ_ACP_AGENT_WSL_PATH` (and the optional
/// `BUZZ_ACP_AGENT_WSL_DISTRO`), so the model picker can spawn a WSL-only
/// harness command instead of failing with "program not found".
///
/// For native or unresolved commands the ambient WSL vars are explicitly
/// removed, so a stale parent environment cannot opt another probe into WSL
/// mode. Returns the env pairs to set (empty value = remove ambient var).
fn wsl_model_discovery_env(agent_command: &str) -> Vec<(String, String)> {
    let host_resolved = resolve_command(agent_command).is_some();
    let resolution = if host_resolved {
        None
    } else {
        crate::managed_agents::wsl::probe_wsl_command(agent_command)
    };
    wsl_model_discovery_env_from_resolution(resolution)
}

/// Pure core of [`wsl_model_discovery_env`], split out so the three branches
/// (WSL propagation, native precedence, unresolved cleanup) are testable on
/// any host without a live WSL probe.
fn wsl_model_discovery_env_from_resolution(
    resolution: Option<crate::managed_agents::wsl::WslCommandResolution>,
) -> Vec<(String, String)> {
    match resolution {
        Some(resolution) => {
            let mut env = vec![("BUZZ_ACP_AGENT_WSL_PATH".to_string(), resolution.linux_path)];
            if let Some(distro) = resolution.distro {
                env.push(("BUZZ_ACP_AGENT_WSL_DISTRO".to_string(), distro));
            }
            env
        }
        None => vec![
            ("BUZZ_ACP_AGENT_WSL_PATH".to_string(), String::new()),
            ("BUZZ_ACP_AGENT_WSL_DISTRO".to_string(), String::new()),
        ],
    }
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
        // WSL fallback: mirror the spawn path so a WSL-only harness command
        // resolves in the model picker. Native/unresolved commands get the
        // ambient WSL vars removed so a stale parent env cannot force WSL mode.
        for (key, value) in wsl_model_discovery_env(&agent_command) {
            if value.is_empty() {
                cmd.env_remove(&key);
            } else {
                cmd.env(&key, value);
            }
        }
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
        // Demo identity is authoritative and must win over ambient/user env.
        crate::build_identity::apply_demo_config_home(&mut cmd)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::wsl::WslCommandResolution;

    fn resolution(linux_path: &str, distro: Option<&str>) -> WslCommandResolution {
        WslCommandResolution {
            distro: distro.map(str::to_string),
            linux_path: linux_path.to_string(),
        }
    }

    #[test]
    fn wsl_resolution_propagates_path_and_distro() {
        let env = wsl_model_discovery_env_from_resolution(Some(resolution(
            "/home/rat/.local/bin/codex-wsl-acp",
            Some("Ubuntu"),
        )));
        assert_eq!(
            env,
            vec![
                (
                    "BUZZ_ACP_AGENT_WSL_PATH".to_string(),
                    "/home/rat/.local/bin/codex-wsl-acp".to_string()
                ),
                ("BUZZ_ACP_AGENT_WSL_DISTRO".to_string(), "Ubuntu".to_string()),
            ]
        );
    }

    #[test]
    fn wsl_resolution_without_distro_sets_path_only() {
        let env = wsl_model_discovery_env_from_resolution(Some(resolution(
            "/usr/local/bin/omp",
            None,
        )));
        assert_eq!(
            env,
            vec![(
                "BUZZ_ACP_AGENT_WSL_PATH".to_string(),
                "/usr/local/bin/omp".to_string()
            )]
        );
    }

    #[test]
    fn unresolved_or_native_command_clears_ambient_wsl_vars() {
        // Native precedence and unresolved cleanup both land here: no WSL
        // resolution means the ambient vars must be removed so a stale parent
        // env cannot force WSL mode.
        let env = wsl_model_discovery_env_from_resolution(None);
        assert_eq!(
            env,
            vec![
                ("BUZZ_ACP_AGENT_WSL_PATH".to_string(), String::new()),
                ("BUZZ_ACP_AGENT_WSL_DISTRO".to_string(), String::new()),
            ]
        );
        // Empty value is the "remove ambient var" sentinel the builder honors.
        for (key, value) in &env {
            assert!(value.is_empty(), "expected removal sentinel for {key}");
        }
    }

    #[test]
    fn wsl_env_application_sets_or_removes_on_command() {
        // WSL resolution -> env set.
        let mut cmd = std::process::Command::new("true");
        for (key, value) in wsl_model_discovery_env_from_resolution(Some(resolution(
            "/home/rat/.local/bin/codex-wsl-acp",
            None,
        ))) {
            cmd.env(&key, value);
        }
        let env = cmd.get_envs().collect::<Vec<_>>();
        assert!(env.iter().any(|(k, v)| {
            *k == "BUZZ_ACP_AGENT_WSL_PATH"
                && v.as_deref() == Some(std::ffi::OsStr::new("/home/rat/.local/bin/codex-wsl-acp"))
        }));

        // No resolution -> ambient vars removed.
        let mut cmd = std::process::Command::new("true");
        cmd.env("BUZZ_ACP_AGENT_WSL_PATH", "stale");
        cmd.env("BUZZ_ACP_AGENT_WSL_DISTRO", "stale");
        for (key, value) in wsl_model_discovery_env_from_resolution(None) {
            if value.is_empty() {
                cmd.env_remove(&key);
            } else {
                cmd.env(&key, value);
            }
        }
        let env = cmd.get_envs().collect::<Vec<_>>();
        assert!(env.iter().any(|(k, v)| *k == "BUZZ_ACP_AGENT_WSL_PATH" && v.is_none()));
        assert!(env.iter().any(|(k, v)| *k == "BUZZ_ACP_AGENT_WSL_DISTRO" && v.is_none()));
    }
}
