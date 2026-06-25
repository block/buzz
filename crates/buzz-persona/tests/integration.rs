//! Integration tests for the buzz-persona crate.
//!
//! These exercise the full pipeline: build a pack on disk → load it →
//! parse personas → resolve defaults → merge config → validate.
//! Each test creates a temporary pack directory with realistic content.

use std::fs;
use std::path::Path;

use buzz_persona::pack;
use buzz_persona::persona;
use buzz_persona::resolve;
use buzz_persona::validate;

/// Create a minimal valid pack in a temp directory.
/// Returns the temp dir (holds the lifetime) and the pack root path.
fn create_test_pack(dir: &Path) {
    let plugin_dir = dir.join(".plugin");
    fs::create_dir_all(&plugin_dir).unwrap();

    let agents_dir = dir.join("agents");
    fs::create_dir_all(&agents_dir).unwrap();

    let skills_dir = dir.join("skills").join("code-review");
    fs::create_dir_all(&skills_dir).unwrap();

    // plugin.json
    fs::write(
        plugin_dir.join("plugin.json"),
        r#"{
  "id": "com.test.example-pack",
  "name": "Example Pack",
  "version": "1.0.0",
  "description": "A test pack for integration tests.",
  "personas": [
    "agents/pip.persona.md",
    "agents/lep.persona.md"
  ],
  "defaults": {
    "model": "anthropic:claude-sonnet-4-20250514",
    "temperature": 0.7,
    "max_context_tokens": 128000,
    "triggers": {
      "mentions": true,
      "keywords": [],
      "all_messages": false
    },
    "thread_replies": true,
    "broadcast_replies": false
  },
  "pack_instructions": "instructions.md"
}"#,
    )
    .unwrap();

    // pip.persona.md — overrides model to Opus
    fs::write(
        agents_dir.join("pip.persona.md"),
        r##"---
name: "pip"
display_name: "Pip 🐝"
description: "Orchestration agent"
model: "anthropic:claude-4-opus-20250514"
subscribe:
  - "#security-reviews"
skills:
  - "./skills/code-review/"
---

You are Pip, an orchestration agent. You coordinate the team.
"##,
    )
    .unwrap();

    // lep.persona.md — uses pack defaults
    fs::write(
        agents_dir.join("lep.persona.md"),
        r##"---
name: "lep"
display_name: "Lep 🍀"
description: "Security-focused code reviewer"
triggers:
  mentions: true
  keywords:
    - "security"
    - "vulnerability"
    - "CVE"
---

You are Lep, a security-focused code reviewer on the team.
"##,
    )
    .unwrap();

    // SKILL.md
    fs::write(
        skills_dir.join("SKILL.md"),
        r##"---
name: "code-review"
description: "Reviews code for quality and correctness"
---

# Code Review

When asked to review code, follow these steps...
"##,
    )
    .unwrap();

    // instructions.md
    fs::write(
        dir.join("instructions.md"),
        "# Team Instructions\n\nBe helpful and thorough.\n",
    )
    .unwrap();
}

