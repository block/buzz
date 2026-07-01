use std::collections::BTreeMap;

use super::{strip_empty_env_vars, validate_global_config, GlobalAgentConfig};

fn config_with_env(pairs: &[(&str, &str)]) -> GlobalAgentConfig {
    GlobalAgentConfig {
        env_vars: pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        ..Default::default()
    }
}

// ── validate_global_config ────────────────────────────────────────────────────

#[test]
fn validate_accepts_valid_env_vars() {
    let config = config_with_env(&[("ANTHROPIC_API_KEY", "sk-test"), ("MY_CUSTOM_KEY", "value")]);
    assert!(validate_global_config(&config).is_ok());
}

#[test]
fn validate_rejects_reserved_key() {
    let config = config_with_env(&[("BUZZ_PRIVATE_KEY", "should-not-be-settable")]);
    let err = validate_global_config(&config).unwrap_err();
    assert!(
        err.contains("reserved"),
        "expected reserved-key error, got: {err}"
    );
}

#[test]
fn validate_rejects_derived_provider_model_key_goose_provider() {
    let config = config_with_env(&[("GOOSE_PROVIDER", "anthropic")]);
    let err = validate_global_config(&config).unwrap_err();
    assert!(
        err.contains("structured provider/model fields"),
        "expected derived-key error, got: {err}"
    );
}

#[test]
fn validate_rejects_derived_key_goose_model() {
    let config = config_with_env(&[("GOOSE_MODEL", "claude-opus-4")]);
    let err = validate_global_config(&config).unwrap_err();
    assert!(
        err.contains("structured provider/model fields"),
        "got: {err}"
    );
}

#[test]
fn validate_rejects_derived_key_buzz_agent_provider() {
    let config = config_with_env(&[("BUZZ_AGENT_PROVIDER", "anthropic")]);
    let err = validate_global_config(&config).unwrap_err();
    assert!(
        err.contains("structured provider/model fields"),
        "got: {err}"
    );
}

#[test]
fn validate_rejects_malformed_key() {
    let config = config_with_env(&[("has spaces", "val")]);
    let err = validate_global_config(&config).unwrap_err();
    assert!(
        err.contains("must match"),
        "expected malformed-key error, got: {err}"
    );
}

#[test]
fn validate_ignores_empty_values_for_reserved_key_check() {
    // A reserved key with an EMPTY value is a no-op (stripped at save time).
    // validate_global_config skips empty-value entries so it does not reject
    // an empty clear for a key that happens to share a name with a reserved key.
    let config = config_with_env(&[("BUZZ_PRIVATE_KEY", "")]);
    // Strip is done inside validate — empty values are stripped before checking.
    assert!(
        validate_global_config(&config).is_ok(),
        "empty value for reserved key should be treated as unset"
    );
}

// ── strip_empty_env_vars ──────────────────────────────────────────────────────

#[test]
fn strip_removes_empty_values_only() {
    let mut config = config_with_env(&[("KEY_A", "value"), ("KEY_B", ""), ("KEY_C", "other")]);
    strip_empty_env_vars(&mut config);
    assert_eq!(config.env_vars.len(), 2);
    assert!(config.env_vars.contains_key("KEY_A"));
    assert!(
        !config.env_vars.contains_key("KEY_B"),
        "empty value must be stripped"
    );
    assert!(config.env_vars.contains_key("KEY_C"));
}

#[test]
fn strip_is_idempotent_on_all_non_empty() {
    let mut config = config_with_env(&[("KEY_A", "v1"), ("KEY_B", "v2")]);
    let original = config.env_vars.clone();
    strip_empty_env_vars(&mut config);
    assert_eq!(config.env_vars, original);
}

// ── GlobalAgentConfig defaults ────────────────────────────────────────────────

#[test]
fn default_config_is_all_none_empty() {
    let config = GlobalAgentConfig::default();
    assert!(config.env_vars.is_empty());
    assert!(config.provider.is_none());
    assert!(config.model.is_none());
}

#[test]
fn roundtrip_serialization() {
    let config = GlobalAgentConfig {
        env_vars: BTreeMap::from([("ANTHROPIC_API_KEY".to_string(), "sk-test".to_string())]),
        provider: Some("anthropic".to_string()),
        model: Some("claude-opus-4".to_string()),
    };
    let json = serde_json::to_string(&config).expect("serialize");
    let back: GlobalAgentConfig = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(config, back);
}

#[test]
fn empty_env_vars_omitted_from_serialization() {
    let config = GlobalAgentConfig::default();
    let json = serde_json::to_string(&config).expect("serialize");
    // With all-default/empty, the JSON should be compact.
    assert!(!json.contains("env_vars"), "empty env_vars must be omitted");
    assert!(!json.contains("provider"), "None provider must be omitted");
    assert!(!json.contains("model"), "None model must be omitted");
}
