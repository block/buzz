//! Tests for the import-side helpers that live in this module. The shared
//! snapshot fixtures and format-level tests live in `../tests.rs`.

use std::collections::BTreeMap;

use super::{build_agent_snapshot_import_preview, resolve_snapshot_import_environment};
use crate::managed_agents::agent_snapshot::{
    AgentSnapshot, AgentSnapshotDefinition, AgentSnapshotMemory, AgentSnapshotProfile,
    FORMAT_DISCRIMINATOR, FORMAT_VERSION,
};

/// Snapshot with the given env key declaration (and optional value hints),
/// everything else minimal.
fn snapshot_with_environment(
    environment: Vec<String>,
    environment_values: BTreeMap<String, String>,
) -> AgentSnapshot {
    AgentSnapshot {
        format: FORMAT_DISCRIMINATOR.to_string(),
        version: FORMAT_VERSION,
        definition: AgentSnapshotDefinition {
            name: "Env Agent".to_string(),
            source_is_builtin: false,
            system_prompt: None,
            runtime: None,
            model: None,
            provider: None,
            parallelism: None,
            respond_to: None,
            respond_to_allowlist: vec![],
            name_pool: vec![],
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            environment,
            environment_values,
        },
        profile: AgentSnapshotProfile {
            display_name: "Env Agent".to_string(),
            about: None,
            avatar_data_url: None,
            avatar_url: None,
        },
        memory: AgentSnapshotMemory {
            level: crate::managed_agents::agent_snapshot::MemoryLevel::None,
            entries: vec![],
        },
    }
}

#[test]
fn environment_scaffolding_creates_blank_sorted_entries() {
    let resolved = resolve_snapshot_import_environment(
        &["ZEBRA_TOKEN".to_string(), "ALPHA_ENDPOINT".to_string()],
        &BTreeMap::new(),
    );
    assert_eq!(
        resolved.keys().cloned().collect::<Vec<_>>(),
        vec!["ALPHA_ENDPOINT".to_string(), "ZEBRA_TOKEN".to_string()]
    );
    assert!(
        resolved.values().all(|v| v.is_empty()),
        "values are blank unless the producer attached an explicit non-secret hint"
    );
}

#[test]
fn environment_scaffolding_drops_reserved_and_malformed_keys() {
    let resolved = resolve_snapshot_import_environment(
        &[
            "BUZZ_PRIVATE_KEY".to_string(), // reserved — stripped at spawn anyway
            "BUZZ_AUTH_TAG".to_string(),    // reserved
            "BAD KEY".to_string(),          // malformed (space)
            "KEY=x".to_string(),            // malformed ('=' smuggling)
            "9LEADING_DIGIT".to_string(),   // malformed
            "VALID_KEY".to_string(),
        ],
        &BTreeMap::new(),
    );
    assert_eq!(
        resolved.keys().cloned().collect::<Vec<_>>(),
        vec!["VALID_KEY".to_string()]
    );
}

#[test]
fn environment_scaffolding_dedups_keys() {
    let resolved = resolve_snapshot_import_environment(
        &[
            "DUP_KEY".to_string(),
            "DUP_KEY".to_string(),
            "dup_key".to_string(), // case-insensitive dedup is NOT applied: env keys are case-sensitive
        ],
        &BTreeMap::new(),
    );
    assert_eq!(
        resolved.keys().cloned().collect::<Vec<_>>(),
        vec!["DUP_KEY".to_string(), "dup_key".to_string()]
    );
}

#[test]
fn environment_value_hint_prefills_non_secret_declared_keys() {
    // The headlining use case: a control plane exports the OpenAI-compatible
    // API route so the imported agent starts pre-wired to the right endpoint,
    // while the API key itself still has to be pasted by the owner.
    let mut values = BTreeMap::new();
    values.insert(
        "OPENAI_COMPAT_BASE_URL".to_string(),
        "https://app.inloop.studio/api/v1/brains/example".to_string(),
    );
    let resolved = resolve_snapshot_import_environment(
        &[
            "OPENAI_COMPAT_API_KEY".to_string(),
            "OPENAI_COMPAT_BASE_URL".to_string(),
        ],
        &values,
    );
    assert_eq!(
        resolved.get("OPENAI_COMPAT_BASE_URL").map(String::as_str),
        Some("https://app.inloop.studio/api/v1/brains/example")
    );
    assert_eq!(
        resolved.get("OPENAI_COMPAT_API_KEY").map(String::as_str),
        Some("")
    );
}

#[test]
fn environment_value_hints_drop_secret_named_keys() {
    // Fail closed: a producer that ships a real credential has it stripped —
    // the key is still scaffolded so the owner knows to fill it in.
    let mut values = BTreeMap::new();
    values.insert("MY_API_TOKEN".to_string(), "sk-live-token".to_string());
    values.insert("APP_PASSWORD".to_string(), "hunter2".to_string());
    let resolved = resolve_snapshot_import_environment(
        &["MY_API_TOKEN".to_string(), "APP_PASSWORD".to_string()],
        &values,
    );
    assert!(
        resolved.values().all(|v| v.is_empty()),
        "secret-class key names must never accept pre-filled values"
    );
}

#[test]
fn environment_value_hints_ignore_undeclared_keys() {
    let mut values = BTreeMap::new();
    values.insert(
        "UNDECLARED_URL".to_string(),
        "https://example.com".to_string(),
    );
    let resolved = resolve_snapshot_import_environment(&["DECLARED_KEY".to_string()], &values);
    assert_eq!(
        resolved.keys().cloned().collect::<Vec<_>>(),
        vec!["DECLARED_KEY".to_string()],
        "value hints may not smuggle in keys absent from definition.environment"
    );
}

#[test]
fn environment_value_hints_treat_empty_string_as_blank() {
    let mut values = BTreeMap::new();
    values.insert("SOME_ENDPOINT".to_string(), String::new());
    let resolved = resolve_snapshot_import_environment(&["SOME_ENDPOINT".to_string()], &values);
    assert_eq!(resolved.get("SOME_ENDPOINT").map(String::as_str), Some(""));
}

#[test]
fn preview_surfaces_only_keys_that_will_be_created() {
    let snapshot = snapshot_with_environment(
        vec![
            "NEEDED_KEY".to_string(),
            "BUZZ_PRIVATE_KEY".to_string(),
            "BAD KEY".to_string(),
        ],
        BTreeMap::new(),
    );
    let preview = build_agent_snapshot_import_preview(&snapshot);
    assert_eq!(
        preview.environment_keys,
        vec!["NEEDED_KEY".to_string()],
        "preview must show only keys the import will actually create"
    );
    assert!(
        preview.environment_prefilled.is_empty(),
        "no value hints were attached"
    );
}

#[test]
fn preview_marks_prefilled_keys() {
    let mut values = BTreeMap::new();
    values.insert(
        "OPENAI_COMPAT_BASE_URL".to_string(),
        "https://app.inloop.studio/api/v1/brains/example".to_string(),
    );
    values.insert("OPENAI_COMPAT_API_KEY".to_string(), "sk-nope".to_string());
    let snapshot = snapshot_with_environment(
        vec![
            "OPENAI_COMPAT_API_KEY".to_string(),
            "OPENAI_COMPAT_BASE_URL".to_string(),
        ],
        values,
    );
    let preview = build_agent_snapshot_import_preview(&snapshot);
    assert_eq!(
        preview.environment_prefilled,
        vec!["OPENAI_COMPAT_BASE_URL".to_string()],
        "only the non-secret hint is pre-filled; the API key stays blank"
    );
}