#[test]
fn full_pipeline_load_and_validate() {
    let dir = tempfile::tempdir().unwrap();
    create_test_pack(dir.path());

    // 1. Validate the pack — should be clean.
    let report = validate::validate_pack(dir.path());
    assert!(
        !report.has_errors(),
        "validation should pass on a valid pack, got: {report}"
    );

    // 2. Load the pack.
    let loaded = pack::load_pack(dir.path()).unwrap();

    // 3. Check manifest data.
    assert_eq!(loaded.manifest.id, "com.test.example-pack");
    assert_eq!(loaded.manifest.name, "Example Pack");
    assert_eq!(loaded.manifest.version, "1.0.0");

    // 4. Check personas loaded.
    assert_eq!(loaded.personas.len(), 2);

    let pip = loaded
        .personas
        .iter()
        .find(|p| p.name == "pip")
        .expect("pip persona should be loaded");
    let lep = loaded
        .personas
        .iter()
        .find(|p| p.name == "lep")
        .expect("lep persona should be loaded");

    // 5. Check pip's overrides.
    assert_eq!(pip.display_name, "Pip 🐝");
    assert_eq!(
        pip.model.as_deref(),
        Some("anthropic:claude-4-opus-20250514"),
        "pip should override model to Opus"
    );
    assert!(
        pip.prompt.contains("You are Pip"),
        "pip should have system prompt"
    );

    // 5b. pip does NOT set triggers → must inherit pack-default triggers.
    // This is the critical regression test for the respond_to/triggers fix.
    let pip_rt = pip
        .triggers
        .as_ref()
        .expect("pip should inherit triggers from pack defaults");
    assert!(
        pip_rt.mentions,
        "pip should inherit mentions=true from defaults"
    );
    assert!(
        pip_rt.keywords.is_empty(),
        "pip should inherit empty keywords from defaults"
    );
    assert!(
        !pip_rt.all_messages,
        "pip should inherit all_messages=false from defaults"
    );

    // 6. Check lep inherits pack defaults.
    assert_eq!(lep.display_name, "Lep 🍀");
    assert_eq!(
        lep.model.as_deref(),
        Some("anthropic:claude-sonnet-4-20250514"),
        "lep should inherit model from pack defaults"
    );
    assert!(lep.thread_replies, "lep should inherit thread_replies=true");
    assert!(
        !lep.broadcast_replies,
        "lep should inherit broadcast_replies=false"
    );

    // 7. Check lep's triggers override.
    let rt = lep.triggers.as_ref().expect("lep should have triggers");
    assert!(rt.mentions, "lep triggers.mentions should be true");
    assert!(
        rt.keywords.contains(&"security".to_string()),
        "lep should have 'security' keyword"
    );

    // 8. Pack instructions should be loaded.
    assert!(
        loaded.pack_instructions.is_some(),
        "pack instructions should be loaded"
    );
    assert!(
        loaded
            .pack_instructions
            .as_ref()
            .unwrap()
            .contains("Be helpful"),
        "instructions content should match"
    );
}

#[test]
fn persona_parse_round_trip() {
    let md = r###"---
name: "test-agent"
display_name: "Test Agent"
description: "A test agent for round-trip verification"
model: "anthropic:claude-sonnet-4-20250514"
temperature: 0.3
subscribe:
  - "#test-channel"
triggers:
  mentions: true
  keywords:
    - "test"
  all_messages: false
thread_replies: true
broadcast_replies: false
---

You are a test agent. Be precise and thorough.
"###;

    let config = persona::parse_persona_md(md).unwrap();

    assert_eq!(config.name, "test-agent");
    assert_eq!(config.display_name, "Test Agent");
    assert_eq!(
        config.description,
        "A test agent for round-trip verification"
    );
    assert_eq!(
        config.model.as_deref(),
        Some("anthropic:claude-sonnet-4-20250514")
    );
    assert_eq!(config.temperature, Some(0.3));
    assert_eq!(config.subscribe, Some(vec!["#test-channel".to_owned()]));
    assert!(config.prompt.contains("Be precise and thorough"));

    let rt = config.triggers.unwrap();
    assert_eq!(rt.mentions, Some(true));
    assert_eq!(rt.keywords, vec!["test"]);
    assert_eq!(rt.all_messages, Some(false));
}

#[test]
fn validation_catches_missing_required_fields() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join(".plugin");
    let agents_dir = dir.path().join("agents");
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::create_dir_all(&agents_dir).unwrap();

    // plugin.json with a persona reference.
    fs::write(
        plugin_dir.join("plugin.json"),
        r#"{
  "id": "com.test.bad-pack",
  "name": "Bad Pack",
  "version": "1.0.0",
  "personas": ["agents/bad.persona.md"]
}"#,
    )
    .unwrap();

    // Persona missing required fields.
    fs::write(
        agents_dir.join("bad.persona.md"),
        "---\nname: \"bad\"\n---\nNo display_name or description.\n",
    )
    .unwrap();

    let report = validate::validate_pack(dir.path());
    // load_pack fails on the first missing required field — the pack is
    // structurally invalid and a single error is emitted.
    assert!(report.has_errors(), "should flag missing required fields");
}

