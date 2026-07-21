use super::{
    known_acp_runtime_exact, managed_agent_avatar_url, normalize_agent_args, PI_AVATAR_URL,
};

#[test]
fn resolves_pi_avatar_for_adapter_path() {
    assert_eq!(
        managed_agent_avatar_url("/usr/local/bin/pi-acp"),
        Some(PI_AVATAR_URL.to_string())
    );
}

#[test]
fn normalizes_pi_args_to_empty() {
    assert_eq!(
        normalize_agent_args("pi-acp", Vec::new()),
        Vec::<String>::new()
    );
    assert_eq!(
        normalize_agent_args("pi-acp", vec!["acp".into()]),
        Vec::<String>::new()
    );
}

#[test]
fn pi_runtime_has_install_commands() {
    let pi = known_acp_runtime_exact("pi").unwrap();
    assert!(!pi.cli_install_commands.is_empty());
    assert_eq!(pi.commands, &["pi-acp"]);
    assert_eq!(pi.adapter_install_commands, &["npm install -g pi-acp"]);
}
