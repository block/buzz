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