#[test]
fn validation_catches_unknown_behavioral_keys() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join(".plugin");
    let agents_dir = dir.path().join("agents");
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::create_dir_all(&agents_dir).unwrap();

    // plugin.json with a typo in defaults.
    fs::write(
        plugin_dir.join("plugin.json"),
        r#"{
  "id": "com.test.typo-pack",
  "name": "Typo Pack",
  "version": "1.0.0",
  "personas": ["agents/t.persona.md"],
  "defaults": {
    "temprature": 0.5,
    "model": "test"
  }
}"#,
    )
    .unwrap();
    fs::write(
        agents_dir.join("t.persona.md"),
        "---\nname: t\ndisplay_name: T\ndescription: T.\n---\n",
    )
    .unwrap();

    let report = validate::validate_pack(dir.path());
    // Unknown manifest keys are advisory warnings, not hard errors.
    assert!(
        !report.has_errors(),
        "unknown manifest keys should not be errors"
    );
    assert!(report.has_warnings(), "should catch typo in defaults");
    let warn_str = format!("{report}");
    assert!(
        warn_str.contains("temprature"),
        "should mention the typo: {warn_str}"
    );
}

#[test]
fn model_split_cases() {
    // provider:model
    let (provider, model) = persona::split_model("anthropic:claude-sonnet-4-20250514");
    assert_eq!(provider, Some("anthropic"));
    assert_eq!(model, "claude-sonnet-4-20250514");

    // no colon — entire string is model
    let (provider, model) = persona::split_model("gpt-4o");
    assert_eq!(provider, None);
    assert_eq!(model, "gpt-4o");

    // multiple colons — split on first only
    let (provider, model) = persona::split_model("custom:my:model:v2");
    assert_eq!(provider, Some("custom"));
    assert_eq!(model, "my:model:v2");
}

#[test]
fn defaults_merge_persona_overrides() {
    let dir = tempfile::tempdir().unwrap();
    create_test_pack(dir.path());

    let loaded = pack::load_pack(dir.path()).unwrap();

    let pip = loaded.personas.iter().find(|p| p.name == "pip").unwrap();
    let lep = loaded.personas.iter().find(|p| p.name == "lep").unwrap();

    // pip overrides model; lep inherits.
    assert_eq!(
        pip.model.as_deref(),
        Some("anthropic:claude-4-opus-20250514")
    );
    assert_eq!(
        lep.model.as_deref(),
        Some("anthropic:claude-sonnet-4-20250514")
    );

    // Both should inherit temperature from defaults (0.7).
    assert_eq!(pip.temperature, Some(0.7));
    assert_eq!(lep.temperature, Some(0.7));

    // Both should inherit max_context_tokens from defaults (128000).
    assert_eq!(pip.max_context_tokens, Some(128000));
    assert_eq!(lep.max_context_tokens, Some(128000));
}

