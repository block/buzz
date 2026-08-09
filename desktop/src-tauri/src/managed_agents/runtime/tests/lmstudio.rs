use super::super::{
    apply_runtime_security_env, runtime_inherited_env_keys_to_remove, runtime_metadata_env_vars,
};

#[test]
fn runtime_metadata_env_vars_injects_model_and_provider() {
    let vars = runtime_metadata_env_vars(
        Some("GOOSE_MODEL"),
        Some("GOOSE_PROVIDER"),
        false,
        None,
        Some("gpt-4o"),
        Some("openai"),
    );
    assert_eq!(
        vars,
        vec![("GOOSE_MODEL", "gpt-4o"), ("GOOSE_PROVIDER", "openai")]
    );
}

#[test]
fn runtime_metadata_env_vars_skips_provider_when_locked() {
    let vars = runtime_metadata_env_vars(
        None, // claude has no model_env_var
        None, // claude has no provider_env_var
        true, // provider_locked = true
        Some("anthropic"),
        Some("claude-opus-4-7"),
        Some("anthropic"),
    );
    assert!(vars.is_empty());
}

#[test]
fn runtime_metadata_env_vars_injects_model_even_with_acp_model_switching() {
    // buzz-agent has supports_acp_model_switching=true but we still inject
    // the model env var because ACP model switching is post-bootstrap
    let vars = runtime_metadata_env_vars(
        Some("BUZZ_AGENT_MODEL"),
        Some("BUZZ_AGENT_PROVIDER"),
        false,
        None,
        Some("goose-claude-4-6-opus"),
        Some("databricks"),
    );
    assert_eq!(
        vars,
        vec![
            ("BUZZ_AGENT_MODEL", "goose-claude-4-6-opus"),
            ("BUZZ_AGENT_PROVIDER", "databricks"),
        ]
    );
}

#[test]
fn runtime_metadata_env_vars_force_exact_locked_provider() {
    let vars = runtime_metadata_env_vars(
        Some("LM_STUDIO_MODEL"),
        Some("BUZZ_AGENT_PROVIDER"),
        true,
        Some("lmstudio-native"),
        Some("qwen/qwen3.6-27b"),
        Some("openai"),
    );
    assert_eq!(
        vars,
        vec![
            ("LM_STUDIO_MODEL", "qwen/qwen3.6-27b"),
            ("BUZZ_AGENT_PROVIDER", "lmstudio-native"),
        ]
    );
}

#[test]
fn lmstudio_security_env_is_force_written_after_user_layers() {
    let runtime = crate::managed_agents::known_acp_runtime_exact("buzz-lmstudio-agent")
        .expect("LM Studio runtime");
    let mut env = std::collections::BTreeMap::from([
        (
            "BUZZ_AGENT_CLASSIFICATION".to_string(),
            "PUBLIC".to_string(),
        ),
        ("BUZZ_AGENT_PROVIDER".to_string(), "openai".to_string()),
        (
            "LM_STUDIO_BASE_URL".to_string(),
            "https://attacker.example".to_string(),
        ),
        (
            "LM_STUDIO_MCP_INTEGRATIONS".to_string(),
            r#"[{"type":"plugin","id":"unsafe"}]"#.to_string(),
        ),
        (
            "LM_STUDIO_COMMAND_EVIDENCE_POLICY".to_string(),
            r#"{"services":[{"server_label":"attacker"}]}"#.to_string(),
        ),
        (
            "LM_STUDIO_FALLBACK_PROVIDER".to_string(),
            "openai".to_string(),
        ),
        ("LM_STUDIO_API_TOKEN".to_string(), "plaintext".to_string()),
    ]);

    apply_runtime_security_env(&mut env, Some(runtime));

    assert_eq!(
        env.get("BUZZ_AGENT_CLASSIFICATION").map(String::as_str),
        Some("OFFICIAL")
    );
    assert_eq!(
        env.get("BUZZ_AGENT_PROVIDER").map(String::as_str),
        Some("lmstudio-native")
    );
    assert_eq!(
        env.get("LM_STUDIO_BASE_URL").map(String::as_str),
        Some("http://127.0.0.1:1234")
    );
    assert_eq!(
        env.get("LM_STUDIO_MCP_INTEGRATIONS").map(String::as_str),
        Some("[]")
    );
    assert!(!env.contains_key("LM_STUDIO_COMMAND_EVIDENCE_POLICY"));
    assert!(!env.contains_key("LM_STUDIO_FALLBACK_PROVIDER"));
    assert!(!env.contains_key("LM_STUDIO_API_TOKEN"));
}

#[test]
fn lmstudio_spawn_removes_all_ambient_catalog_owned_keys_before_projection() {
    let runtime = crate::managed_agents::known_acp_runtime_exact("buzz-lmstudio-agent")
        .expect("LM Studio runtime");

    assert_eq!(
        runtime_inherited_env_keys_to_remove(Some(runtime)),
        &[
            "BUZZ_AGENT_CLASSIFICATION",
            "BUZZ_AGENT_PROVIDER",
            "LM_STUDIO_MODEL",
            "LM_STUDIO_BASE_URL",
            "LM_STUDIO_MCP_INTEGRATIONS",
            "LM_STUDIO_COMMAND_EVIDENCE_POLICY",
            "LM_STUDIO_FALLBACK_PROVIDER",
            "LM_STUDIO_API_TOKEN",
        ]
    );
}
