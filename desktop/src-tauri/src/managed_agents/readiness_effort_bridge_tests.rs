//! Spawn-side legacy effort bridge tests (Phase 3).
//!
//! Tests for `resolve_effective_agent_env` when the runtime has `effort_normalization`.
//! Verifies: tier-first precedence, alias normalization, foreign-key stripping,
//! invalid-value skip-as-absent, and global-scope invariants.
//!
//! Included from `readiness.rs` via `#[path]`; `super::*` resolves against that module.

use std::collections::BTreeMap;

use super::*;
use crate::managed_agents::discovery::known_acp_runtime_exact;

fn goose_runtime() -> Option<&'static KnownAcpRuntime> {
    known_acp_runtime_exact("goose")
}

fn empty_global() -> crate::managed_agents::global_config::GlobalAgentConfig {
    Default::default()
}

fn make_record(
    env_vars: BTreeMap<String, String>,
) -> crate::managed_agents::types::ManagedAgentRecord {
    crate::managed_agents::types::ManagedAgentRecord {
        pubkey: "test-pubkey".to_string(),
        name: "test-agent".to_string(),
        persona_id: None,
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: None,
        acp_command: "goose-acp".to_string(),
        agent_command: "goose".to_string(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 320,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars,
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: String::new(),
        updated_at: String::new(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: Default::default(),
        respond_to_allowlist: vec![],
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
        relay_mesh: None,
    }
}

fn make_persona(
    id: &str,
    env_vars: BTreeMap<String, String>,
) -> crate::managed_agents::types::AgentDefinition {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "display_name": "test-persona",
        "system_prompt": "",
        "env_vars": env_vars,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z"
    }))
    .unwrap()
}

