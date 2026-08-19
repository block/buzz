use super::test_support::*;
use super::*;

// ── reconcile_databricks_v1_to_v2_in_file ────────────────────────────────

#[test]
fn reconcile_databricks_v1_to_v2_rewrites_v1_provider_on_block_build() {
    // rewrite_v1_provider=true simulates a Block build (baked env has
    // BUZZ_AGENT_PROVIDER=databricks_v2). The structured provider field
    // must be migrated V1→V2 and the stale V1 model field must be cleared
    // so the baked DATABRICKS_MODEL wins at spawn time instead of the V1 name.
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Brain",
            "provider": "databricks",
            "model": "dbrx-instruct"
        }]),
    );

    reconcile_databricks_v1_to_v2_in_file(
        &dir.path().join("agents/managed-agents.json"),
        /*rewrite_v1_provider=*/ true,
    );

    let records = read_agents_json(dir.path());
    assert_eq!(
        records[0]["provider"], "databricks_v2",
        "provider: \"databricks\" must be rewritten to \"databricks_v2\" on Block builds"
    );
    // Stale V1 model must be cleared so the baked DATABRICKS_MODEL is not
    // shadowed by BUZZ_AGENT_MODEL at spawn time (last-write-wins in Command::env).
    assert!(
        records[0].get("model").is_none_or(|v| v.is_null()),
        "stale V1 model field must be cleared when provider is rewritten to V2"
    );
}

#[test]
fn reconcile_databricks_v1_to_v2_preserves_v1_provider_on_oss_build() {
    // rewrite_v1_provider=false simulates an OSS build (empty baked env).
    // V1 ("databricks") is a valid Model Serving provider for OSS users;
    // the structured provider field must NOT be rewritten.
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Brain",
            "provider": "databricks",
            "model": "dbrx-instruct",
            "env_vars": { "BUZZ_AGENT_PROVIDER": "databricks" }
        }]),
    );

    reconcile_databricks_v1_to_v2_in_file(
        &dir.path().join("agents/managed-agents.json"),
        /*rewrite_v1_provider=*/ false,
    );

    let records = read_agents_json(dir.path());
    // Provider field preserved.
    assert_eq!(
        records[0]["provider"], "databricks",
        "provider field must not be rewritten on OSS builds"
    );
    assert_eq!(records[0]["model"], "dbrx-instruct");
    // Stale env var is still stripped even on OSS builds.
    assert!(
        records[0]["env_vars"].get("BUZZ_AGENT_PROVIDER").is_none(),
        "BUZZ_AGENT_PROVIDER must be stripped even when provider rewrite is disabled"
    );
}

#[test]
fn reconcile_databricks_v1_to_v2_clears_model_on_provider_rewrite() {
    // When a V1 record is migrated to V2 on a Block build, the model field
    // must be removed. A stale V1 model name (e.g. "dbrx-instruct") emitted
    // via BUZZ_AGENT_MODEL at spawn time would shadow the baked DATABRICKS_MODEL
    // (last-write-wins), sending the agent to a V1 model on V2 endpoints.
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            { "name": "A", "provider": "databricks", "model": "dbrx-instruct" },
            { "name": "B", "provider": "databricks", "model": "goose-claude-opus-4-8-wrong" },
            // V2 record with model — model must NOT be cleared.
            { "name": "C", "provider": "databricks_v2", "model": "goose-claude-4-8-opus" }
        ]),
    );

    reconcile_databricks_v1_to_v2_in_file(
        &dir.path().join("agents/managed-agents.json"),
        /*rewrite_v1_provider=*/ true,
    );

    let records = read_agents_json(dir.path());
    // V1 records: provider migrated, model cleared.
    assert_eq!(records[0]["provider"], "databricks_v2");
    assert!(
        records[0].get("model").is_none_or(|v| v.is_null()),
        "model must be cleared for V1→V2 migrated record A"
    );
    assert_eq!(records[1]["provider"], "databricks_v2");
    assert!(
        records[1].get("model").is_none_or(|v| v.is_null()),
        "model must be cleared for V1→V2 migrated record B"
    );
    // V2 record: model untouched.
    assert_eq!(records[2]["provider"], "databricks_v2");
    assert_eq!(
        records[2]["model"], "goose-claude-4-8-opus",
        "model must not be cleared for already-V2 record C"
    );
}

