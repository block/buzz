use crate::managed_agents::discovery::{clear_resolve_cache, resolve_command};

#[cfg(unix)]
#[test]
fn resolve_command_prefers_buzz_managed_npm_shim_over_path() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = crate::managed_agents::lock_path_mutex();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let xdg_data = temp.path().join("xdg-data");
    let global_bin = temp.path().join("global-bin");
    std::fs::create_dir_all(&home).expect("create home");
    std::fs::create_dir_all(&xdg_data).expect("create xdg data");
    std::fs::create_dir_all(&global_bin).expect("create global bin");

    let old_home = std::env::var_os("HOME");
    let old_xdg_data = std::env::var_os("XDG_DATA_HOME");
    let old_path = std::env::var_os("PATH").unwrap_or_default();

    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_DATA_HOME", &xdg_data);
    let managed_bin = dirs::data_dir()
        .expect("data dir")
        .join("Buzz")
        .join("node-tools")
        .join("bin");
    std::fs::create_dir_all(&managed_bin).expect("create managed bin");

    let managed_shim = managed_bin.join("codex-acp");
    let global_shim = global_bin.join("codex-acp");
    std::fs::write(&managed_shim, "#!/bin/sh\necho managed\n").expect("write managed shim");
    std::fs::write(&global_shim, "#!/bin/sh\necho global\n").expect("write global shim");
    std::fs::set_permissions(&managed_shim, std::fs::Permissions::from_mode(0o755))
        .expect("chmod managed shim");
    std::fs::set_permissions(&global_shim, std::fs::Permissions::from_mode(0o755))
        .expect("chmod global shim");

    let new_path = std::env::join_paths(
        std::iter::once(global_bin.clone()).chain(std::env::split_paths(&old_path)),
    )
    .expect("join PATH");
    std::env::set_var("PATH", new_path);
    clear_resolve_cache();

    let resolved = resolve_command("codex-acp");

    std::env::set_var("PATH", &old_path);
    match old_home {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
    match old_xdg_data {
        Some(value) => std::env::set_var("XDG_DATA_HOME", value),
        None => std::env::remove_var("XDG_DATA_HOME"),
    }
    clear_resolve_cache();

    assert_eq!(
        resolved.as_deref(),
        Some(managed_shim.as_path()),
        "Buzz-managed npm shim must win over PATH/global shims"
    );
}

/// The managed-command allowlist deliberately does not cover preset adapters.
///
/// `buzz_managed_command_path` short-circuits on an explicit allowlist
/// (`codex-acp`, `claude-agent-acp`, `claude-code-acp`, `node`, `npm`). Every
/// preset adapter — `amp-acp` today — is therefore resolved by the generic
/// `common_binary_paths()` / PATH scan instead. This pins that boundary, so the
/// generic-resolution test below is exercising the path it claims to.
#[cfg(unix)]
#[test]
fn managed_command_allowlist_excludes_preset_adapters() {
    use crate::managed_agents::buzz_managed_command_path;

    assert!(
        buzz_managed_command_path("amp-acp", "amp-acp").is_none(),
        "amp-acp must not take the allowlisted managed-command path"
    );
}

/// A preset adapter must be discovered as `Available` through the real
/// resolver and the real discovery seam.
///
/// The existing managed-prefix test above uses `codex-acp`, which
/// `buzz_managed_command_path` resolves via its explicit allowlist. `amp-acp`
/// is excluded from that allowlist (pinned by the test above), so it can only
/// be found by generic resolution — the path every preset adapter depends on.
/// This drives `resolve_command` and `discover_acp_runtime_availability`
/// against real `PRESET_HARNESSES` data with no injected resolver and no
/// network.
///
/// The shim is placed on a `PATH` whose first entry is the tempdir.
/// `resolve_command_uncached` consults `PATH` before it consults the login
/// shell, so the shim wins deterministically even on a machine that has a real
/// `amp-acp` installed (this one does, via Homebrew) — which would otherwise
/// let the test pass for the wrong reason.
#[cfg(unix)]
#[test]
fn preset_adapter_on_path_is_discovered_available() {
    use std::os::unix::fs::PermissionsExt;

    use crate::managed_agents::discovery::discover_acp_runtime_availability;
    use crate::managed_agents::AcpAvailabilityStatus;

    let _guard = crate::managed_agents::lock_path_mutex();
    let temp = tempfile::tempdir().expect("tempdir");
    let adapter_bin = temp.path().join("adapter-bin");
    std::fs::create_dir_all(&adapter_bin).expect("create adapter bin");

    let shim = adapter_bin.join("amp-acp");
    std::fs::write(&shim, "#!/bin/sh\nexit 0\n").expect("write amp-acp shim");
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("chmod shim");

    let old_path = std::env::var_os("PATH").unwrap_or_default();
    std::env::set_var("PATH", &adapter_bin);
    clear_resolve_cache();

    let resolved = resolve_command("amp-acp");
    let availability = discover_acp_runtime_availability("amp");

    std::env::set_var("PATH", &old_path);
    clear_resolve_cache();

    assert_eq!(
        resolved.as_deref(),
        Some(shim.as_path()),
        "the shim under test must be the resolved binary, not a machine-wide install"
    );
    assert_eq!(
        availability,
        Some(AcpAvailabilityStatus::Available),
        "a resolvable preset adapter must read as Available"
    );
}
