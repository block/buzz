//! Provider-specific model fallback readiness tests.

use std::collections::BTreeMap;

use super::*;
use crate::managed_agents::discovery::known_acp_runtime_exact;

fn make_env(command: &str, env: BTreeMap<String, String>) -> EffectiveAgentEnv {
    let runtime = known_acp_runtime_exact(command);
    EffectiveAgentEnv {
        env,
        config_file_path: runtime.and_then(|runtime| runtime.config_file_path),
        effective_command: command.to_string(),
    }
}

fn env_with(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

// ── provider-specific model fallback tests ────────────────────────────

#[test]
fn buzz_agent_databricks_v2_with_databricks_model_but_no_buzz_agent_model_is_ready() {
    // The baked buzz-releases env sets DATABRICKS_MODEL but not BUZZ_AGENT_MODEL.
    // An agent with only DATABRICKS_MODEL must pass the readiness gate.
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks_v2"),
            ("DATABRICKS_MODEL", "goose-claude-4-6-sonnet"),
            ("DATABRICKS_HOST", "https://dbc.example.com"),
        ]),
    );
    assert!(
        agent_readiness(&env).is_ready(),
        "DATABRICKS_MODEL must satisfy the model requirement for databricks_v2"
    );
}

#[test]
fn buzz_agent_databricks_v2_hyphen_alias_with_databricks_model_is_ready() {
    // buzz-agent accepts both "databricks_v2" and "databricks-v2". The
    // readiness gate must recognize the hyphen alias and accept DATABRICKS_MODEL.
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks-v2"),
            ("DATABRICKS_MODEL", "goose-claude-4-6-sonnet"),
            ("DATABRICKS_HOST", "https://dbc.example.com"),
        ]),
    );
    assert!(
        agent_readiness(&env).is_ready(),
        "databricks-v2 alias with DATABRICKS_MODEL must be Ready"
    );
}

#[test]
fn buzz_agent_databricks_hyphen_alias_missing_host_returns_not_ready() {
    // The hyphen alias "databricks-v2" requires DATABRICKS_HOST just like
    // the underscore variants. Without it the agent cannot reach the endpoint.
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks-v2"),
            ("DATABRICKS_MODEL", "goose-claude-4-6-sonnet"),
            // DATABRICKS_HOST intentionally absent
        ]),
    );
    let result = agent_readiness(&env);
    assert!(
        !result.is_ready(),
        "databricks-v2 without DATABRICKS_HOST must be NotReady"
    );
    let reqs = result.requirements();
    assert!(
        reqs.iter()
            .any(|r| matches!(r, Requirement::EnvKey { key } if key == "DATABRICKS_HOST")),
        "missing requirements must include DATABRICKS_HOST; got {reqs:?}"
    );
}

#[test]
fn buzz_agent_databricks_v1_with_databricks_model_but_no_buzz_agent_model_is_ready() {
    // V1 (Model Serving) also resolves DATABRICKS_MODEL — same fallback applies.
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks"),
            ("DATABRICKS_MODEL", "dbrx-instruct"),
            ("DATABRICKS_HOST", "https://dbc.example.com"),
        ]),
    );
    assert!(
        agent_readiness(&env).is_ready(),
        "DATABRICKS_MODEL must satisfy the model requirement for databricks (V1)"
    );
}

#[test]
fn buzz_agent_anthropic_with_anthropic_model_but_no_buzz_agent_model_is_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "anthropic"),
            ("ANTHROPIC_MODEL", "claude-opus-4-5"),
            ("ANTHROPIC_API_KEY", "sk-test"),
        ]),
    );
    assert!(
        agent_readiness(&env).is_ready(),
        "ANTHROPIC_MODEL must satisfy the model requirement for anthropic"
    );
}

#[test]
fn buzz_agent_openai_with_openai_compat_model_but_no_buzz_agent_model_is_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "openai"),
            ("OPENAI_COMPAT_MODEL", "gpt-4o"),
            ("OPENAI_COMPAT_API_KEY", "sk-test"),
        ]),
    );
    assert!(
        agent_readiness(&env).is_ready(),
        "OPENAI_COMPAT_MODEL must satisfy the model requirement for openai"
    );
}

#[test]
fn buzz_agent_empty_provider_model_fallback_key_is_not_ready() {
    // An empty DATABRICKS_MODEL with no BUZZ_AGENT_MODEL must still be NotReady.
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks_v2"),
            ("DATABRICKS_MODEL", ""),
            ("DATABRICKS_HOST", "https://dbc.example.com"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(
        !result.is_ready(),
        "empty DATABRICKS_MODEL with no BUZZ_AGENT_MODEL must be NotReady"
    );
    assert!(result
        .requirements()
        .contains(&Requirement::NormalizedField {
            field: "model".to_string()
        }));
}

// ── OpenRouter readiness ─────────────────────────────────────────────

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
