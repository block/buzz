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
    run_agent_models_command_with_path(
        resolved_acp,
        agent_command,
        agent_args,
        persisted_model,
        merged_env,
        crate::managed_agents::readiness::cli_probe::augmented_path,
    )
    .await
}

async fn run_agent_models_command_with_path<PathProvider>(
    resolved_acp: PathBuf,
    agent_command: String,
    agent_args: Vec<String>,
    persisted_model: Option<String>,
    merged_env: BTreeMap<String, String>,
    path_provider: PathProvider,
) -> Result<AgentModelsResponse, String>
where
    PathProvider: FnOnce() -> Option<String> + Send + 'static,
{
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
        // Same PATH as runtime spawn / CLI probes: managed node + npm bins
        // ahead of the login-shell PATH so `#!/usr/bin/env node` ACP shims
        // resolve when Buzz is launched from a GUI (no interactive shell).
        if let Some(path) = path_provider() {
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

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[tokio::test]
    async fn model_discovery_uses_augmented_path_for_node_adapter() {
        use std::collections::BTreeMap;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let interpreter_dir = temp.path().join("interpreter-bin");
        fs::create_dir_all(&interpreter_dir).expect("interpreter dir");

        let marker_path = temp.path().join("fake-node-ran");
        let node_path = interpreter_dir.join("node");
        fs::write(
            &node_path,
            format!(
                "#!/bin/sh\nprintf 'fake node ran\\n' > '{}' || exit 1\nprintf '%s\\n' '{{\"agent\":{{\"name\":\"fake-agent\",\"version\":\"1.0\"}},\"unstable\":{{\"currentModelId\":\"fake-model\",\"availableModels\":[{{\"modelId\":\"fake-model\",\"name\":\"Fake Model\"}}]}}}}'\n",
                marker_path.display()
            ),
        )
        .expect("write fake node");
        fs::set_permissions(&node_path, fs::Permissions::from_mode(0o755))
            .expect("chmod fake node");

        let adapter_path = temp.path().join("codex-acp");
        fs::write(&adapter_path, "#!/usr/bin/env node\n").expect("write adapter");
        fs::set_permissions(&adapter_path, fs::Permissions::from_mode(0o755))
            .expect("chmod adapter");

        let acp_path = temp.path().join("buzz-acp");
        fs::write(&acp_path, "#!/bin/sh\nexec \"$BUZZ_ACP_AGENT_COMMAND\"\n")
            .expect("write buzz-acp");
        fs::set_permissions(&acp_path, fs::Permissions::from_mode(0o755)).expect("chmod buzz-acp");

        let augmented_path = std::env::join_paths([interpreter_dir.as_path()])
            .expect("join augmented PATH")
            .to_string_lossy()
            .into_owned();
        let response = super::run_agent_models_command_with_path(
            acp_path,
            adapter_path.display().to_string(),
            Vec::new(),
            None,
            BTreeMap::new(),
            move || Some(augmented_path),
        )
        .await
        .expect("discover models through fake node adapter");

        assert!(marker_path.exists(), "the fake node interpreter should run");
        assert_eq!(response.agent_name, "fake-agent");
        assert_eq!(response.agent_version, "1.0");
        assert_eq!(response.models.len(), 1);
        assert_eq!(response.models[0].id, "fake-model");
        assert_eq!(response.agent_default_model.as_deref(), Some("fake-model"));
    }
}
