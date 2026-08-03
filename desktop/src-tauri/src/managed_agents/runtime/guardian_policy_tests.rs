#[test]
fn defaults_managed_agents_to_monitor() {
    assert_eq!(
        crate::managed_agents::readiness::GuardianPermissionPolicy::Monitor.as_env_value(),
        "default",
    );
}

#[test]
fn preserves_explicit_lockdown() {
    assert_eq!(
        crate::managed_agents::readiness::GuardianPermissionPolicy::Lockdown.as_env_value(),
        "dont-ask"
    );
}

fn command_env_value(command: &std::process::Command, key: &str) -> Option<String> {
    command.get_envs().find_map(|(candidate, value)| {
        (candidate == key).then(|| value.map(|value| value.to_string_lossy().into_owned()))
    })?
}

#[test]
fn spawn_boundary_injects_monitor_when_policy_is_absent() {
    let mut command = std::process::Command::new("buzz-acp");

    super::apply_guardian_permission_env(
        &mut command,
        crate::managed_agents::readiness::GuardianPermissionPolicy::Monitor,
    );

    assert_eq!(
        command_env_value(&command, "BUZZ_ACP_PERMISSION_MODE").as_deref(),
        Some("default")
    );
}

#[test]
fn spawn_boundary_injects_explicit_lockdown_override() {
    let mut command = std::process::Command::new("buzz-acp");

    super::apply_guardian_permission_env(
        &mut command,
        crate::managed_agents::readiness::GuardianPermissionPolicy::Lockdown,
    );

    assert_eq!(
        command_env_value(&command, "BUZZ_ACP_PERMISSION_MODE").as_deref(),
        Some("dont-ask")
    );
}