#[test]
fn reconcile_databricks_v1_to_v2_preserves_v2_provider() {
    let dir = tempfile::tempdir().unwrap();
    let json = serde_json::json!([{
        "name": "Brain",
        "provider": "databricks_v2",
        "model": "goose-claude-4-6-sonnet"
    }]);
    write_agents_json(dir.path(), &json);
    let path = dir.path().join("agents/managed-agents.json");
    let before = std::fs::read_to_string(&path).unwrap();

    reconcile_databricks_v1_to_v2_in_file(&path, /*rewrite_v1_provider=*/ true);

    // File must be unchanged — no spurious re-write.
    assert_eq!(before, std::fs::read_to_string(&path).unwrap());
}

#[test]
fn reconcile_databricks_v1_to_v2_strips_stale_buzz_agent_provider_from_env_vars() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Brain",
            "provider": "databricks_v2",
            "model": "goose-claude-4-6-sonnet",
            "env_vars": {
                "BUZZ_AGENT_PROVIDER": "databricks",
                "DATABRICKS_HOST": "https://dbc.example.com"
            }
        }]),
    );

    reconcile_databricks_v1_to_v2_in_file(
        &dir.path().join("agents/managed-agents.json"),
        /*rewrite_v1_provider=*/ true,
    );

    let records = read_agents_json(dir.path());
    // Stale derived key must be removed.
    assert!(
        records[0]["env_vars"].get("BUZZ_AGENT_PROVIDER").is_none(),
        "BUZZ_AGENT_PROVIDER must be stripped from env_vars"
    );
    // Non-derived keys must be preserved.
    assert_eq!(
        records[0]["env_vars"]["DATABRICKS_HOST"],
        "https://dbc.example.com"
    );
}

#[test]
fn reconcile_databricks_v1_to_v2_strips_all_derived_keys_from_env_vars() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Brain",
            "provider": "anthropic",
            "model": "claude-opus-4-5",
            "env_vars": {
                "BUZZ_AGENT_PROVIDER": "anthropic",
                "BUZZ_AGENT_MODEL": "claude-opus-4-5",
                "GOOSE_PROVIDER": "anthropic",
                "GOOSE_MODEL": "claude-opus-4-5",
                "ANTHROPIC_API_KEY": "sk-test"
            }
        }]),
    );

    reconcile_databricks_v1_to_v2_in_file(
        &dir.path().join("agents/managed-agents.json"),
        /*rewrite_v1_provider=*/ true,
    );

    let records = read_agents_json(dir.path());
    let env_vars = &records[0]["env_vars"];
    // All four derived keys must be stripped.
    assert!(env_vars.get("BUZZ_AGENT_PROVIDER").is_none());
    assert!(env_vars.get("BUZZ_AGENT_MODEL").is_none());
    assert!(env_vars.get("GOOSE_PROVIDER").is_none());
    assert!(env_vars.get("GOOSE_MODEL").is_none());
    // Non-derived key must be preserved.
    assert_eq!(env_vars["ANTHROPIC_API_KEY"], "sk-test");
}

#[test]
fn reconcile_databricks_v1_to_v2_handles_multiple_records_block_build() {
    // On Block builds (rewrite_v1_provider=true): V1 provider is migrated and
    // env_vars stripping applies to every record.
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            {
                "name": "Agent A",
                "provider": "databricks",
                "env_vars": { "BUZZ_AGENT_PROVIDER": "databricks" }
            },
            {
                "name": "Agent B",
                "provider": "anthropic",
                "env_vars": { "BUZZ_AGENT_MODEL": "claude-3-5-sonnet" }
            },
            {
                "name": "Agent C",
                "provider": "databricks_v2",
                "env_vars": {}
            }
        ]),
    );

    reconcile_databricks_v1_to_v2_in_file(
        &dir.path().join("agents/managed-agents.json"),
        /*rewrite_v1_provider=*/ true,
    );

    let records = read_agents_json(dir.path());
    // A: provider rewritten, stale env_var stripped.
    assert_eq!(records[0]["provider"], "databricks_v2");
    assert!(records[0]["env_vars"].get("BUZZ_AGENT_PROVIDER").is_none());
    // B: provider untouched, stale BUZZ_AGENT_MODEL stripped.
    assert_eq!(records[1]["provider"], "anthropic");
    assert!(records[1]["env_vars"].get("BUZZ_AGENT_MODEL").is_none());
    // C: V2 provider, no stale keys — unchanged.
    assert_eq!(records[2]["provider"], "databricks_v2");
}

