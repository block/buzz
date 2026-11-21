/// The claude runtime must declare a model env channel — without one, the
/// model picked in the UI is persisted but never applied at spawn (#2692).
#[test]
fn claude_runtime_declares_anthropic_model_env_var() {
    let claude = super::super::known_acp_runtime_exact("claude").expect("claude runtime registered");
    assert_eq!(claude.model_env_var, Some("ANTHROPIC_MODEL"));
    assert!(claude.provider_locked);
    assert_eq!(claude.provider_env_var, None);
}
