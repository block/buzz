//! Per-provider readiness tests for buzz-agent (OpenRouter, Maple).
//!
//! Included from `readiness.rs` via `#[path]`, so `super::*` resolves against
//! that module. Sibling file, like `readiness_goose_file_config_tests.rs`,
//! to keep `readiness.rs` under the file-size limit.

use std::collections::BTreeMap;

use super::*;
use crate::managed_agents::discovery::known_acp_runtime_exact;

/// Build a minimal `EffectiveAgentEnv` with the given env map and command.
fn make_env(command: &str, env: BTreeMap<String, String>) -> EffectiveAgentEnv {
    let runtime = known_acp_runtime_exact(command);
    EffectiveAgentEnv {
        env,
        config_file_path: runtime.and_then(|r| r.config_file_path),
        effective_command: command.to_string(),
    }
}

fn env_with(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ── OpenRouter readiness ─────────────────────────────────────────────────

#[test]
fn buzz_agent_openrouter_with_all_fields_is_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "openrouter"),
            ("BUZZ_AGENT_MODEL", "anthropic/claude-sonnet-4"),
            ("OPENROUTER_API_KEY", "sk-or-test-key"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(
        result.is_ready(),
        "openrouter with all fields should be ready"
    );
}

#[test]
fn buzz_agent_openrouter_missing_key_returns_not_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "openrouter"),
            ("BUZZ_AGENT_MODEL", "anthropic/claude-sonnet-4"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(!result.is_ready());
    assert!(result.requirements().contains(&Requirement::EnvKey {
        key: "OPENROUTER_API_KEY".to_string()
    }));
}

#[test]
fn buzz_agent_openrouter_with_provider_model_fallback_is_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "openrouter"),
            ("OPENROUTER_MODEL", "google/gemini-2.5-flash"),
            ("OPENROUTER_API_KEY", "sk-or-test-key"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(
        result.is_ready(),
        "OPENROUTER_MODEL fallback should satisfy model requirement"
    );
}

// ── Maple readiness ──────────────────────────────────────────────────────

#[test]
fn buzz_agent_maple_with_all_fields_is_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "maple"),
            ("BUZZ_AGENT_MODEL", "llama3-3-70b"),
            ("MAPLE_API_KEY", "maple-test-key"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(result.is_ready(), "maple with all fields should be ready");
}

#[test]
fn buzz_agent_maple_missing_key_returns_not_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "maple"),
            ("BUZZ_AGENT_MODEL", "llama3-3-70b"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(!result.is_ready());
    assert!(result.requirements().contains(&Requirement::EnvKey {
        key: "MAPLE_API_KEY".to_string()
    }));
}

#[test]
fn buzz_agent_maple_with_provider_model_fallback_is_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "maple"),
            ("MAPLE_MODEL", "llama3-3-70b"),
            ("MAPLE_API_KEY", "maple-test-key"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(
        result.is_ready(),
        "MAPLE_MODEL fallback should satisfy model requirement"
    );
}
