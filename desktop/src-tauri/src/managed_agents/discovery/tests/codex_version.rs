use super::super::probe_codex_acp_version_with_path;
#[cfg(unix)]
use super::write_executable_script;

#[cfg(unix)]
#[test]
fn probe_codex_acp_version_uses_augmented_path_for_env_shebang_interpreter() {
    use std::fs;
    let temp = tempfile::tempdir().expect("temp dir");
    let script_dir = temp.path().join("script-bin");
    let interpreter_dir = temp.path().join("interpreter-bin");
    let empty_path_dir = temp.path().join("empty-bin");
    fs::create_dir_all(&script_dir).expect("script dir");
    fs::create_dir_all(&interpreter_dir).expect("interpreter dir");
    fs::create_dir_all(&empty_path_dir).expect("empty path dir");

    let interpreter_path = interpreter_dir.join("node");
    write_executable_script(
        &interpreter_path,
        "#!/bin/sh\necho '@agentclientprotocol/codex-acp 1.1.2'\n",
    );

    let shim_path = script_dir.join("codex-acp");
    write_executable_script(&shim_path, "#!/usr/bin/env node\n");

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

    for version in ["1.1.5", "1.1.7", "1.6.2", "1.9.0"] {
        let dir = tempfile::tempdir().expect("temp dir");
        let bin = dir.path().join("codex-acp");
        write_executable_script(
            &bin,
            format!("#!/bin/sh\necho '@agentclientprotocol/codex-acp {version}'\nexit 0\n"),
        );

        assert_eq!(
            codex_adapter_availability(&bin),
            AcpAvailabilityStatus::AdapterOutdated,
            "adapter {version} must be offered an upgrade"
        );
        assert!(codex_adapter_is_outdated(&bin));
    }
}

#[cfg(unix)]
#[test]
fn codex_adapter_availability_available_for_supported_versions() {
    use super::super::codex_adapter_availability;
    use crate::managed_agents::AcpAvailabilityStatus;

    // 1.10.0 is the supported floor; the whole 2.x line is validated.
    for version in ["1.10.0", "1.11.2", "2.0.0", "2.2.2", "2.3.0"] {
        let dir = tempfile::tempdir().expect("temp dir");
        let bin = dir.path().join("codex-acp");
        write_executable_script(
            &bin,
            format!("#!/bin/sh\necho '@agentclientprotocol/codex-acp {version}'\nexit 0\n"),
        );

        assert_eq!(
            codex_adapter_availability(&bin),
            AcpAvailabilityStatus::Available,
            "adapter {version} is inside the supported window"
        );
    }
}

#[cfg(unix)]
#[test]
fn codex_adapter_availability_outdated_for_unvalidated_major() {
    use super::super::codex_adapter_availability;
    use crate::managed_agents::AcpAvailabilityStatus;

    // A major beyond the validated line may change the ACP contract again
    // (2.0 already did). The app must not run it unreviewed: it offers the
    // supported line instead, rather than leaving managed agents unable to
    // start a session.
    for version in ["3.0.0", "3.4.2", "4.0.0"] {
        let dir = tempfile::tempdir().expect("temp dir");
        let bin = dir.path().join("codex-acp");
        write_executable_script(
            &bin,
            format!("#!/bin/sh\necho '@agentclientprotocol/codex-acp {version}'\nexit 0\n"),
        );

        assert_eq!(
            codex_adapter_availability(&bin),
            AcpAvailabilityStatus::AdapterOutdated,
            "adapter {version} is beyond the validated major line"
        );
    }
}