#[test]
fn reconcile_databricks_v1_to_v2_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Brain",
            "provider": "databricks",
            "env_vars": { "BUZZ_AGENT_PROVIDER": "databricks" }
        }]),
    );
    let path = dir.path().join("agents/managed-agents.json");

    reconcile_databricks_v1_to_v2_in_file(&path, /*rewrite_v1_provider=*/ true);
    let after_first = std::fs::read_to_string(&path).unwrap();
    reconcile_databricks_v1_to_v2_in_file(&path, /*rewrite_v1_provider=*/ true);
    let after_second = std::fs::read_to_string(&path).unwrap();

    assert_eq!(
        after_first, after_second,
        "second pass must not modify the file"
    );
}

#[test]
fn reconcile_databricks_v1_to_v2_preserves_non_databricks_providers() {
    let dir = tempfile::tempdir().unwrap();
    let json = serde_json::json!([
        { "name": "A", "provider": "anthropic" },
        { "name": "B", "provider": "openai" },
        { "name": "C", "provider": "openai-compat" },
    ]);
    write_agents_json(dir.path(), &json);
    let path = dir.path().join("agents/managed-agents.json");
    let before = std::fs::read_to_string(&path).unwrap();

    reconcile_databricks_v1_to_v2_in_file(&path, /*rewrite_v1_provider=*/ true);

    // No provider is modified, so the file content is identical.
    assert_eq!(before, std::fs::read_to_string(&path).unwrap());
}

#[test]
fn reconcile_databricks_v1_to_v2_strips_derived_keys_from_keyless_persona_definition() {
    // Folded persona definitions land in managed-agents.json without a
    // "provider" key (they are keyless/definition records). Stale derived env
    // keys in their env_vars must be stripped by the migration just like
    // full agent records — persona env is merged after runtime metadata and
    // can shadow structured fields at spawn time.
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            // No "provider" or "model" key — this is a folded persona definition.
            "name": "Fizz",
            "persona_id": "builtin:fizz",
            "env_vars": {
                "BUZZ_AGENT_PROVIDER": "databricks",
                "BUZZ_AGENT_MODEL": "goose-claude-4-6-sonnet",
                "DATABRICKS_HOST": "https://dbc.example.com"
            }
        }]),
    );

    reconcile_databricks_v1_to_v2_in_file(
        &dir.path().join("agents/managed-agents.json"),
        /*rewrite_v1_provider=*/ true,
    );

    let records = read_agents_json(dir.path());
    let env_vars = &records[0]["env_vars"];
    // Derived keys stripped even though there is no top-level "provider" field.
    assert!(
        env_vars.get("BUZZ_AGENT_PROVIDER").is_none(),
        "BUZZ_AGENT_PROVIDER must be stripped from keyless persona definition env_vars"
    );
    assert!(
        env_vars.get("BUZZ_AGENT_MODEL").is_none(),
        "BUZZ_AGENT_MODEL must be stripped from keyless persona definition env_vars"
    );
    // Non-derived key preserved.
    assert_eq!(env_vars["DATABRICKS_HOST"], "https://dbc.example.com");
}

#[test]
fn reconcile_databricks_v1_to_v2_strips_derived_keys_case_insensitively() {
    // The derived-key check is case-insensitive (matching is_derived_provider_model_key).
    // A record with mixed-case variants must have those keys stripped.
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Brain",
            "provider": "databricks_v2",
            "env_vars": {
                "buzz_agent_provider": "databricks",
                "Buzz_Agent_Model": "goose-claude-4-6-sonnet",
                "DATABRICKS_HOST": "https://dbc.example.com"
            }
        }]),
    );

    reconcile_databricks_v1_to_v2_in_file(
        &dir.path().join("agents/managed-agents.json"),
        /*rewrite_v1_provider=*/ true,
    );

    let records = read_agents_json(dir.path());
    let env_vars = &records[0]["env_vars"];
    // Mixed-case derived keys must be stripped.
    assert!(env_vars.get("buzz_agent_provider").is_none());
    assert!(env_vars.get("Buzz_Agent_Model").is_none());
    // Non-derived key preserved.
    assert_eq!(env_vars["DATABRICKS_HOST"], "https://dbc.example.com");
}