/// Build a pack on disk → resolve → verify all fields on each persona.
#[test]
fn resolve_full_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    create_test_pack(dir.path());

    let resolved = resolve::resolve_pack(dir.path()).unwrap();

    assert_eq!(resolved.id, "com.test.example-pack");
    assert_eq!(resolved.name, "Example Pack");
    assert_eq!(resolved.version, "1.0.0");
    assert_eq!(resolved.personas.len(), 2);

    let pip = resolved
        .personas
        .iter()
        .find(|p| p.name == "pip")
        .expect("pip should be resolved");
    let lep = resolved
        .personas
        .iter()
        .find(|p| p.name == "lep")
        .expect("lep should be resolved");

    // Identity
    assert_eq!(pip.display_name, "Pip 🐝");
    assert_eq!(pip.description, "Orchestration agent");
    assert_eq!(pip.version, "1.0.0"); // defaults to pack version

    // Model split: "anthropic:claude-4-opus-20250514" → llm_provider + model
    assert_eq!(pip.llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(pip.model.as_deref(), Some("claude-4-opus-20250514"));

    // Lep inherits pack default model
    assert_eq!(lep.llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(lep.model.as_deref(), Some("claude-sonnet-4-20250514"));

    // System prompt composed: persona body + pack instructions
    assert!(
        pip.system_prompt.contains("You are Pip"),
        "pip prompt should contain persona body"
    );
    assert!(
        pip.system_prompt.contains("Be helpful"),
        "pip prompt should contain pack instructions"
    );
    assert!(
        pip.system_prompt.contains("Team Instructions"),
        "pip prompt should contain instructions header"
    );

    // Temperature inherited from defaults
    assert_eq!(pip.temperature, Some(0.7));
    assert_eq!(lep.temperature, Some(0.7));

    // Subscribe
    assert_eq!(pip.subscribe, vec!["#security-reviews"]);

    // Triggers: lep has explicit triggers
    assert!(lep.triggers.mentions);
    assert!(lep.triggers.keywords.contains(&"security".to_string()));
    assert!(lep.triggers.keywords.contains(&"vulnerability".to_string()));
    assert!(!lep.triggers.all_messages);

    // Env vars projected from model
    let pip_env: std::collections::HashMap<_, _> = pip.runtime_env_vars.iter().cloned().collect();
    assert_eq!(
        pip_env.get("GOOSE_PROVIDER").map(|s| s.as_str()),
        Some("anthropic")
    );
    assert_eq!(
        pip_env.get("GOOSE_MODEL").map(|s| s.as_str()),
        Some("claude-4-opus-20250514")
    );
    assert_eq!(
        pip_env.get("GOOSE_TEMPERATURE").map(|s| s.as_str()),
        Some("0.7")
    );
}

/// Pack with 3 personas, each with different configs.
#[test]
fn resolve_multi_persona_pack() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join(".plugin")).unwrap();
    fs::create_dir_all(root.join("agents")).unwrap();

    fs::write(
        root.join(".plugin/plugin.json"),
        r#"{
  "id": "com.test.multi",
  "name": "Multi Pack",
  "version": "2.0.0",
  "personas": [
    "agents/alpha.persona.md",
    "agents/beta.persona.md",
    "agents/gamma.persona.md"
  ],
  "defaults": {
    "model": "openai:gpt-4o",
    "temperature": 0.5,
    "thread_replies": true,
    "broadcast_replies": false
  }
}"#,
    )
    .unwrap();

    // Alpha: overrides model and temperature
    fs::write(
        root.join("agents/alpha.persona.md"),
        "---\nname: alpha\ndisplay_name: Alpha\ndescription: The first.\nmodel: \"anthropic:claude-sonnet-4-20250514\"\ntemperature: 0.9\n---\nYou are Alpha.\n",
    )
    .unwrap();

    // Beta: uses all defaults
    fs::write(
        root.join("agents/beta.persona.md"),
        "---\nname: beta\ndisplay_name: Beta\ndescription: The second.\n---\nYou are Beta.\n",
    )
    .unwrap();

    // Gamma: overrides subscribe and thread_replies
    fs::write(
        root.join("agents/gamma.persona.md"),
        "---\nname: gamma\ndisplay_name: Gamma\ndescription: The third.\nsubscribe:\n  - \"#ops\"\nthread_replies: false\n---\nYou are Gamma.\n",
    )
    .unwrap();

    let resolved = resolve::resolve_pack(root).unwrap();
    assert_eq!(resolved.personas.len(), 3);

    let alpha = resolved
        .personas
        .iter()
        .find(|p| p.name == "alpha")
        .unwrap();
    let beta = resolved.personas.iter().find(|p| p.name == "beta").unwrap();
    let gamma = resolved
        .personas
        .iter()
        .find(|p| p.name == "gamma")
        .unwrap();

    // Alpha overrides model and temperature
    assert_eq!(alpha.llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(alpha.model.as_deref(), Some("claude-sonnet-4-20250514"));
    assert_eq!(alpha.temperature, Some(0.9));

    // Beta inherits all defaults
    assert_eq!(beta.llm_provider.as_deref(), Some("openai"));
    assert_eq!(beta.model.as_deref(), Some("gpt-4o"));
    assert_eq!(beta.temperature, Some(0.5));
    assert!(beta.thread_replies);
    assert!(!beta.broadcast_replies);

    // Gamma overrides subscribe and thread_replies
    assert_eq!(gamma.subscribe, vec!["#ops"]);
    assert!(!gamma.thread_replies);

    // All share pack version
    assert_eq!(alpha.version, "2.0.0");
    assert_eq!(beta.version, "2.0.0");
    assert_eq!(gamma.version, "2.0.0");
}

