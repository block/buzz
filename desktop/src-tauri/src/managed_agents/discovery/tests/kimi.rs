use super::super::{managed_agent_avatar_url, normalize_agent_args, KIMI_CODE_AVATAR_URL};

#[test]
fn resolves_kimi_avatar() {
    assert_eq!(
        managed_agent_avatar_url("/usr/local/bin/kimi"),
        Some(KIMI_CODE_AVATAR_URL.to_string())
    );
}

#[test]
fn normalizes_kimi_args_to_acp() {
    assert_eq!(
        normalize_agent_args("kimi", Vec::new()),
        vec!["acp".to_string()]
    );
    assert_eq!(
        normalize_agent_args("kimi-code", Vec::new()),
        vec!["acp".to_string()]
    );
}
