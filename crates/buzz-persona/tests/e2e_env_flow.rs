//! End-to-end tests for the env var flow introduced in PRs #783 and #794.
//!
//! These tests exercise the full pack-resolve pipeline and verify that:
//! - Goose personas emit GOOSE_PROVIDER, GOOSE_MODEL, GOOSE_TEMPERATURE
//! - Sprout-agent personas emit SPROUT_AGENT_MODEL, SPROUT_AGENT_PROVIDER
//! - The import filter strips derived provider/model keys but preserves knobs
//! - Multi-runtime packs produce correct per-persona env var prefixes
//! - Models without a provider prefix emit only the model key (no provider)

use std::collections::BTreeMap;
use std::fs;

use sprout_persona::resolve::resolve_pack;

// ── Import filter (replicates desktop crate logic) ───────────────────────────

const DERIVED_PROVIDER_MODEL_ENV_KEYS: &[&str] = &[
    "GOOSE_MODEL",
    "GOOSE_PROVIDER",
    "SPROUT_AGENT_MODEL",
    "SPROUT_AGENT_PROVIDER",
];

fn filter_derived(env_vars: Vec<(String, String)>) -> BTreeMap<String, String> {
    env_vars
        .into_iter()
        .filter(|(k, _)| {
            !DERIVED_PROVIDER_MODEL_ENV_KEYS
                .iter()
                .any(|d| d.eq_ignore_ascii_case(k))
        })
        .collect()
}

// ── Test 1: Goose persona emits correct runtime env vars ─────────────────────

#[test]
fn resolve_pack_goose_persona_emits_correct_runtime_env_vars() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join(".plugin")).unwrap();
    fs::create_dir_all(root.join("agents")).unwrap();

    fs::write(
        root.join(".plugin/plugin.json"),
        r#"{
  "id": "com.test.e2e-env",
  "name": "E2E Env Test",
  "version": "1.0.0",
  "personas": ["agents/bot.persona.md"],
  "defaults": {}
}"#,
    )
    .unwrap();

    fs::write(
        root.join("agents/bot.persona.md"),
        r#"---
name: "bot"
display_name: "Bot"
description: "Test bot"
model: "databricks:goose-claude-4-6-opus"
temperature: 0.7
---
You are a test bot.
"#,
    )
    .unwrap();

    let pack = resolve_pack(root).unwrap();
    let persona = &pack.personas[0];

    let env: std::collections::HashMap<_, _> = persona
        .runtime_env_vars
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    assert_eq!(
        env.get("GOOSE_PROVIDER"),
        Some(&"databricks"),
        "should emit GOOSE_PROVIDER=databricks"
    );
    assert_eq!(
        env.get("GOOSE_MODEL"),
        Some(&"goose-claude-4-6-opus"),
        "should emit GOOSE_MODEL=goose-claude-4-6-opus"
    );
    assert_eq!(
        env.get("GOOSE_TEMPERATURE"),
        Some(&"0.7"),
        "should emit GOOSE_TEMPERATURE=0.7"
    );
}

// ── Test 2: Sprout-agent persona emits SPROUT_AGENT_* vars ───────────────────

#[test]
fn resolve_pack_sprout_agent_persona_emits_sprout_agent_vars() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join(".plugin")).unwrap();
    fs::create_dir_all(root.join("agents")).unwrap();

    fs::write(
        root.join(".plugin/plugin.json"),
        r#"{
  "id": "com.test.e2e-env",
  "name": "E2E Env Test",
  "version": "1.0.0",
  "personas": ["agents/bot.persona.md"],
  "defaults": {}
}"#,
    )
    .unwrap();

    fs::write(
        root.join("agents/bot.persona.md"),
        r#"---
name: "bot"
display_name: "Bot"
description: "Test bot"
runtime: "sprout-agent"
model: "openai:gpt-4o"
---
You are a test bot.
"#,
    )
    .unwrap();

    let pack = resolve_pack(root).unwrap();
    let persona = &pack.personas[0];

    let env: std::collections::HashMap<_, _> = persona
        .runtime_env_vars
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    assert_eq!(
        env.get("SPROUT_AGENT_MODEL"),
        Some(&"gpt-4o"),
        "should emit SPROUT_AGENT_MODEL=gpt-4o"
    );
    assert_eq!(
        env.get("SPROUT_AGENT_PROVIDER"),
        Some(&"openai"),
        "should emit SPROUT_AGENT_PROVIDER=openai"
    );

    // Must NOT contain GOOSE_* keys
    assert!(
        !env.contains_key("GOOSE_MODEL"),
        "sprout-agent runtime must not emit GOOSE_MODEL"
    );
    assert!(
        !env.contains_key("GOOSE_PROVIDER"),
        "sprout-agent runtime must not emit GOOSE_PROVIDER"
    );
}

// ── Test 3: Import filter strips derived keys, preserves knobs ───────────────