/// resolve_persona_by_name finds the correct persona.
#[test]
fn resolve_persona_by_name_found() {
    let dir = tempfile::tempdir().unwrap();
    create_test_pack(dir.path());

    let pip = resolve::resolve_persona_by_name(dir.path(), "pip").unwrap();
    assert_eq!(pip.name, "pip");
    assert_eq!(pip.display_name, "Pip 🐝");
}

/// resolve_persona_by_name returns error for nonexistent name.
#[test]
fn resolve_persona_by_name_not_found() {
    let dir = tempfile::tempdir().unwrap();
    create_test_pack(dir.path());

    let err = resolve::resolve_persona_by_name(dir.path(), "nonexistent").unwrap_err();
    assert!(
        format!("{err}").contains("not found")
            || matches!(err, pack::PackError::PersonaNotFound(_)),
        "expected PersonaNotFound, got: {err}"
    );
}

/// Validation catches zero-persona packs in the full pipeline.
#[test]
fn validate_zero_personas_in_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join(".plugin")).unwrap();
    fs::write(
        root.join(".plugin/plugin.json"),
        r#"{"id":"com.test.empty","name":"Empty","version":"1.0.0","personas":[]}"#,
    )
    .unwrap();

    let report = validate::validate_pack(root);
    assert!(report.has_errors());
    let msg = format!("{report}");
    assert!(msg.contains("zero personas"), "got: {msg}");
}

/// Validation catches duplicate persona names in the full pipeline.
#[test]
fn validate_duplicate_names_in_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join(".plugin")).unwrap();
    fs::create_dir_all(root.join("agents")).unwrap();

    fs::write(
        root.join(".plugin/plugin.json"),
        r#"{"id":"com.test.dupes","name":"Dupes","version":"1.0.0","personas":["agents/a.persona.md","agents/b.persona.md"]}"#,
    )
    .unwrap();
    fs::write(
        root.join("agents/a.persona.md"),
        "---\nname: same\ndisplay_name: A\ndescription: First.\n---\n",
    )
    .unwrap();
    fs::write(
        root.join("agents/b.persona.md"),
        "---\nname: same\ndisplay_name: B\ndescription: Second.\n---\n",
    )
    .unwrap();

    let report = validate::validate_pack(root);
    assert!(report.has_errors());
    let msg = format!("{report}");
    assert!(msg.contains("duplicate persona name"), "got: {msg}");
}

/// Operator config fields are rejected in persona frontmatter.
/// This documents the security boundary: pack authors define behavior,
/// operators define limits.
#[test]
fn operator_config_fields_rejected_in_frontmatter() {
    for field in [
        "idle_timeout",
        "max_turn_duration",
        "agents",
        "heartbeat_interval",
        "permission_mode",
    ] {
        let src =
            format!("---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\n{field}: 300\n---\n");
        assert!(
            persona::parse_persona_md(&src).is_err(),
            "{field} should be rejected by deny_unknown_fields"
        );
    }
}
