#[test]
fn goose_uses_system_prompt_only_after_custom_method_succeeds() {
    assert!(!has_system_prompt_support(2, "goose", None));
    assert!(!has_system_prompt_support(2, "goose", Some(false)));
    assert!(has_system_prompt_support(2, "goose", Some(true)));
    assert!(has_system_prompt_support(1, "goose", Some(true)));
    assert!(has_system_prompt_support(2, "buzz-agent", None));
    // Goose never receives system prompt via session/new (uses post-hoc method).
    assert_eq!(
        session_new_system_prompt(true, 2, "goose", Some("instructions")),
        None
    );
    // Protocol-v2 non-goose gets Field transport.
    assert_eq!(
        session_new_system_prompt(false, 2, "buzz-agent", Some("instructions")),
        Some(SystemPromptTransport::Field("instructions"))
    );
    // Protocol-v1 non-goose, non-claude gets None (legacy user-message framing).
    assert_eq!(
        session_new_system_prompt(false, 1, "codex", Some("instructions")),
        None
    );
    // claude-agent-acp gets ClaudeMeta transport regardless of protocol version.
    assert_eq!(
        session_new_system_prompt(false, 1, CLAUDE_AGENT_ACP_NAME, Some("instructions")),
        Some(SystemPromptTransport::ClaudeMeta("instructions"))
    );
    assert_eq!(
        session_new_system_prompt(true, 1, CLAUDE_AGENT_ACP_NAME, Some("instructions")),
        None,
        "goose path must never produce a transport even when agent_name matches"
    );
}

#[test]
fn claude_agent_acp_has_system_prompt_support_regardless_of_protocol_version() {
    // claude-agent-acp declares protocolVersion:1 but supports _meta.systemPrompt;
    // has_system_prompt_support must return true so user-message framing is suppressed.
    assert!(has_system_prompt_support(1, CLAUDE_AGENT_ACP_NAME, None));
    assert!(has_system_prompt_support(2, CLAUDE_AGENT_ACP_NAME, None));
}

#[test]
fn old_zed_adapter_name_falls_through_to_protocol_version_gate() {
    // The renamed @zed-industries package predates the _meta.systemPrompt support,
    // so it must not be treated as capable and stays on legacy user-message framing.
    let old_name = "@zed-industries/claude-code-acp";
    assert!(!has_system_prompt_support(1, old_name, None));
    assert!(has_system_prompt_support(2, old_name, None));
}

#[test]
fn pi_prompt_support_uses_field_regardless_of_protocol_version() {
    for version in [1, 2] {
        assert!(has_system_prompt_support(version, "pi-acp", None));
        assert_eq!(
            session_new_system_prompt(false, version, "pi-acp", Some("instructions")),
            Some(SystemPromptTransport::Field("instructions"))
        );
        assert_eq!(
            session_new_system_prompt(false, version, "pi-acp", None),
            None
        );
    }
}