#[test]
fn import_filter_strips_derived_preserves_knobs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join(".plugin")).unwrap();
    fs::create_dir_all(root.join("agents")).unwrap();

    fs::write(
        root.join(".plugin/plugin.json"),
        r#"{
  "id": "com.test.e2e-env",
  "name": "E2E Env Test",
  "version": "1.0.0",
  "personas": ["agents/bot.persona.md"],
  "defaults": {}
}"#,
    )
    .unwrap();

    fs::write(
        root.join("agents/bot.persona.md"),
        r#"---
name: "bot"
display_name: "Bot"
description: "Test bot"
model: "databricks:goose-claude-4-6-opus"
temperature: 0.7
---
You are a test bot.
"#,
    )
    .unwrap();

    let pack = resolve_pack(root).unwrap();
    let persona = &pack.personas[0];

    // Apply the import filter (mirrors desktop import_persona_pack logic).
    let filtered = filter_derived(persona.runtime_env_vars.clone());

    // Derived provider/model keys must be stripped.
    assert!(
        !filtered.contains_key("GOOSE_MODEL"),
        "GOOSE_MODEL must be stripped by import filter"
    );
    assert!(
        !filtered.contains_key("GOOSE_PROVIDER"),
        "GOOSE_PROVIDER must be stripped by import filter"
    );

    // Knob keys must survive.
    assert_eq!(
        filtered.get("GOOSE_TEMPERATURE").map(|s| s.as_str()),
        Some("0.7"),
        "GOOSE_TEMPERATURE must survive the import filter"
    );
}

// ── Test 4: Two runtimes in one pack get different env var prefixes ───────────

#[test]
fn full_pipeline_two_runtimes_different_env_vars() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join(".plugin")).unwrap();
    fs::create_dir_all(root.join("agents")).unwrap();

    fs::write(
        root.join(".plugin/plugin.json"),
        r#"{
  "id": "com.test.e2e-env",
  "name": "E2E Env Test",
  "version": "1.0.0",
  "personas": [
    "agents/goose-bot.persona.md",
    "agents/sprout-bot.persona.md"
  ],
  "defaults": {}
}"#,
    )
    .unwrap();

    // Goose persona (default runtime)
    fs::write(
        root.join("agents/goose-bot.persona.md"),
        r#"---
name: "goose-bot"
display_name: "Goose Bot"
description: "A goose runtime bot"
model: "anthropic:claude-sonnet-4-20250514"
---
You are a goose bot.
"#,
    )
    .unwrap();

    // Sprout-agent persona
    fs::write(
        root.join("agents/sprout-bot.persona.md"),
        r#"---
name: "sprout-bot"
display_name: "Sprout Bot"
description: "A sprout-agent runtime bot"
runtime: "sprout-agent"
model: "openai:gpt-4o"
---
You are a sprout bot.
"#,
    )
    .unwrap();

    let pack = resolve_pack(root).unwrap();
    assert_eq!(pack.personas.len(), 2);

    let goose = pack
        .personas
        .iter()
        .find(|p| p.name == "goose-bot")
        .expect("goose-bot should exist");
    let sprout = pack
        .personas
        .iter()
        .find(|p| p.name == "sprout-bot")
        .expect("sprout-bot should exist");

    // Goose persona gets GOOSE_* env vars
    let goose_env: std::collections::HashMap<_, _> = goose
        .runtime_env_vars
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(goose_env.get("GOOSE_PROVIDER"), Some(&"anthropic"));
    assert_eq!(
        goose_env.get("GOOSE_MODEL"),
        Some(&"claude-sonnet-4-20250514")
    );
    assert!(
        !goose_env.contains_key("SPROUT_AGENT_MODEL"),
        "goose persona must not emit SPROUT_AGENT_MODEL"
    );
    assert!(
        !goose_env.contains_key("SPROUT_AGENT_PROVIDER"),
        "goose persona must not emit SPROUT_AGENT_PROVIDER"
    );

    // Sprout-agent persona gets SPROUT_AGENT_* env vars
    let sprout_env: std::collections::HashMap<_, _> = sprout
        .runtime_env_vars
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(sprout_env.get("SPROUT_AGENT_MODEL"), Some(&"gpt-4o"));
    assert_eq!(sprout_env.get("SPROUT_AGENT_PROVIDER"), Some(&"openai"));
    assert!(
        !sprout_env.contains_key("GOOSE_MODEL"),
        "sprout-agent persona must not emit GOOSE_MODEL"
    );
    assert!(
        !sprout_env.contains_key("GOOSE_PROVIDER"),
        "sprout-agent persona must not emit GOOSE_PROVIDER"
    );
}

// ── Test 5: Model without provider prefix emits model only ───────────────────

#[test]
fn model_without_provider_prefix_emits_model_only() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join(".plugin")).unwrap();
    fs::create_dir_all(root.join("agents")).unwrap();

    fs::write(
        root.join(".plugin/plugin.json"),
        r#"{
  "id": "com.test.e2e-env",
  "name": "E2E Env Test",
  "version": "1.0.0",
  "personas": ["agents/bot.persona.md"],
  "defaults": {}
}"#,
    )
    .unwrap();

    fs::write(
        root.join("agents/bot.persona.md"),
        r#"---
name: "bot"
display_name: "Bot"
description: "Test bot"
model: "gpt-4o"
---
You are a test bot.
"#,
    )
    .unwrap();

    let pack = resolve_pack(root).unwrap();
    let persona = &pack.personas[0];

    let env: std::collections::HashMap<_, _> = persona
        .runtime_env_vars
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    assert_eq!(
        env.get("GOOSE_MODEL"),
        Some(&"gpt-4o"),
        "should emit GOOSE_MODEL=gpt-4o"
    );
    assert!(
        !env.contains_key("GOOSE_PROVIDER"),
        "model without colon prefix must NOT emit GOOSE_PROVIDER"
    );
}
