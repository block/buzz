use crate::managed_agents::discovery::{clear_resolve_cache, resolve_command};

/// A login-shell command lookup must treat its argument as pure data — a
/// payload containing shell metacharacters must never execute.
#[test]
fn login_shell_lookup_treats_command_as_data() {
    use super::super::find_via_login_shell;

    let _guard = crate::managed_agents::lock_path_mutex();
    let marker =
        std::env::temp_dir().join(format!("buzz-discovery-marker-{}", uuid::Uuid::new_v4()));
    let payload = format!("doesnotexist; touch {} #", marker.display());

    let resolved = find_via_login_shell(&payload);

    assert!(
        resolved.is_none(),
        "payload should not resolve to a command"
    );
    assert!(
        !marker.exists(),
        "shell lookup must not execute injected commands"
    );
}

/// The legacy Goose Windows installer wrote `%USERPROFILE%\goose\goose.exe`,
/// a directory on no standard PATH. `resolve_command_uncached` finds binaries
/// outside PATH only by scanning `common_binary_paths()`, so that directory
/// must appear there or those installs stay undiscovered (#2239 residual).
///
/// Asserts the probe list rather than a planted binary: `common_binary_paths`
/// is a process-lifetime `OnceLock`, so a test cannot re-seed `USERPROFILE`
/// deterministically, and planting an executable under the real user profile
/// is not an acceptable test side effect.
#[cfg(windows)]
#[test]
fn common_binary_paths_probes_legacy_goose_install_dir() {
    use std::path::PathBuf;

    let profile = std::env::var_os("USERPROFILE").expect("USERPROFILE is always set on Windows");
    let legacy_dir = PathBuf::from(profile).join("goose");

    let probed = super::super::common_binary_paths();

    assert!(
        probed.contains(&legacy_dir),
        "legacy Goose install dir {} must be probed, got: {probed:?}",
        legacy_dir.display()
    );
}

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

/// The cheap discovery path must never spawn a login shell — not even on a
/// cold cache.
///
/// `force: false` resolves commands from cache only (`resolve_command_cached`):
/// on a resolve-cache miss it reports the command absent instead of falling
/// through to `resolve_command_uncached` → `find_via_login_shell`, which spawns
/// zsh/bash. That spawn on the channel-switch/composer hot path is the exact
/// freeze source the cheap path exists to avoid, so a cold cheap call must
/// spawn zero login shells. The forced path remains the sole prober: the same
/// absent-command fixture spawns at least once under `force: true`, proving the
/// cheap-path zero is real and not a fixture that never reaches the probe.
#[cfg(unix)]
#[test]
fn cheap_discovery_never_spawns_login_shell_even_when_cold() {
    use crate::managed_agents::custom_harnesses::registry_test_lock;
    use crate::managed_agents::discovery::{
        clear_resolve_cache, discover_acp_runtimes_from, login_shell_spawn_probe,
    };
    use std::fs;
    use tempfile::tempdir;

    // Serialize with every other test that spawns a login shell: the spawn
    // counter and the PATH/login-shell caches are process-global.
    let _path_guard = crate::managed_agents::lock_path_mutex();
    let _registry = registry_test_lock();

    // A custom harness whose command cannot resolve anywhere, so the resolver
    // reaches `find_via_login_shell` under the forced (live) path.
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("absent-harness.json"),
        r#"{
            "id": "absent-harness",
            "label": "Absent Harness",
            "command": "buzz-absent-command-xyzzy",
            "args": []
        }"#,
    )
    .unwrap();

    // Cold cache, cheap path: must spawn ZERO login shells (cache-only resolve
    // reports the absent command missing without probing).
    clear_resolve_cache();
    login_shell_spawn_probe::reset();
    let _ = discover_acp_runtimes_from(Some(dir.path()), false);
    let cold_cheap = login_shell_spawn_probe::count();
    assert_eq!(
        cold_cheap, 0,
        "a cold cheap discovery must not spawn any login shell, got {cold_cheap}"
    );

    // Second cheap discovery, still cold (no forced probe populated the cache):
    // still zero — cache-only resolution never probes.
    login_shell_spawn_probe::reset();
    let _ = discover_acp_runtimes_from(Some(dir.path()), false);
    let second_cheap = login_shell_spawn_probe::count();
    assert_eq!(
        second_cheap, 0,
        "a repeated cheap discovery must not spawn any login shell, got {second_cheap}"
    );

    // Forced path over the SAME absent fixture: resolves live and reaches
    // `find_via_login_shell` at least once. Proves the cheap-path zero above is
    // genuine — the fixture does drive the probe when live resolution runs —
    // not a vacuous zero from a fixture that never reaches it.
    clear_resolve_cache();
    login_shell_spawn_probe::reset();
    let _ = discover_acp_runtimes_from(Some(dir.path()), true);
    let forced = login_shell_spawn_probe::count();
    clear_resolve_cache();
    assert!(
        forced >= 1,
        "the forced path must probe the absent command via login shell at least once, got {forced}"
    );
}
