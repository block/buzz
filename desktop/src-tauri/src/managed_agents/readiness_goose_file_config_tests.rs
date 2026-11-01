//! Goose file-config and provider-environment readiness tests.
//!
//! These tests call `goose_requirements` directly, injecting a synthetic
//! `RuntimeFileConfig` so there is no disk I/O and tests are deterministic.
//!
//! Included from `readiness.rs` via `#[path]`; `super::*` therefore resolves
//! against that module, matching the `storage_tests.rs` convention.

use std::collections::BTreeMap;

use super::*;
use crate::managed_agents::config_bridge::RuntimeFileConfig;

fn empty_env() -> EffectiveAgentEnv {
    EffectiveAgentEnv {
        env: BTreeMap::new(),
        config_file_path: Some("~/.config/goose/config.yaml"),
        effective_command: "goose".to_string(),
    }
}

fn env_with(pairs: &[(&str, &str)]) -> EffectiveAgentEnv {
    EffectiveAgentEnv {
        env: pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        config_file_path: Some("~/.config/goose/config.yaml"),
        effective_command: "goose".to_string(),
    }
}

fn databricks_file_config() -> RuntimeFileConfig {
    let mut extra = BTreeMap::new();
    extra.insert(
        "DATABRICKS_HOST".to_string(),
        "https://dbc.example.com".to_string(),
    );
    RuntimeFileConfig {
        provider: Some("databricks_v2".to_string()),
        model: Some("goose-claude-4-6-opus".to_string()),
        extra,
        ..Default::default()
    }
}

#[test]
fn goose_file_config_silences_databricks_host_requirement() {
    // File has provider, model, and DATABRICKS_HOST — all requirements silenced.
    let env = empty_env();
    let cfg = databricks_file_config();
    let result = goose_requirements(&env, Some(&cfg));
    assert!(
        result.is_empty(),
        "all requirements should be silenced by goose file config; \
         got: {:?}",
        result
    );
}

#[test]
fn goose_env_empty_file_absent_still_not_ready() {
    // No env, no file config → provider and model both required.
    let env = empty_env();
    let result = goose_requirements(&env, None);
    assert!(
        result.contains(&Requirement::NormalizedField {
            field: "provider".to_string()
        }),
        "provider must be required when absent from both env and file"
    );
    assert!(
        result.contains(&Requirement::NormalizedField {
            field: "model".to_string()
        }),
        "model must be required when absent from both env and file"
    );
}

#[test]
fn goose_file_config_silences_provider_and_model_but_not_anthropic_key() {
    // File has provider=anthropic and model, but ANTHROPIC_API_KEY is not
    // in the file's `extra` map — it must still be required.
    let cfg = RuntimeFileConfig {
        provider: Some("anthropic".to_string()),
        model: Some("claude-opus-4-5".to_string()),
        extra: BTreeMap::new(),
        ..Default::default()
    };
    let env = empty_env();
    let result = goose_requirements(&env, Some(&cfg));
    // Provider and model silenced.
    assert!(
        !result.contains(&Requirement::NormalizedField {
            field: "provider".to_string()
        }),
        "provider silenced by file config"
    );
    assert!(
        !result.contains(&Requirement::NormalizedField {
            field: "model".to_string()
        }),
        "model silenced by file config"
    );
    // ANTHROPIC_API_KEY not in file extra → still required.
    assert!(
        result.contains(&Requirement::EnvKey {
            key: "ANTHROPIC_API_KEY".to_string()
        }),
        "ANTHROPIC_API_KEY must remain required when not in file extra"
    );
}

#[test]
fn goose_env_provider_wins_over_file_provider_for_cred_check() {
    // Env has GOOSE_PROVIDER=anthropic (different from file's databricks_v2).
    // The env provider must win for credential checking.
    let env = env_with(&[
        ("GOOSE_PROVIDER", "anthropic"),
        ("GOOSE_MODEL", "claude-opus-4-5"),
    ]);
    let cfg = databricks_file_config(); // has provider=databricks_v2
    let result = goose_requirements(&env, Some(&cfg));
    // anthropic requires ANTHROPIC_API_KEY, not DATABRICKS_HOST.
    assert!(
        result.contains(&Requirement::EnvKey {
            key: "ANTHROPIC_API_KEY".to_string()
        }),
        "env provider=anthropic must require ANTHROPIC_API_KEY"
    );
    assert!(
        !result.contains(&Requirement::EnvKey {
            key: "DATABRICKS_HOST".to_string()
        }),
        "env provider=anthropic must NOT require DATABRICKS_HOST"
    );
}

