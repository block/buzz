//! Goose file-config-aware requirement tests.
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
