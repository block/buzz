//! End-to-end tests for the env var flow introduced in PRs #783 and #794.
//!
//! These tests exercise the full pack-resolve pipeline and verify that:
//! - Goose personas emit GOOSE_PROVIDER, GOOSE_MODEL, GOOSE_TEMPERATURE
//! - Buzz-agent personas emit BUZZ_AGENT_MODEL, BUZZ_AGENT_PROVIDER
//! - The import filter strips derived provider/model keys but preserves knobs
//! - Multi-runtime packs produce correct per-persona env var prefixes
//! - Models without a provider prefix emit only the model key (no provider)

use std::collections::BTreeMap;
use std::fs;

use buzz_persona::resolve::resolve_pack;

const DERIVED_PROVIDER_MODEL_ENV_KEYS: &[&str] = &[
    "GOOSE_MODEL",
    "GOOSE_PROVIDER",
    "BUZZ_AGENT_MODEL",
    "BUZZ_AGENT_PROVIDER",
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

#[test]
fn resolve_pack_buzz_agent_persona_emits_buzz_agent_vars() {
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
runtime: "buzz-agent"
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
        env.get("BUZZ_AGENT_MODEL"),
        Some(&"gpt-4o"),
        "should emit BUZZ_AGENT_MODEL=gpt-4o"
    );
    assert_eq!(
        env.get("BUZZ_AGENT_PROVIDER"),
        Some(&"openai"),
        "should emit BUZZ_AGENT_PROVIDER=openai"
    );

    // Must NOT contain GOOSE_* keys
    assert!(
        !env.contains_key("GOOSE_MODEL"),
        "buzz-agent runtime must not emit GOOSE_MODEL"
    );
    assert!(
        !env.contains_key("GOOSE_PROVIDER"),
        "buzz-agent runtime must not emit GOOSE_PROVIDER"
    );
}

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
    "agents/buzz-bot.persona.md"
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

    // Buzz-agent persona
    fs::write(
        root.join("agents/buzz-bot.persona.md"),
        r#"---
name: "buzz-bot"
display_name: "Buzz Bot"
description: "A buzz-agent runtime bot"
runtime: "buzz-agent"
model: "openai:gpt-4o"
---
You are a buzz bot.
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
    let buzz = pack
        .personas
        .iter()
        .find(|p| p.name == "buzz-bot")
        .expect("buzz-bot should exist");

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
        !goose_env.contains_key("BUZZ_AGENT_MODEL"),
        "goose persona must not emit BUZZ_AGENT_MODEL"
    );
    assert!(
        !goose_env.contains_key("BUZZ_AGENT_PROVIDER"),
        "goose persona must not emit BUZZ_AGENT_PROVIDER"
    );

    // Buzz-agent persona gets BUZZ_AGENT_* env vars
    let buzz_env: std::collections::HashMap<_, _> = buzz
        .runtime_env_vars
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(buzz_env.get("BUZZ_AGENT_MODEL"), Some(&"gpt-4o"));
    assert_eq!(buzz_env.get("BUZZ_AGENT_PROVIDER"), Some(&"openai"));
    assert!(
        !buzz_env.contains_key("GOOSE_MODEL"),
        "buzz-agent persona must not emit GOOSE_MODEL"
    );
    assert!(
        !buzz_env.contains_key("GOOSE_PROVIDER"),
        "buzz-agent persona must not emit GOOSE_PROVIDER"
    );
}

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
