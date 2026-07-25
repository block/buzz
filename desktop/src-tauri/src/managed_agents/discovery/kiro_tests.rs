use super::{managed_agent_avatar_url, normalize_agent_args};
use crate::managed_agents::KIRO_AVATAR_URL;

#[test]
fn normalizes_kiro_cli_args_to_acp() {
    assert_eq!(normalize_agent_args("kiro-cli", Vec::new()), vec!["acp"]);
    assert_eq!(
        normalize_agent_args("/usr/local/bin/kiro-cli", vec!["".into()]),
        vec!["acp"]
    );
    assert_eq!(
        normalize_agent_args("Kiro_CLI.EXE", Vec::new()),
        vec!["acp"]
    );
}

#[test]
fn resolves_kiro_avatar() {
    assert_eq!(
        managed_agent_avatar_url("kiro-cli"),
        Some(KIRO_AVATAR_URL.to_string())
    );
}
