use super::super::{managed_agent_avatar_url, normalize_agent_args, OPENCODE_AVATAR_URL};

#[test]
fn resolves_opencode_avatar() {
    assert_eq!(
        managed_agent_avatar_url("/usr/local/bin/opencode"),
        Some(OPENCODE_AVATAR_URL.to_string())
    );
}

#[test]
fn normalizes_opencode_args_to_acp() {
    assert_eq!(
        normalize_agent_args("opencode", Vec::new()),
        vec!["acp".to_string()]
    );
    assert_eq!(
        normalize_agent_args("open-code", Vec::new()),
        vec!["acp".to_string()]
    );
}
