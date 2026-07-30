use super::super::probe_codex_acp_version_with_path;

#[cfg(unix)]
#[test]
fn probe_codex_acp_version_parses_full_semver_output() {
    use super::super::probe_codex_acp_version;
    use std::os::unix::fs::PermissionsExt;

    // Simulate a current `@agentclientprotocol/codex-acp` output.
    let dir = std::env::temp_dir().join(format!("buzz-probe-1x-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let bin = dir.join("codex-acp");
    std::fs::write(
        &bin,
        "#!/bin/sh\necho '@agentclientprotocol/codex-acp 1.1.7'\nexit 0\n",
    )
    .expect("write script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod script");

    let version = probe_codex_acp_version(&bin);
    let _ = std::fs::remove_dir_all(dir);

    assert_eq!(
        version,
        Some((1, 1, 7)),
        "adapter output must parse to its full semantic version"
    );
}

#[cfg(unix)]
#[test]
fn probe_codex_acp_version_uses_augmented_path_for_env_shebang_interpreter() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().expect("temp dir");
    let script_dir = temp.path().join("script-bin");
    let interpreter_dir = temp.path().join("interpreter-bin");
    let empty_path_dir = temp.path().join("empty-bin");
    fs::create_dir_all(&script_dir).expect("script dir");
    fs::create_dir_all(&interpreter_dir).expect("interpreter dir");
    fs::create_dir_all(&empty_path_dir).expect("empty path dir");

    let interpreter_path = interpreter_dir.join("node");
    fs::write(
        &interpreter_path,
        "#!/bin/sh\necho '@agentclientprotocol/codex-acp 1.1.2'\n",
    )
    .expect("write interpreter");
    fs::set_permissions(&interpreter_path, fs::Permissions::from_mode(0o755))
        .expect("chmod interpreter");

    let shim_path = script_dir.join("codex-acp");
    fs::write(&shim_path, "#!/usr/bin/env node\n").expect("write shim");
    fs::set_permissions(&shim_path, fs::Permissions::from_mode(0o755)).expect("chmod shim");

    let scrubbed_path = std::env::join_paths([empty_path_dir.as_path()])
        .expect("join scrubbed PATH")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        probe_codex_acp_version_with_path(&shim_path, Some(&scrubbed_path)),
        None,
        "with a scrubbed PATH, /usr/bin/env should not find node"
    );

    let augmented_path = std::env::join_paths([interpreter_dir.as_path()])
        .expect("join augmented PATH")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        probe_codex_acp_version_with_path(&shim_path, Some(&augmented_path)),
        Some((1, 1, 2)),
        "the injected augmented PATH should allow /usr/bin/env to find node"
    );
}
