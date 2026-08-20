use super::*;

fn make_env(command: &str, pairs: &[(&str, &str)]) -> EffectiveAgentEnv {
    EffectiveAgentEnv {
        env: pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
        config_file_path: None,
        effective_command: command.to_string(),
    }
}

#[test]
fn provider_env_canonicalization_isolates_consumers() {
    let shared = BTreeMap::from([
        ("OPENAI_API_KEY".into(), "official".into()),
        ("OPENAI_COMPAT_API_KEY".into(), "compat".into()),
        (
            "OPENAI_COMPAT_BASE_URL".into(),
            "http://localhost:11434/v1".into(),
        ),
    ]);
    let mut official = shared.clone();
    crate::managed_agents::openai_env::canonicalize_openai_provider_env(
        &mut official,
        Some("openai"),
    );
    assert_eq!(
        official.get("OPENAI_API_KEY").map(String::as_str),
        Some("official")
    );
    assert!(!official.contains_key("OPENAI_COMPAT_API_KEY"));
    assert!(!official.contains_key("OPENAI_COMPAT_BASE_URL"));

    let mut compatible = shared;
    crate::managed_agents::openai_env::canonicalize_openai_provider_env(
        &mut compatible,
        Some("openai-compat"),
    );
    assert!(!compatible.contains_key("OPENAI_API_KEY"));
    assert_eq!(
        compatible.get("OPENAI_COMPAT_API_KEY").map(String::as_str),
        Some("compat")
    );
}

#[test]
fn migrated_identity_overrides_legacy_provider_without_secret_inference() {
    let mut env = BTreeMap::from([
        ("BUZZ_AGENT_PROVIDER".into(), "openai".into()),
        (
            crate::managed_agents::openai_env::MIGRATED_OPENAI_PROVIDER_ENV.into(),
            "openai-compat".into(),
        ),
        ("OPENAI_COMPAT_API_KEY".into(), "legacy".into()),
        (
            "OPENAI_COMPAT_BASE_URL".into(),
            "http://localhost:11434/v1".into(),
        ),
    ]);
    assert_eq!(
        crate::managed_agents::openai_env::canonicalize_openai_provider_env(
            &mut env,
            Some("openai")
        )
        .as_deref(),
        Some("openai-compat")
    );
    assert_eq!(
        env.get("BUZZ_AGENT_PROVIDER").map(String::as_str),
        Some("openai-compat")
    );
}

#[test]
fn equal_credentials_do_not_override_explicit_official_provider() {
    let mut env = BTreeMap::from([
        ("BUZZ_AGENT_PROVIDER".into(), "openai".into()),
        ("OPENAI_API_KEY".into(), "same-secret".into()),
        ("OPENAI_COMPAT_API_KEY".into(), "same-secret".into()),
        (
            "OPENAI_COMPAT_BASE_URL".into(),
            "https://gateway.example/v1".into(),
        ),
    ]);
    assert_eq!(
        crate::managed_agents::openai_env::canonicalize_openai_provider_env(
            &mut env,
            Some("openai")
        )
        .as_deref(),
        Some("openai")
    );
    assert!(!env.contains_key("OPENAI_COMPAT_API_KEY"));
    assert!(!env.contains_key("OPENAI_COMPAT_BASE_URL"));
}

#[test]
fn non_buzz_harness_keeps_its_configured_provider() {
    let env = BTreeMap::from([("BUZZ_AGENT_PROVIDER".into(), "openai-compat".into())]);
    assert_eq!(
        crate::managed_agents::openai_env::effective_runtime_provider(
            "goose",
            Some("openai".into()),
            &env,
        )
        .as_deref(),
        Some("openai")
    );
}

#[test]
fn keyless_compatible_provider_gets_explicit_blank_credential() {
    let mut env = BTreeMap::from([(
        "OPENAI_COMPAT_BASE_URL".into(),
        "http://localhost:11434/v1".into(),
    )]);
    crate::managed_agents::openai_env::canonicalize_openai_provider_env(
        &mut env,
        Some("openai-compat"),
    );
    assert_eq!(
        env.get("OPENAI_COMPAT_API_KEY").map(String::as_str),
        Some("")
    );
}

#[test]
fn readiness_enforces_openai_provider_boundaries() {
    let official = make_env(
        "buzz-agent",
        &[
            ("BUZZ_AGENT_PROVIDER", "openai"),
            ("BUZZ_AGENT_MODEL", "gpt-4o"),
            ("OPENAI_COMPAT_API_KEY", "must-not-cross-provider-boundary"),
        ],
    );
    assert!(agent_readiness(&official)
        .requirements()
        .contains(&Requirement::EnvKey {
            key: "OPENAI_API_KEY".to_string(),
        }));

    let keyless_compat = make_env(
        "buzz-agent",
        &[
            ("BUZZ_AGENT_PROVIDER", "OpenAI-Compat"),
            ("BUZZ_AGENT_MODEL", "llama3"),
            ("OPENAI_COMPAT_BASE_URL", "http://localhost:11434/v1"),
        ],
    );
    assert!(agent_readiness(&keyless_compat).is_ready());
}
