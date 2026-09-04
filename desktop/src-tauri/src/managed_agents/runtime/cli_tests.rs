//! Runtime CLI configuration regression tests kept beside the configured seam.

use super::super::configure_runtime_cli;
use crate::managed_agents::known_acp_runtime;

#[test]
fn claude_spawn_uses_the_probed_cli_executable() {
    let _guard = crate::managed_agents::lock_path_mutex();
    let temp = tempfile::tempdir().expect("temp dir");
    let cli = temp
        .path()
        .join(format!("claude{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&cli, "").expect("write fake cli");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755))
            .expect("make fake cli executable");
    }
    let original_path = std::env::var_os("PATH");
    std::env::set_var("PATH", temp.path());
    // The resolver retains negative results across tests, so the fake CLI must
    // invalidate both before configuration and after restoring PATH.
    crate::managed_agents::clear_resolve_cache();

    let mut command = std::process::Command::new("buzz-acp");
    configure_runtime_cli(&mut command, known_acp_runtime("claude-agent-acp"));

    if let Some(path) = original_path {
        std::env::set_var("PATH", path);
    } else {
        std::env::remove_var("PATH");
    }
    crate::managed_agents::clear_resolve_cache();
    assert!(command
        .get_envs()
        .any(|(key, value)| { key == "CLAUDE_CODE_EXECUTABLE" && value == Some(cli.as_os_str()) }));
}
