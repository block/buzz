use super::super::{
    known_acp_runtime_exact, managed_agent_avatar_url, normalize_agent_args, ANTIGRAVITY_AVATAR_URL,
};

#[test]
fn resolves_avatar_for_alias() {
    assert_eq!(
        managed_agent_avatar_url("agy"),
        Some(ANTIGRAVITY_AVATAR_URL.to_string())
    );
}

#[test]
fn runtime_uses_bundled_bridge_and_official_cli() {
    let runtime = known_acp_runtime_exact("antigravity").expect("Antigravity runtime should exist");

    assert_eq!(runtime.commands, &["buzz-acp"]);
    assert_eq!(runtime.underlying_cli, Some("agy"));
    assert_eq!(runtime.skill_dir, None);
    assert_eq!(runtime.model_env_var, Some("BUZZ_AGY_MODEL"));
    assert!(runtime
        .cli_install_commands
        .iter()
        .any(|command| command.contains("antigravity.google/cli/install.sh")));
    assert_eq!(
        normalize_agent_args("buzz-acp", Vec::new()),
        vec!["agy-acp"]
    );
    assert_eq!(
        normalize_agent_args("buzz-acp", vec!["acp".into()]),
        vec!["agy-acp"]
    );
}