#[test]
fn goose_flat_databricks_host_in_file_config_silences_requirement() {
    // Will's typical goose config: flat DATABRICKS_HOST at the top level,
    // no active_provider — provider inferred as "databricks".
    // The parser must store extra["DATABRICKS_HOST"] = value (canonical key),
    // and goose_requirements must then silence the DATABRICKS_HOST requirement.
    let mut extra = BTreeMap::new();
    extra.insert(
        "DATABRICKS_HOST".to_string(),
        "https://block.cloud.databricks.com".to_string(),
    );
    let cfg = RuntimeFileConfig {
        provider: Some("databricks".to_string()),
        model: Some("goose-claude-4-5".to_string()),
        extra,
        ..Default::default()
    };
    let env = empty_env();
    let result = goose_requirements(&env, Some(&cfg));
    // All requirements silenced — provider (file), model (file), DATABRICKS_HOST (file).
    assert!(
        result.is_empty(),
        "flat DATABRICKS_HOST in file config must silence all requirements; \
         got: {:?}",
        result
    );
}

#[test]
fn goose_goose_provider_databricks_flat_host_silences_databricks_host() {
    // GOOSE_PROVIDER=databricks (not active_provider) + flat DATABRICKS_HOST.
    // The parser canonicalizes to extra["DATABRICKS_HOST"]; readiness must silence it.
    let mut extra = BTreeMap::new();
    extra.insert(
        "DATABRICKS_HOST".to_string(),
        "https://dbc.example.com".to_string(),
    );
    let cfg = RuntimeFileConfig {
        provider: Some("databricks".to_string()),
        model: Some("some-model".to_string()),
        extra,
        ..Default::default()
    };
    let env = empty_env();
    let result = goose_requirements(&env, Some(&cfg));
    assert!(
        !result.contains(&Requirement::EnvKey {
            key: "DATABRICKS_HOST".to_string()
        }),
        "DATABRICKS_HOST must be silenced when canonical key is in file extra"
    );
}

#[test]
fn descriptor_maps_authoritative_openai_compatible_config_for_spawn_and_readiness() {
    let record: crate::managed_agents::types::ManagedAgentRecord = serde_json::from_str(
        r#"{
            "pubkey": "test-pubkey",
            "name": "test-agent",
            "private_key_nsec": "",
            "relay_url": "",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "runtime": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "parallelism": 1,
            "system_prompt": null,
            "model": "provider-model",
            "provider": "openai-compat",
            "env_vars": {
                "GOOSE_PROVIDER": "anthropic",
                "OPENAI_COMPAT_API_KEY": "test-key",
                "OPENAI_COMPAT_BASE_URL": "https://provider.example/v1"
            },
            "created_at": "",
            "updated_at": "",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }"#,
    )
    .expect("sample managed agent record");

    let descriptor = resolve_effective_harness_descriptor(&record, &[], &Default::default())
        .expect("descriptor");

    assert_eq!(
        descriptor.env.get("GOOSE_PROVIDER").map(String::as_str),
        Some("openai"),
        "structured effective provider must beat arbitrary layered GOOSE_PROVIDER"
    );
    assert_eq!(
        descriptor.env.get("OPENAI_API_KEY").map(String::as_str),
        Some("test-key")
    );
    assert_eq!(
        descriptor.env.get("OPENAI_HOST").map(String::as_str),
        Some("https://provider.example/v1")
    );
    assert_eq!(
        descriptor
            .env
            .get("GOOSE_PROVIDER__HOST")
            .map(String::as_str),
        Some("https://provider.example/v1")
    );

    let effective = EffectiveAgentEnv {
        env: descriptor.env,
        config_file_path: Some("~/.config/goose/config.yaml"),
        effective_command: descriptor.command,
    };
    assert!(agent_readiness(&effective).is_ready());
}

#[test]
fn descriptor_keeps_legacy_raw_provider_url_records_runnable() {
    let record: crate::managed_agents::types::ManagedAgentRecord = serde_json::from_str(
        r#"{
            "pubkey": "legacy-pubkey",
            "name": "legacy-agent",
            "private_key_nsec": "",
            "relay_url": "",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "runtime": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "parallelism": 1,
            "system_prompt": null,
            "model": "legacy-model",
            "provider": "http://127.0.0.1:9337/v1",
            "env_vars": {"OPENAI_COMPAT_API_KEY": "test-key"},
            "created_at": "",
            "updated_at": "",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }"#,
    )
    .expect("legacy managed agent record");

    let descriptor = resolve_effective_harness_descriptor(&record, &[], &Default::default())
        .expect("descriptor");

    assert_eq!(
        descriptor.env.get("GOOSE_PROVIDER").map(String::as_str),
        Some("openai")
    );
    for key in [
        "OPENAI_COMPAT_BASE_URL",
        "GOOSE_PROVIDER__HOST",
        "OPENAI_HOST",
        "OPENAI_BASE_URL",
    ] {
        assert_eq!(
            descriptor.env.get(key).map(String::as_str),
            Some("http://127.0.0.1:9337/v1")
        );
    }
}
