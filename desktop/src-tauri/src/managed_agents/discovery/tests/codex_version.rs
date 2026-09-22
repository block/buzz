use super::super::probe_codex_acp_version_with_path;

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

#[cfg(unix)]
#[test]
fn codex_adapter_availability_outdated_for_older_1x_binary() {
    use super::super::{codex_adapter_availability, codex_adapter_is_outdated};
    use crate::managed_agents::AcpAvailabilityStatus;
    use std::os::unix::fs::PermissionsExt;

    for version in ["1.1.5", "1.1.7", "1.6.2", "1.9.0"] {
        let dir = tempfile::tempdir().expect("temp dir");
        let bin = dir.path().join("codex-acp");
        std::fs::write(
            &bin,
            format!("#!/bin/sh\necho '@agentclientprotocol/codex-acp {version}'\nexit 0\n"),
        )
        .expect("write script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");

        assert_eq!(
            codex_adapter_availability(&bin),
            AcpAvailabilityStatus::AdapterOutdated,
            "adapter {version} must be offered an upgrade"
        );
        assert!(codex_adapter_is_outdated(&bin));
    }
}