fn env_with(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ── Bridge activation ───────────────────────────────────────────────────────

#[test]
fn goose_record_native_effort_is_translated_and_foreign_key_stripped() {
    // Record has GOOSE_THINKING_EFFORT — bridge uses it. No legacy key present.
    let record = make_record(env_with(&[("GOOSE_THINKING_EFFORT", "high")]));
    let effective = resolve_effective_agent_env(&record, &[], goose_runtime(), &empty_global());
    assert_eq!(
        effective
            .env
            .get("GOOSE_THINKING_EFFORT")
            .map(String::as_str),
        Some("high")
    );
    // No legacy key in output.
    assert!(!effective.env.contains_key("BUZZ_AGENT_THINKING_EFFORT"));
}

#[test]
fn goose_record_legacy_effort_migrated_to_native_key() {
    // Record has only BUZZ_AGENT_THINKING_EFFORT — bridge translates it.
    let record = make_record(env_with(&[("BUZZ_AGENT_THINKING_EFFORT", "medium")]));
    let effective = resolve_effective_agent_env(&record, &[], goose_runtime(), &empty_global());
    assert_eq!(
        effective
            .env
            .get("GOOSE_THINKING_EFFORT")
            .map(String::as_str),
        Some("medium"),
        "legacy key must be translated to GOOSE_THINKING_EFFORT"
    );
    assert!(
        !effective.env.contains_key("BUZZ_AGENT_THINKING_EFFORT"),
        "legacy key must be stripped from the descriptor"
    );
}

// ── Tier-first precedence ────────────────────────────────────────────────────

#[test]
fn goose_record_native_beats_persona_native() {
    // record GOOSE_THINKING_EFFORT=high, persona GOOSE_THINKING_EFFORT=low → high wins.
    let record = make_record(env_with(&[("GOOSE_THINKING_EFFORT", "high")]));
    let persona = make_persona("p1", env_with(&[("GOOSE_THINKING_EFFORT", "low")]));
    let mut record_with_persona = record;
    record_with_persona.persona_id = Some("p1".to_string());
    let effective = resolve_effective_agent_env(
        &record_with_persona,
        &[persona],
        goose_runtime(),
        &empty_global(),
    );
    assert_eq!(
        effective
            .env
            .get("GOOSE_THINKING_EFFORT")
            .map(String::as_str),
        Some("high")
    );
}

#[test]
fn goose_record_legacy_beats_persona_native_tier_first() {
    // record BUZZ_AGENT_THINKING_EFFORT=high, persona GOOSE_THINKING_EFFORT=low
    // → tier-first: record legacy wins over persona native.
    let record = make_record(env_with(&[("BUZZ_AGENT_THINKING_EFFORT", "high")]));
    let persona = make_persona("p1", env_with(&[("GOOSE_THINKING_EFFORT", "low")]));
    let mut r2 = record;
    r2.persona_id = Some("p1".to_string());
    let effective = resolve_effective_agent_env(&r2, &[persona], goose_runtime(), &empty_global());
    assert_eq!(
        effective
            .env
            .get("GOOSE_THINKING_EFFORT")
            .map(String::as_str),
        Some("high"),
        "record legacy must beat persona native under tier-first rule"
    );
}

#[test]
fn goose_global_native_excluded_from_legacy_fallback() {
    // Only global has BUZZ_AGENT_THINKING_EFFORT (legacy at global tier).
    // Global legacy is excluded end-to-end → no effort key in output.
    let mut global = empty_global();
    global
        .env_vars
        .insert("BUZZ_AGENT_THINKING_EFFORT".to_string(), "low".to_string());
    let record = make_record(BTreeMap::new());
    let effective = resolve_effective_agent_env(&record, &[], goose_runtime(), &global);
    assert!(
        !effective.env.contains_key("GOOSE_THINKING_EFFORT"),
        "global legacy must not seed Goose effort (excluded end-to-end)"
    );
    assert!(
        !effective.env.contains_key("BUZZ_AGENT_THINKING_EFFORT"),
        "global legacy must be stripped from Goose descriptor"
    );
}

// ── Alias normalization ──────────────────────────────────────────────────────

#[test]
fn goose_alias_none_is_normalized_to_off() {
    let record = make_record(env_with(&[("GOOSE_THINKING_EFFORT", "none")]));
    let effective = resolve_effective_agent_env(&record, &[], goose_runtime(), &empty_global());
    assert_eq!(
        effective
            .env
            .get("GOOSE_THINKING_EFFORT")
            .map(String::as_str),
        Some("off"),
        "none→off alias normalization"
    );
}

#[test]
fn goose_alias_xhigh_is_normalized_to_max() {
    let record = make_record(env_with(&[("GOOSE_THINKING_EFFORT", "xhigh")]));
    let effective = resolve_effective_agent_env(&record, &[], goose_runtime(), &empty_global());
    assert_eq!(
        effective
            .env
            .get("GOOSE_THINKING_EFFORT")
            .map(String::as_str),
        Some("max"),
        "xhigh→max alias normalization"
    );
}

#[test]
fn goose_invalid_value_skip_as_absent_allows_lower_tier_to_win() {
    // Record has invalid "minimal" — skipped. Persona native "low" wins.
    let record = make_record(env_with(&[("BUZZ_AGENT_THINKING_EFFORT", "minimal")]));
    let persona = make_persona("p1", env_with(&[("GOOSE_THINKING_EFFORT", "low")]));
    let mut r2 = record;
    r2.persona_id = Some("p1".to_string());
    let effective = resolve_effective_agent_env(&r2, &[persona], goose_runtime(), &empty_global());
    assert_eq!(
        effective
            .env
            .get("GOOSE_THINKING_EFFORT")
            .map(String::as_str),
        Some("low"),
        "invalid record legacy must be skipped, persona native wins"
    );
}

// ── Foreign-key stripping (global-scope coexist invariant) ─────────────────

#[test]
fn buzz_agent_global_effort_survives_when_goose_record_effort_migrated() {
    // Global config has both BUZZ_AGENT_THINKING_EFFORT (buzz-agent's native key)
    // and GOOSE_THINKING_EFFORT (Goose's native key). A Goose agent's effective
    // descriptor must strip BUZZ_AGENT_THINKING_EFFORT (foreign to Goose) while
    // honouring GOOSE_THINKING_EFFORT from the global tier.
    //
    // Note: global GOOSE_THINKING_EFFORT is treated as a global-native tier (not
    // legacy), so it IS honoured. Global legacy (BUZZ_AGENT_THINKING_EFFORT for
    // Goose) is excluded. Only the native key appears in the Goose descriptor.
    let mut global = empty_global();
    global
        .env_vars
        .insert("BUZZ_AGENT_THINKING_EFFORT".to_string(), "high".to_string());
    global
        .env_vars
        .insert("GOOSE_THINKING_EFFORT".to_string(), "low".to_string());
    let record = make_record(BTreeMap::new());
    let effective = resolve_effective_agent_env(&record, &[], goose_runtime(), &global);
    // Global native (GOOSE_THINKING_EFFORT=low) wins.
    assert_eq!(
        effective
            .env
            .get("GOOSE_THINKING_EFFORT")
            .map(String::as_str),
        Some("low"),
        "global native GOOSE_THINKING_EFFORT must reach Goose descriptor"
    );
    // Foreign key must be stripped from Goose descriptor.
    assert!(
        !effective.env.contains_key("BUZZ_AGENT_THINKING_EFFORT"),
        "BUZZ_AGENT_THINKING_EFFORT must be stripped from Goose descriptor"
    );
}

// ── Bidirectional global coexist: each runtime sees only its own key ────────

#[test]
fn buzz_agent_global_effort_descriptor_strips_goose_native_key() {
    // Both BUZZ_AGENT_THINKING_EFFORT and GOOSE_THINKING_EFFORT are in global
    // config. A buzz-agent descriptor must contain only BUZZ_AGENT_THINKING_EFFORT;
    // GOOSE_THINKING_EFFORT (foreign to buzz-agent) must be stripped.
    // Paired with buzz_agent_global_effort_survives_when_goose_record_effort_migrated
    // to assert the global coexist invariant from both directions.
    use crate::managed_agents::discovery::known_acp_runtime_exact;
    let buzz_agent_runtime = known_acp_runtime_exact("buzz-agent");
    let mut global = empty_global();
    global
        .env_vars
        .insert("BUZZ_AGENT_THINKING_EFFORT".to_string(), "high".to_string());
    global
        .env_vars
        .insert("GOOSE_THINKING_EFFORT".to_string(), "low".to_string());
    let record = make_record(std::collections::BTreeMap::new());
    let effective = resolve_effective_agent_env(&record, &[], buzz_agent_runtime, &global);
    // buzz-agent's native key must be present.
    assert_eq!(
        effective
            .env
            .get("BUZZ_AGENT_THINKING_EFFORT")
            .map(String::as_str),
        Some("high"),
        "buzz-agent native key must survive in global coexist scenario"
    );
    // Foreign key must be stripped.
    assert!(
        !effective.env.contains_key("GOOSE_THINKING_EFFORT"),
        "GOOSE_THINKING_EFFORT must be stripped from buzz-agent descriptor"
    );
}

#[test]
fn global_coexist_both_runtimes_retain_own_key_independently() {
    // Using the same global config, build descriptors for both Goose and buzz-agent
    // and verify each sees only its own key. Proves the invariant is symmetric.
    use crate::managed_agents::discovery::known_acp_runtime_exact;
    let goose = goose_runtime();
    let buzz = known_acp_runtime_exact("buzz-agent");
    let mut global = empty_global();
    global.env_vars.insert(
        "BUZZ_AGENT_THINKING_EFFORT".to_string(),
        "medium".to_string(),
    );
    global
        .env_vars
        .insert("GOOSE_THINKING_EFFORT".to_string(), "high".to_string());
    let record = make_record(std::collections::BTreeMap::new());

    let goose_eff = resolve_effective_agent_env(&record, &[], goose, &global);
    let buzz_eff = resolve_effective_agent_env(&record, &[], buzz, &global);

    // Goose sees its own key.
    assert_eq!(
        goose_eff
            .env
            .get("GOOSE_THINKING_EFFORT")
            .map(String::as_str),
        Some("high")
    );
    assert!(!goose_eff.env.contains_key("BUZZ_AGENT_THINKING_EFFORT"));

    // buzz-agent sees its own key.
    assert_eq!(
        buzz_eff
            .env
            .get("BUZZ_AGENT_THINKING_EFFORT")
            .map(String::as_str),
        Some("medium")
    );
    assert!(!buzz_eff.env.contains_key("GOOSE_THINKING_EFFORT"));
}
