use super::super::{known_acp_runtime_exact, normalize_agent_args, NativeModelDiscovery};

#[test]
fn runtime_declares_native_locked_capabilities() {
    let runtime = known_acp_runtime_exact("buzz-lmstudio-agent")
        .expect("LM Studio runtime must be in the canonical catalog");

    assert_eq!(runtime.commands, &["buzz-lmstudio-agent"]);
    assert_eq!(runtime.mcp_command, None);
    assert!(runtime.supports_acp_model_switching);
    assert_eq!(runtime.model_env_var, Some("LM_STUDIO_MODEL"));
    assert_eq!(runtime.provider_env_var, Some("BUZZ_AGENT_PROVIDER"));
    assert_eq!(runtime.locked_provider_id, Some("lmstudio-native"));
    assert_eq!(runtime.locked_provider_label, Some("LM Studio native"));
    assert_eq!(
        runtime.native_model_discovery,
        Some(NativeModelDiscovery::LmStudioV1)
    );
    assert_eq!(runtime.base_url_env_var, Some("LM_STUDIO_BASE_URL"));
    assert_eq!(
        runtime.classification_env_var,
        Some("BUZZ_AGENT_CLASSIFICATION")
    );
    assert_eq!(
        runtime.integrations_env_var,
        Some("LM_STUDIO_MCP_INTEGRATIONS")
    );
    assert_eq!(runtime.keychain_token_key, Some("lm-studio-api-token"));
    assert_eq!(runtime.required_normalized_fields, &["model"]);
}

#[test]
fn agent_args_are_empty() {
    assert_eq!(
        normalize_agent_args("buzz-lmstudio-agent", vec!["acp".into()]),
        Vec::<String>::new()
    );
}
