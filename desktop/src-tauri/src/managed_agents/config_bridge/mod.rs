mod buzz_agent;
mod claude;
mod codex;
mod goose;
mod pi;
pub(crate) mod reader;
mod schema_walker;
pub(crate) mod types;

pub(crate) use types::*;

/// Read the goose harness config file (`~/.config/goose/config.yaml`).
///
/// Used by readiness evaluation to silence requirements that are already
/// satisfied in the file config layer — the harness reads this file at startup
/// so env vars we would otherwise require are not needed from Buzz.
pub(crate) fn read_goose_file_config() -> Option<RuntimeFileConfig> {
    goose::read_config_file()
}

fn pi_mcp_write_target(
    runtime: Option<&crate::managed_agents::KnownAcpRuntime>,
    workdir: Option<&std::path::Path>,
    nest: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    if !runtime.is_some_and(|runtime| runtime.id == "pi") {
        return None;
    }
    let workdir = workdir?;
    let nest = nest?;
    let nest_is_real_dir = nest
        .symlink_metadata()
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false);
    (nest_is_real_dir && workdir == nest).then(|| nest.to_path_buf())
}

/// Write pi's MCP config only when the spawn CWD is the real Buzz nest.
/// Failures are non-fatal but are appended to the per-agent runtime log.
pub(crate) fn prepare_pi_workdir_mcp_json(
    runtime: Option<&crate::managed_agents::KnownAcpRuntime>,
    workdir: Option<&std::path::Path>,
    nest: Option<&std::path::Path>,
    log_path: &std::path::Path,
) {
    if !runtime.is_some_and(|runtime| runtime.id == "pi") {
        return;
    }
    let result = pi_mcp_write_target(runtime, workdir, nest)
        .as_deref()
        .ok_or_else(|| "refusing to write outside the real Buzz nest".to_string())
        .and_then(pi::ensure_workdir_mcp_json);
    if let Err(error) = result {
        let message = format!("buzz-desktop: failed to write pi mcp.json in nest: {error}");
        eprintln!("{message}");
        if let Err(log_error) = crate::managed_agents::append_log_marker(log_path, &message) {
            eprintln!(
                "buzz-desktop: failed to append pi mcp.json error to {}: {log_error}",
                log_path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pi_mcp_write_target_rejects_non_nest_workdir() {
        let temp = tempfile::tempdir().unwrap();
        let nest = temp.path().join("nest");
        let fallback_home = temp.path().join("home");
        std::fs::create_dir(&nest).unwrap();
        std::fs::create_dir(&fallback_home).unwrap();
        let pi = crate::managed_agents::known_acp_runtime("pi-acp").expect("should resolve");

        assert_eq!(
            super::pi_mcp_write_target(Some(pi), Some(&fallback_home), Some(&nest)),
            None
        );
    }
}
