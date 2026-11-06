use std::collections::BTreeMap;

use super::{
    apply_runtime_provider_env_mapping, merge_runtime_provider_env_layers,
    provider_is_http_base_url, validate_openai_compat_base_url,
};

fn goose() -> &'static crate::managed_agents::KnownAcpRuntime {
    crate::managed_agents::discovery::known_acp_runtime_exact("goose").expect("goose runtime")
}

#[test]
fn goose_openai_compatible_mapping_uses_runtime_native_names() {
    let mut env = BTreeMap::from([
        (
            "OPENAI_COMPAT_API_KEY".to_string(),
            "canonical-key".to_string(),
        ),
        (
            "OPENAI_COMPAT_BASE_URL".to_string(),
            "http://localhost:1234/v1".to_string(),
        ),
    ]);

    apply_runtime_provider_env_mapping(goose(), Some("openai-compat"), &mut env);

    assert_eq!(
        env.get("GOOSE_PROVIDER").map(String::as_str),
        Some("openai")
    );
    assert_eq!(
        env.get("OPENAI_API_KEY").map(String::as_str),
        Some("canonical-key")
    );
    assert_eq!(
        env.get("GOOSE_PROVIDER__API_KEY").map(String::as_str),
        Some("canonical-key")
    );
    for key in [
        "OPENAI_COMPAT_BASE_URL",
        "GOOSE_PROVIDER__HOST",
        "OPENAI_HOST",
        "OPENAI_BASE_URL",
    ] {
        assert_eq!(
            env.get(key).map(String::as_str),
            Some("http://localhost:1234/v1")
        );
    }
}

#[test]
fn ordinary_openai_clears_inherited_compatibility_hosts() {
    let mut env = BTreeMap::from([
        (
            "OPENAI_COMPAT_API_KEY".to_string(),
            "canonical-key".to_string(),
        ),
        (
            "OPENAI_COMPAT_BASE_URL".to_string(),
            "http://stale.example/v1".to_string(),
        ),
        (
            "GOOSE_PROVIDER__HOST".to_string(),
            "http://stale.example/v1".to_string(),
        ),
        (
            "OPENAI_HOST".to_string(),
            "http://stale.example/v1".to_string(),
        ),
        (
            "OPENAI_BASE_URL".to_string(),
            "http://stale.example/v1".to_string(),
        ),
    ]);

    apply_runtime_provider_env_mapping(goose(), Some("openai"), &mut env);

    assert_eq!(
        env.get("GOOSE_PROVIDER").map(String::as_str),
        Some("openai")
    );
    assert_eq!(
        env.get("OPENAI_API_KEY").map(String::as_str),
        Some("canonical-key")
    );
    for key in [
        "OPENAI_COMPAT_BASE_URL",
        "GOOSE_PROVIDER__HOST",
        "OPENAI_HOST",
        "OPENAI_BASE_URL",
    ] {
        assert!(!env.contains_key(key), "{key} should be cleared");
    }
}

#[test]
fn openai_compatible_mapping_trims_base_url_aliases() {
    let mut env = BTreeMap::from([(
        "OPENAI_COMPAT_BASE_URL".to_string(),
        "  http://localhost:1234/v1  ".to_string(),
    )]);

    apply_runtime_provider_env_mapping(goose(), Some("openai-compat"), &mut env);

    for key in [
        "OPENAI_COMPAT_BASE_URL",
        "GOOSE_PROVIDER__HOST",
        "OPENAI_HOST",
        "OPENAI_BASE_URL",
    ] {
        assert_eq!(
            env.get(key).map(String::as_str),
            Some("http://localhost:1234/v1")
        );
    }
}

#[test]
fn effective_provider_overrides_arbitrary_layered_goose_provider() {
    let mut env = BTreeMap::new();
    let agent = BTreeMap::from([("GOOSE_PROVIDER".to_string(), "anthropic".to_string())]);

    merge_runtime_provider_env_layers(goose(), Some("openai-compat"), &mut env, [agent]);

    assert_eq!(
        env.get("GOOSE_PROVIDER").map(String::as_str),
        Some("openai")
    );
}

#[test]
fn higher_canonical_layer_overrides_lower_native_layer() {
    let mut env = BTreeMap::new();
    let global = BTreeMap::from([
        ("OPENAI_API_KEY".to_string(), "global-key".to_string()),
        (
            "OPENAI_BASE_URL".to_string(),
            "http://global.example/v1".to_string(),
        ),
    ]);
    let agent = BTreeMap::from([
        ("OPENAI_COMPAT_API_KEY".to_string(), "agent-key".to_string()),
        (
            "OPENAI_COMPAT_BASE_URL".to_string(),
            "http://agent.example/v1".to_string(),
        ),
    ]);

    merge_runtime_provider_env_layers(goose(), Some("openai-compat"), &mut env, [global, agent]);

    assert_eq!(
        env.get("OPENAI_API_KEY").map(String::as_str),
        Some("agent-key")
    );
    assert_eq!(
        env.get("OPENAI_COMPAT_API_KEY").map(String::as_str),
        Some("agent-key")
    );
    assert_eq!(
        env.get("OPENAI_BASE_URL").map(String::as_str),
        Some("http://agent.example/v1")
    );
}

#[test]
fn padded_provider_preserves_alias_aware_layer_precedence() {
    let mut env = BTreeMap::new();
    let global = BTreeMap::from([("OPENAI_API_KEY".to_string(), "global-key".to_string())]);
    let agent = BTreeMap::from([("OPENAI_COMPAT_API_KEY".to_string(), "agent-key".to_string())]);

    merge_runtime_provider_env_layers(goose(), Some(" openai-compat "), &mut env, [global, agent]);

    assert_eq!(
        env.get("OPENAI_API_KEY").map(String::as_str),
        Some("agent-key")
    );
}

#[test]
fn legacy_raw_provider_url_maps_to_openai_and_overrides_stale_base_url() {
    let mut env = BTreeMap::from([
        (
            "OPENAI_COMPAT_BASE_URL".to_string(),
            "http://stale.example/v1".to_string(),
        ),
        (
            "OPENAI_COMPAT_API_KEY".to_string(),
            "canonical-key".to_string(),
        ),
    ]);

    apply_runtime_provider_env_mapping(goose(), Some("http://selected.example/v1"), &mut env);

    assert_eq!(
        env.get("GOOSE_PROVIDER").map(String::as_str),
        Some("openai")
    );
    for key in [
        "OPENAI_COMPAT_BASE_URL",
        "GOOSE_PROVIDER__HOST",
        "OPENAI_HOST",
        "OPENAI_BASE_URL",
    ] {
        assert_eq!(
            env.get(key).map(String::as_str),
            Some("http://selected.example/v1")
        );
    }
    assert_eq!(
        env.get("OPENAI_API_KEY").map(String::as_str),
        Some("canonical-key")
    );
}

#[test]
fn provider_urls_reject_publishable_secret_material() {
    for invalid in [
        "https://user:password@example.com/v1",
        "https://example.com/v1?api_key=secret",
        "https://example.com/v1#api-key",
        "file:///tmp/provider",
    ] {
        assert!(
            validate_openai_compat_base_url(invalid).is_err(),
            "{invalid} must be rejected"
        );
        assert!(!provider_is_http_base_url(invalid));
    }
    assert!(provider_is_http_base_url("https://example.com/v1"));
}
