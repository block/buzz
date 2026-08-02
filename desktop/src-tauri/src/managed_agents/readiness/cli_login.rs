use std::path::Path;

use crate::managed_agents::{
    discovery::{
        availability_with_cached_compatibility, classify_runtime, codex_adapter_availability,
        find_command, resolve_command, KnownAcpRuntime, RuntimeAuthProbe,
    },
    AcpAvailabilityStatus,
};

use super::{cli_probe, Requirement};

#[cfg(test)]
pub(super) fn test_probe(args: &[&'static str]) -> RuntimeAuthProbe {
    RuntimeAuthProbe {
        args: Box::leak(args.to_vec().into_boxed_slice()),
        usable_exit_codes: &[0],
    }
}

/// Requirements for CLI-login runtimes (claude, codex).
pub(super) fn requirements(
    probe: RuntimeAuthProbe,
    setup_copy: &str,
    runtime: &KnownAcpRuntime,
) -> Vec<Requirement> {
    let probe_args = probe.args;
    let adapter_result = runtime
        .commands
        .iter()
        .find_map(|cmd| find_command(cmd).map(|path| (*cmd, path)));
    let underlying_cli_found = runtime
        .underlying_cli
        .map(|cli| find_command(cli).is_some())
        .unwrap_or(false);

    let (availability, _cmd, adapter_path) =
        classify_runtime(adapter_result, runtime.underlying_cli, underlying_cli_found);
    let availability = if runtime.id == "codex" && availability == AcpAvailabilityStatus::Available
    {
        adapter_path
            .as_deref()
            .map(|path| codex_adapter_availability(Path::new(path)))
            .unwrap_or(availability)
    } else {
        availability
    };
    let availability = availability_with_cached_compatibility(runtime, availability);

    match availability {
        AcpAvailabilityStatus::Available => {
            let Some(binary_path) = resolve_command(probe_args[0]) else {
                return vec![missing_requirement(
                    probe_args,
                    setup_copy,
                    AcpAvailabilityStatus::Available,
                )];
            };
            let augmented_path = cli_probe::augmented_path();
            match cli_probe::login_probe(
                &binary_path,
                probe_args,
                probe.usable_exit_codes,
                augmented_path.as_deref(),
            ) {
                cli_probe::ProbeOutcome::LoggedIn => vec![],
                // Authentication readiness is advisory. Operational probe
                // failures must not masquerade as invalid credentials.
                cli_probe::ProbeOutcome::Unknown => vec![],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_setup_copy_does_not_mention_openai_api_key() {
        let requirement = missing_requirement(
            &["codex", "login", "status"],
            "run `codex login`",
            AcpAvailabilityStatus::Available,
        );
        let Requirement::CliLogin { setup_copy, .. } = requirement else {
            panic!("expected CLI login requirement");
        };
        assert!(!setup_copy.contains("OPENAI_API_KEY"));
        assert!(setup_copy.contains("codex login"));
    }
}
