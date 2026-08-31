//! Managed-agent runtime-log markers: generic run-lifecycle lines and the
//! resolved-launch marker recorded immediately before spawn.
//!
//! Split out of `storage.rs` to keep that module under the desktop file-size
//! ratchet. The resolved-launch marker deliberately serializes only the agent
//! command and the argument count — never argument values — mirroring the
//! `spawn_snapshot::diff` `MaskedBare` policy for `args`.

use std::io::Write;
use std::path::Path;

use super::storage::open_log_file;
use crate::managed_agents::resolve_command;

pub(crate) fn append_log_marker(path: &Path, message: &str) -> Result<(), String> {
    let mut file = open_log_file(path)?;
    writeln!(file, "{message}").map_err(|error| format!("failed to write log marker: {error}"))
}

/// Resolve `command` to a full path (DMG launches have a minimal PATH), append
/// the resolved-launch marker, and return the resolved command for the spawn
/// env (`BUZZ_ACP_AGENT_COMMAND`). The marker records the agent command and
/// the argument count, and nothing else: `agent_args` is user-controlled and
/// may legally carry credentials (`--token=...`), and the runtime log is
/// retrievable end-to-end via `get_managed_agent_log`, so no argument value
/// may ever be serialized — mirroring the `spawn_snapshot::diff` policy,
/// which masks `args` as `MaskedBare` for exactly this reason. When
/// resolution fails, both the marker and the return value carry the
/// unresolved command verbatim.
pub(crate) fn append_resolved_launch_marker(
    path: &Path,
    command: &str,
    args: &[String],
) -> Result<String, String> {
    let resolved = resolve_command(command)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| command.to_string());
    let marker = format!(
        "resolved launch: agent_command={resolved:?} args_count={}",
        args.len()
    );
    append_log_marker(path, &marker)?;
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_resolved_launch_marker_never_writes_argument_values() {
        // `agent_args` is user-controlled and may legally carry credentials
        // (`--token=...`), and the runtime log is retrievable end-to-end via
        // `get_managed_agent_log`, so the marker may carry only the command and
        // the argument count. The command is a nonexistent absolute path so
        // `acp` can only reach the log through an argument value.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent.log");

        let resolved = append_resolved_launch_marker(
            &path,
            "/nonexistent/zuzu-agent-cli",
            &["acp".to_string(), "--token=supersecret-value".to_string()],
        )
        .expect("append marker");

        // Unresolvable commands fall through verbatim for the spawn env.
        assert_eq!(resolved, "/nonexistent/zuzu-agent-cli");
        let logged = std::fs::read_to_string(&path).expect("read log");
        assert!(logged.contains("args_count=2"));
        assert!(logged.contains("/nonexistent/zuzu-agent-cli"));
        assert!(!logged.contains("supersecret-value"));
        assert!(!logged.contains("--token"));
        assert!(!logged.contains("acp"));
    }
}
