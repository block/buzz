use super::*;

fn devin_runtime_for_test(
    commands: &'static [&'static str],
    underlying_cli: Option<&'static str>,
    auth_probe_args: &'static [&'static str],
) -> KnownAcpRuntime {
    KnownAcpRuntime {
        id: "devin",
        label: "Devin",
        default_args: &["acp"],
        login_hint: Some("Run `devin auth login` to authenticate."),
        auth_probe_args: Some(auth_probe_args),
        auth_login_args: Some(&["devin", "auth", "login"]),
        ..make_cli_runtime(commands, underlying_cli)
    }
}

#[test]
fn devin_readiness_is_ready_when_auth_probe_succeeds() {
    let exe = present_binary_str();
    let runtime = devin_runtime_for_test(
        static_commands(vec![exe]),
        Some(exe),
        static_commands(vec![exe, "--list"]),
    );
    let effective = EffectiveAgentEnv {
        env: BTreeMap::new(),
        config_file_path: None,
        effective_command: "devin".to_string(),
    };

    assert!(
        collect_missing_requirements(&effective, Some(&runtime)).is_empty(),
        "a successful catalog-declared Devin auth probe must be ready"
    );
}

#[test]
fn devin_readiness_requires_login_when_auth_probe_fails() {
    let exe = present_binary_str();
    let runtime = devin_runtime_for_test(
        static_commands(vec![exe]),
        Some(exe),
        static_commands(vec![exe, "--buzz-probe-fail-xyz"]),
    );
    let effective = EffectiveAgentEnv {
        env: BTreeMap::new(),
        config_file_path: None,
        effective_command: "devin".to_string(),
    };

    let requirements = collect_missing_requirements(&effective, Some(&runtime));
    assert_eq!(requirements.len(), 1);
    assert!(matches!(
        &requirements[0],
        Requirement::CliLogin {
            availability: AcpAvailabilityStatus::Available,
            setup_copy,
            ..
        } if setup_copy.contains("devin auth login")
    ));
}

#[test]
fn devin_readiness_reports_missing_cli_before_authentication() {
    let missing = "__buzz_nonexistent_devin_xyz789__";
    let runtime = devin_runtime_for_test(
        static_commands(vec![missing]),
        Some(missing),
        static_commands(vec![missing, "auth", "status"]),
    );
    let effective = EffectiveAgentEnv {
        env: BTreeMap::new(),
        config_file_path: None,
        effective_command: "devin".to_string(),
    };

    let requirements = collect_missing_requirements(&effective, Some(&runtime));
    assert_eq!(requirements.len(), 1);
    assert!(matches!(
        requirements[0],
        Requirement::CliLogin {
            availability: AcpAvailabilityStatus::NotInstalled,
            ..
        }
    ));
}
