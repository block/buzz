use std::collections::BTreeMap;
use std::path::Path;

use crate::managed_agents::{
    discovery::{
        classify_runtime, codex_adapter_availability, find_command, resolve_command,
        KnownAcpRuntime,
    },
    AcpAvailabilityStatus,
};

use super::{cli_probe, Requirement};

/// Requirements for CLI-login runtimes (claude, codex).
#[cfg(test)]
pub(super) fn requirements(
    probe_args: &[&str],
    setup_copy: &str,
    runtime: &KnownAcpRuntime,
) -> Vec<Requirement> {
    requirements_with_env_auth(probe_args, setup_copy, runtime, false)
}

pub(super) fn requirements_with_effective_env(
    probe_args: &[&str],
    setup_copy: &str,
    runtime: &KnownAcpRuntime,
    effective_env: &BTreeMap<String, String>,
) -> Vec<Requirement> {
    let env_auth_satisfied =
        crate::managed_agents::discovery::runtime_auth::auth_evidence_satisfied(
            runtime.auth_evidence,
            std::slice::from_ref(effective_env),
        );
    requirements_with_env_auth(probe_args, setup_copy, runtime, env_auth_satisfied)
}

pub(super) fn requirements_for_runtime(
    runtime: &KnownAcpRuntime,
    effective_env: &BTreeMap<String, String>,
) -> Vec<Requirement> {
    match runtime.id {
        "claude" => requirements_with_effective_env(
            &["claude", "auth", "status"],
            "complete Claude Code authentication by running the Claude CLI",
            runtime,
            effective_env,
        ),
        "codex" => requirements_with_effective_env(
            &["codex", "login", "status"],
            "run `codex login`",
            runtime,
            effective_env,
        ),
        _ => vec![],
    }
}

/// Requirements for a CLI-login runtime that may also be authenticated by an
/// environment credential outside the CLI's persisted login store.
pub(super) fn requirements_with_env_auth(
    probe_args: &[&str],
    setup_copy: &str,
    runtime: &KnownAcpRuntime,
    env_auth_satisfied: bool,
) -> Vec<Requirement> {
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

    match availability {
        AcpAvailabilityStatus::Available => {
            if env_auth_satisfied {
                return vec![];
            }
            let Some(binary_path) = resolve_command(probe_args[0]) else {
                return vec![missing_requirement(
                    probe_args,
                    setup_copy,
                    AcpAvailabilityStatus::Available,
                )];
            };
            let augmented_path = cli_probe::augmented_path();
            match cli_probe::login_probe(&binary_path, probe_args, augmented_path.as_deref()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn present_binary_str() -> &'static str {
        Box::leak(
            std::env::current_exe()
                .expect("test executable path")
                .to_string_lossy()
                .into_owned()
                .into_boxed_str(),
        )
    }

    fn static_commands(commands: Vec<&'static str>) -> &'static [&'static str] {
        Box::leak(commands.into_boxed_slice())
    }

    fn make_runtime(
        commands: &'static [&'static str],
        underlying_cli: Option<&'static str>,
    ) -> KnownAcpRuntime {
        KnownAcpRuntime {
            id: "test-cli-runtime",
            label: "Test CLI",
            commands,
            aliases: &[],
            avatar_url: "",
            mcp_command: None,
            mcp_hooks: false,
            underlying_cli,
            cli_install_commands: &[],
            cli_install_commands_windows: &[],
            adapter_install_commands: &[],
            cli_install_instructions_url: "",
            adapter_install_instructions_url: "",
            cli_install_hint: "",
            adapter_install_hint: "",
            skill_dir: None,
            supports_acp_model_switching: false,
            model_env_var: None,
            provider_env_var: None,
            provider_locked: false,
            default_env: &[],
            config_file_path: None,
            config_file_format: None,
            supports_acp_native_config: false,
            thinking_env_var: None,
            max_tokens_env_var: None,
            context_limit_env_var: None,
            max_rounds_env_var: None,
            required_normalized_fields: &[],
            login_hint: None,
            auth_probe_args: None,
            auth_evidence: crate::managed_agents::AuthEvidenceStrategy::None,
        }
    }

    #[test]
    fn satisfied_env_auth_bypasses_login_probe() {
        let exe = present_binary_str();
        let runtime = make_runtime(static_commands(vec![exe]), Some(exe));
        let requirements = requirements_with_env_auth(
            &[exe, "--buzz-probe-must-not-run"],
            "this should not show",
            &runtime,
            true,
        );

        assert!(requirements.is_empty());
    }

    #[test]
    fn static_runtime_env_auth_bypasses_login_probe() {
        let exe = present_binary_str();
        let mut runtime = make_runtime(static_commands(vec![exe]), Some(exe));
        runtime.auth_evidence =
            crate::managed_agents::AuthEvidenceStrategy::StaticEnvKeys(&["TOKEN"]);
        let env = BTreeMap::from([("TOKEN".to_string(), "secret".to_string())]);

        let requirements = requirements_with_effective_env(
            &[exe, "--buzz-probe-must-not-run"],
            "this should not show",
            &runtime,
            &env,
        );

        assert!(requirements.is_empty());
    }

    #[test]
    fn env_auth_does_not_hide_missing_tooling() {
        let runtime = make_runtime(
            &["__buzz_nonexistent_adapter_env_auth__"],
            Some("__buzz_nonexistent_cli_env_auth__"),
        );
        let requirements = requirements_with_env_auth(
            &["__buzz_nonexistent_cli_env_auth__", "status"],
            "install the tool",
            &runtime,
            true,
        );

        assert!(matches!(
            requirements.as_slice(),
            [Requirement::CliLogin {
                availability: AcpAvailabilityStatus::NotInstalled,
                ..
            }]
        ));
    }
}
