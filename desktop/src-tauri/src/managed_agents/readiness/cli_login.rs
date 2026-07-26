use std::path::{Path, PathBuf};

use crate::managed_agents::{
    discovery::{
        classify_runtime, codex_adapter_availability, find_command, resolve_command,
        KnownAcpRuntime,
    },
    AcpAvailabilityStatus,
    resolve_runtime_adapter,
};

use super::{cli_probe, Requirement};

/// Requirements for CLI-login runtimes (claude, codex).
pub(super) fn requirements(
    probe_args: &[&str],
    setup_copy: &str,
    runtime: &KnownAcpRuntime,
) -> Vec<Requirement> {
    let adapter_result = resolve_runtime_adapter(runtime);
    let underlying_cli_found = runtime
        .underlying_cli
        .map(|cli| find_command(cli).is_some())
        .unwrap_or(false);

    let (availability, adapter_command, launch_command, adapter_path) = if runtime.native_acp {
        match adapter_result {
            Some(adapter) => (
                AcpAvailabilityStatus::Available,
                Some(adapter.command),
                Some(adapter.launch_command),
                Some(adapter.path.display().to_string()),
            ),
            None => (AcpAvailabilityStatus::NotInstalled, None, None, None),
        }
    } else {
        let (availability, cmd, path) =
            classify_runtime(adapter_result, runtime.underlying_cli, underlying_cli_found);
        (availability, cmd, None, path)
    };
    let availability = if runtime.id == "codex" && availability == AcpAvailabilityStatus::Available
    {
        adapter_path
            .as_deref()
            .map(|path| codex_adapter_availability(Path::new(path)))
            .unwrap_or(availability)
    } else {
        availability
    };

    match availability {
        AcpAvailabilityStatus::Available => {
            let probe_binary = if runtime.native_acp {
                adapter_path.as_ref().map(PathBuf::from)
            } else {
                resolve_command(probe_args[0])
            };
            let Some(binary_path) = probe_binary else {
                return vec![missing_requirement(
                    probe_args,
                    setup_copy,
                    AcpAvailabilityStatus::Available,
                )];
            };
            let mut effective_probe_args = probe_args.to_vec();
            if runtime.native_acp {
                if let Some(command) = adapter_command {
                    if command == "wsl.exe" {
                        effective_probe_args = vec![
                            "wsl.exe",
                            "--cd",
                            "~",
                            "--",
                            launch_command.unwrap_or("agent"),
                            "status",
                        ];
                    } else {
                        effective_probe_args[0] = command;
                    }
                }
            }
            let augmented_path = cli_probe::augmented_path();
            match cli_probe::login_probe(&binary_path, &effective_probe_args, augmented_path.as_deref()) {
                cli_probe::ProbeOutcome::LoggedIn => vec![],
                cli_probe::ProbeOutcome::LoggedOut => vec![missing_requirement(
                    probe_args,
                    setup_copy,
                    AcpAvailabilityStatus::Available,
                )],
                cli_probe::ProbeOutcome::ConfigInvalid { stderr_excerpt } => {
                    vec![Requirement::CliConfigInvalid {
                        probe_args: probe_args.iter().map(|value| value.to_string()).collect(),
                        setup_copy: setup_copy.to_string(),
                        diagnostic: stderr_excerpt,
                    }]
                }
            }
        }
        other => vec![missing_requirement(probe_args, setup_copy, other)],
    }
}

fn missing_requirement(
    probe_args: &[&str],
    setup_copy: &str,
    availability: AcpAvailabilityStatus,
) -> Requirement {
    Requirement::CliLogin {
        probe_args: probe_args.iter().map(|value| value.to_string()).collect(),
        setup_copy: setup_copy.to_string(),
        availability,
    }
}
