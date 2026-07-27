use super::*;

#[test]
fn resolves_known_avatar_for_bare_command() {
    let avatar_url = managed_agent_avatar_url("goose").expect("goose avatar should resolve");

    assert_eq!(avatar_url, GOOSE_AVATAR_URL);
}

#[test]
fn resolves_known_avatar_for_command_paths_and_aliases() {
    assert_eq!(
        managed_agent_avatar_url("/usr/local/bin/codex-acp"),
        Some(CODEX_AVATAR_URL.to_string())
    );
    assert_eq!(
        managed_agent_avatar_url("Claude Code"),
        Some(CLAUDE_CODE_AVATAR_URL.to_string())
    );
    assert_eq!(
        managed_agent_avatar_url(r"C:\Tools\claude-agent-acp.exe"),
        Some(CLAUDE_CODE_AVATAR_URL.to_string())
    );
    assert_eq!(
        managed_agent_avatar_url("/usr/local/bin/claude-code-acp"),
        Some(CLAUDE_CODE_AVATAR_URL.to_string())
    );
}

#[test]
fn returns_none_for_unknown_commands() {
    assert!(managed_agent_avatar_url("custom-agent").is_none());
}
