//! Parity matrix for the single harness-agnostic effort projection
//! (`effort_launch_projection`, PR #4625).
//!
//! Covers, per runtime: CLEAR authority order; decisive mixed-authority;
//! `value == None` when no tier resolves; single-key emission + suppress;
//! unknown/custom-runtime ACP-sentinel fallback.

use std::collections::BTreeMap;

use super::{effort_launch_projection, effort_suppress_keys, EffortLaunch};
use crate::managed_agents::custom_harnesses::HarnessDefinition;
use crate::managed_agents::discovery::{known_acp_runtime_exact, KnownAcpRuntime};
use crate::managed_agents::types::{AgentDefinition, ManagedAgentRecord};

pub(super) const GOOSE_KEY: &str = "GOOSE_THINKING_EFFORT";
pub(super) const BUZZ_AGENT_KEY: &str = "BUZZ_AGENT_THINKING_EFFORT";
pub(super) const ACP_KEY: &str = "BUZZ_ACP_EFFORT_LEVEL";

pub(super) fn goose() -> &'static KnownAcpRuntime {
    known_acp_runtime_exact("goose").expect("goose runtime in catalog")
}
fn claude() -> &'static KnownAcpRuntime {
    known_acp_runtime_exact("claude").expect("claude runtime in catalog")
}
fn buzz_agent() -> &'static KnownAcpRuntime {
    known_acp_runtime_exact("buzz-agent").expect("buzz-agent runtime in catalog")
}

pub(super) fn record() -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: "test".to_string(),
        name: "Test Agent".to_string(),
        persona_id: None,
        private_key_nsec: "".to_string(),
        auth_tag: None,
        relay_url: "ws://localhost:3000".to_string(),
        avatar_url: None,
        description: None,
        acp_command: "buzz-acp".to_string(),
        agent_command: "goose".to_string(),
        agent_args: vec![],
        mcp_command: "".to_string(),
        turn_timeout_seconds: 300,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        env_vars: BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: crate::managed_agents::types::BackendKind::Local,
        backend_agent_id: None,
        provider_policy_pending: false,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        team_catalog_source: None,
        created_at: "".to_string(),
        updated_at: "".to_string(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: crate::managed_agents::types::RespondTo::OwnerOnly,
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
        effort_level: None,
        agent_command_override: None,
        persona_source_version: None,
        provider: None,
    }
}

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn persona(id: &str, env_vars: BTreeMap<String, String>) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: "P".to_string(),
        avatar_url: None,
        description: None,
        system_prompt: String::new(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        env_vars,
        respond_to: None,
        respond_to_allowlist: vec![],
        parallelism: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn harness_def(env: BTreeMap<String, String>) -> HarnessDefinition {
    HarnessDefinition {
        id: "custom".to_string(),
        label: "Custom".to_string(),
        command: "custom".to_string(),
        args: vec![],
        env,
        install_instructions_url: String::new(),
        install_hint: String::new(),
    }
}

/// Convenience: project with no persona/global/definition/baked tiers.
fn project_record_only(
    record: &ManagedAgentRecord,
    runtime: Option<&KnownAcpRuntime>,
) -> EffortLaunch {
    effort_launch_projection(
        record,
        runtime,
        &[],
        None,
        &BTreeMap::new(),
        None,
        &BTreeMap::new(),
    )
}

// --------------------------------------------------------------------------
// Destination key + emission strategy per runtime
// --------------------------------------------------------------------------

#[test]
fn goose_emits_only_goose_key() {
    let mut r = record();
    r.effort_level = Some("high".into());
    let launch = project_record_only(&r, Some(goose()));
    assert_eq!(launch.value.as_deref(), Some("high"));
    assert_eq!(launch.key, GOOSE_KEY);
}

#[test]
fn claude_routes_canonical_through_acp_sentinel() {
    let mut r = record();
    r.effort_level = Some("high".into());
    let launch = project_record_only(&r, Some(claude()));
    // Claude has no native key: the column is the sole authority and it emits
    // under the retained ACP-startup sentinel.
    assert_eq!(launch.value.as_deref(), Some("high"));
    assert_eq!(launch.key, ACP_KEY);
}

#[test]
fn buzz_agent_passes_raw_contract_less_value_under_native_key() {
    let mut r = record();
    // buzz-agent has no static normalization contract: a per-model value that
    // Goose would reject (e.g. "minimal") passes through raw.
    r.effort_level = Some("minimal".into());
    let launch = project_record_only(&r, Some(buzz_agent()));
    assert_eq!(launch.value.as_deref(), Some("minimal"));
    assert_eq!(launch.key, BUZZ_AGENT_KEY);
}

#[test]
fn unknown_runtime_falls_back_to_acp_sentinel() {
    let mut r = record();
    r.effort_level = Some("high".into());
    // No runtime metadata (custom/unknown adapter): preserve main's behavior —
    // canonical routes through the raw ACP sentinel path.
    let launch = project_record_only(&r, None);
    assert_eq!(launch.value.as_deref(), Some("high"));
    assert_eq!(launch.key, ACP_KEY);
}

// --------------------------------------------------------------------------
// CLEAR authority order + the decisive mixed-authority case
// --------------------------------------------------------------------------

#[test]
fn decisive_record_native_outranks_a_different_valid_column() {
    // The mixed-authority pin Thufir/Will require: a valid record-native env
    // key and a DIFFERENT valid canonical column must resolve to the
    // record-native value — reader, local, remote, and snapshot all agree.
    let mut r = record();
    r.env_vars = env(&[(GOOSE_KEY, "low")]);
    r.effort_level = Some("high".into());
    let launch = project_record_only(&r, Some(goose()));
    assert_eq!(
        launch.value.as_deref(),
        Some("low"),
        "record-native env outranks the canonical column"
    );
}

#[test]
fn canonical_column_wins_when_no_record_native() {
    // No record-native key present: the column is the next tier and wins over
    // lower tiers (here, persona).
    let mut r = record();
    r.persona_id = Some("p".into());
    r.effort_level = Some("high".into());
    let personas = vec![persona("p", env(&[(GOOSE_KEY, "low")]))];
    let launch = effort_launch_projection(
        &r,
        Some(goose()),
        &personas,
        Some("p"),
        &BTreeMap::new(),
        None,
        &BTreeMap::new(),
    );
    assert_eq!(launch.value.as_deref(), Some("high"));
}

#[test]
fn record_legacy_alias_wins_over_persona_for_goose() {
    // Record legacy `BUZZ_AGENT_THINKING_EFFORT` outranks persona for a runtime
    // whose native key differs from the legacy key.
    let mut r = record();
    r.persona_id = Some("p".into());
    r.env_vars = env(&[(BUZZ_AGENT_KEY, "max")]);
    let personas = vec![persona("p", env(&[(GOOSE_KEY, "low")]))];
    let launch = effort_launch_projection(
        &r,
        Some(goose()),
        &personas,
        Some("p"),
        &BTreeMap::new(),
        None,
        &BTreeMap::new(),
    );
    assert_eq!(launch.value.as_deref(), Some("max"));
}

#[test]
fn persona_then_global_then_definition_then_baked_fall_through() {
    // With no record tier set, each lower tier wins in order once the ones
    // above it are absent. Verify persona > global by presence.
    let mut r = record();
    r.persona_id = Some("p".into());
    let personas = vec![persona("p", env(&[(GOOSE_KEY, "high")]))];
    let global = env(&[(GOOSE_KEY, "low")]);
    let launch = effort_launch_projection(
        &r,
        Some(goose()),
        &personas,
        Some("p"),
        &global,
        None,
        &BTreeMap::new(),
    );
    assert_eq!(
        launch.value.as_deref(),
        Some("high"),
        "persona outranks global"
    );

    // Drop the persona value: global wins.
    let personas = vec![persona("p", BTreeMap::new())];
    let launch = effort_launch_projection(
        &r,
        Some(goose()),
        &personas,
        Some("p"),
        &global,
        None,
        &BTreeMap::new(),
    );
    assert_eq!(
        launch.value.as_deref(),
        Some("low"),
        "global outranks definition"
    );

    // Drop global too: definition wins.
    let def = harness_def(env(&[(GOOSE_KEY, "medium")]));
    let launch = effort_launch_projection(
        &r,
        Some(goose()),
        &personas,
        Some("p"),
        &BTreeMap::new(),
        Some(&def),
        &BTreeMap::new(),
    );
    assert_eq!(
        launch.value.as_deref(),
        Some("medium"),
        "definition outranks baked"
    );

    // Drop definition: baked build floor wins.
    let baked = env(&[(GOOSE_KEY, "off")]);
    let launch = effort_launch_projection(
        &r,
        Some(goose()),
        &personas,
        Some("p"),
        &BTreeMap::new(),
        None,
        &baked,
    );
    assert_eq!(launch.value.as_deref(), Some("off"));
}

// --------------------------------------------------------------------------
// Normalization + skip-as-absent fall-through
// --------------------------------------------------------------------------

#[test]
fn goose_alias_column_xhigh_normalizes_to_max() {
    let mut r = record();
    r.effort_level = Some("xhigh".into());
    let launch = project_record_only(&r, Some(goose()));
    assert_eq!(launch.value.as_deref(), Some("max"));
}

#[test]
fn invalid_goose_column_skips_and_falls_through_to_persona() {
    // "minimal" is invalid for Goose: it skips as absent so the persona tier
    // supplies the effective value (nondestructive switch policy relies on this).
    let mut r = record();
    r.persona_id = Some("p".into());
    r.effort_level = Some("minimal".into());
    let personas = vec![persona("p", env(&[(GOOSE_KEY, "high")]))];
    let launch = effort_launch_projection(
        &r,
        Some(goose()),
        &personas,
        Some("p"),
        &BTreeMap::new(),
        None,
        &BTreeMap::new(),
    );
    assert_eq!(launch.value.as_deref(), Some("high"));
}

#[test]
fn invalid_goose_value_with_no_lower_tier_is_none() {
    let mut r = record();
    r.effort_level = Some("minimal".into());
    let launch = project_record_only(&r, Some(goose()));
    assert_eq!(
        launch.value, None,
        "invalid canonical with no fallback → None"
    );
}

#[test]
fn no_tier_set_is_none() {
    let launch = project_record_only(&record(), Some(goose()));
    assert_eq!(launch.value, None);
}

// --------------------------------------------------------------------------
// Suppression + single-key emission (the double-authority guard)
// --------------------------------------------------------------------------

#[test]
fn suppress_covers_all_native_legacy_and_sentinel_keys() {
    let keys = effort_suppress_keys();
    assert!(keys.contains(&GOOSE_KEY), "goose native key suppressed");
    assert!(
        keys.contains(&BUZZ_AGENT_KEY),
        "buzz-agent native + legacy key suppressed"
    );
    assert!(keys.contains(&ACP_KEY), "ACP transport sentinel suppressed");
}

#[test]
fn apply_strips_every_foreign_effort_key_then_emits_one() {
    let mut r = record();
    r.effort_level = Some("high".into());
    let launch = project_record_only(&r, Some(goose()));
    let mut launch_env = env(&[
        (ACP_KEY, "stale"),
        (BUZZ_AGENT_KEY, "stale"),
        (GOOSE_KEY, "stale"),
        ("UNRELATED", "keep"),
    ]);
    launch.apply(&mut launch_env);
    assert_eq!(launch_env.get(GOOSE_KEY).map(String::as_str), Some("high"));
    assert_eq!(launch_env.get(ACP_KEY), None);
    assert_eq!(launch_env.get(BUZZ_AGENT_KEY), None);
    assert_eq!(
        launch_env.get("UNRELATED").map(String::as_str),
        Some("keep")
    );
    let effort_keys = launch_env
        .keys()
        .filter(|k| effort_suppress_keys().contains(&k.as_str()))
        .count();
    assert_eq!(effort_keys, 1, "exactly one effort key survives");
}

#[test]
fn apply_with_no_value_strips_all_effort_keys() {
    let launch = project_record_only(&record(), Some(goose()));
    assert_eq!(launch.value, None);
    let mut launch_env = env(&[(ACP_KEY, "x"), (GOOSE_KEY, "y")]);
    launch.apply(&mut launch_env);
    assert!(
        launch_env
            .keys()
            .all(|k| !effort_suppress_keys().contains(&k.as_str())),
        "no effort key remains when the projection has no value"
    );
}

#[test]
fn buzz_agent_generic_column_does_not_leak_acp_sentinel() {
    let mut r = record();
    r.env_vars = env(&[(ACP_KEY, "high")]);
    r.effort_level = Some("medium".into());
    let launch = project_record_only(&r, Some(buzz_agent()));
    assert_eq!(launch.value.as_deref(), Some("medium"));
    assert_eq!(launch.key, BUZZ_AGENT_KEY);

    let mut launch_env = r.env_vars.clone();
    launch.apply(&mut launch_env);
    assert_eq!(
        launch_env.get(ACP_KEY),
        None,
        "ACP sentinel stripped for buzz-agent"
    );
    assert_eq!(
        launch_env.get(BUZZ_AGENT_KEY).map(String::as_str),
        Some("medium")
    );
}

// --------------------------------------------------------------------------
// External review fix #2 — unknown/custom runtimes restore main's pass-through
// --------------------------------------------------------------------------

#[test]
fn unknown_runtime_does_not_suppress_user_effort_env() {
    // Regression: a custom wrapper with GOOSE_THINKING_EFFORT=high in record env
    // must reach the child unchanged. For unknown runtimes `suppress` is exactly
    // `[BUZZ_ACP_EFFORT_LEVEL]` — no foreign effort key is stripped.
    let mut r = record();
    r.env_vars = env(&[(GOOSE_KEY, "high"), ("UNRELATED", "keep")]);
    let launch = project_record_only(&r, None);
    assert_eq!(
        launch.suppress,
        vec![ACP_KEY],
        "unknown runtime suppresses only its own sentinel, never a foreign key"
    );

    let mut launch_env = r.env_vars.clone();
    launch.apply(&mut launch_env);
    assert_eq!(
        launch_env.get(GOOSE_KEY).map(String::as_str),
        Some("high"),
        "custom-wrapper effort key survives an unknown-runtime launch"
    );
    assert_eq!(
        launch_env.get("UNRELATED").map(String::as_str),
        Some("keep")
    );
}

#[test]
fn unknown_runtime_keeps_user_acp_sentinel_when_no_column() {
    // Custom adapter with hand-set sentinel and no column: sentinel carries through.
    let mut r = record();
    r.env_vars = env(&[(ACP_KEY, "low")]);
    let launch = project_record_only(&r, None);
    assert_eq!(launch.value, None);
    assert!(launch.preserve_passthrough);
    let mut launch_env = r.env_vars.clone();
    launch.apply(&mut launch_env);
    assert_eq!(
        launch_env.get(ACP_KEY).map(String::as_str),
        Some("low"),
        "hand-set sentinel survives on an unknown runtime with no column"
    );
}

#[test]
fn unknown_runtime_collapses_mixed_case_sentinel_to_canonical_when_no_column() {
    // Carl P2 (no-column): a hand-set mixed-case sentinel on a custom runtime
    // must survive AND be re-emitted under the canonical spelling. Leaving the
    // lowercase variant would hand the child a value the snapshot read misses.
    let mut r = record();
    r.env_vars = env(&[("buzz_acp_effort_level", "low")]);
    let launch = project_record_only(&r, None);
    assert_eq!(launch.value, None);
    let mut launch_env = r.env_vars.clone();
    launch.apply(&mut launch_env);
    assert_eq!(
        launch_env.get(ACP_KEY).map(String::as_str),
        Some("low"),
        "mixed-case pass-through sentinel re-emitted under canonical key"
    );
    assert_eq!(
        launch_env.get("buzz_acp_effort_level"),
        None,
        "mixed-case spelling is collapsed away"
    );
}

#[test]
fn unknown_runtime_no_column_multi_variant_preserves_windows_effective_value() {
    // Pass-3 IMPORTANT (Thufir): both case spellings of the sentinel survive the
    // case-sensitive layer merge. Rust `Command` writes in `BTreeMap` iteration
    // order into a case-folded env map (last set wins); canonical `B` sorts before
    // lowercase `b`, so the lowercase `low` is written last and wins. The carry
    // selects the LAST case-insensitive match, matching that.
    let mut r = record();
    r.env_vars = env(&[(ACP_KEY, "high"), ("buzz_acp_effort_level", "low")]);
    let launch = project_record_only(&r, None);
    assert_eq!(launch.value, None);
    assert!(launch.preserve_passthrough);
    let mut launch_env = r.env_vars.clone();
    launch.apply(&mut launch_env);
    assert_eq!(
        launch_env.get(ACP_KEY).map(String::as_str),
        Some("low"),
        "carry preserves the last-in-iteration-order value the Windows child receives"
    );
    assert_eq!(launch_env.get("buzz_acp_effort_level"), None);
    // Mutation: reverting the carry to exact-first `get_ci` selects `high`.
}

#[test]
fn unknown_runtime_column_wins_over_mixed_case_sentinel() {
    // Carl P2 (with-column): canonical column plus mixed-case sentinel. Column
    // wins; the projection strips ALL case variants of the sentinel before emit.
    let mut r = record();
    r.effort_level = Some("high".into());
    r.env_vars = env(&[("buzz_acp_effort_level", "low")]);
    let launch = project_record_only(&r, None);
    assert_eq!(launch.value.as_deref(), Some("high"));
    assert_eq!(launch.key, ACP_KEY);
    let mut launch_env = r.env_vars.clone();
    launch.apply(&mut launch_env);
    assert_eq!(
        launch_env.get(ACP_KEY).map(String::as_str),
        Some("high"),
        "column wins, emitted under canonical sentinel key"
    );
    assert_eq!(launch_env.get("buzz_acp_effort_level"), None);
    // Mutation: empty suppress set leaves `buzz_acp_effort_level=low` in child.
}

#[test]
fn unknown_runtime_column_still_emits_under_acp_sentinel() {
    // The retained compatibility emission: an unknown runtime with a canonical
    // column emits it raw under the ACP sentinel (matches the PR-body decision).
    let mut r = record();
    r.effort_level = Some("high".into());
    let launch = project_record_only(&r, None);
    assert_eq!(launch.value.as_deref(), Some("high"));
    assert_eq!(launch.key, ACP_KEY);
}

// --------------------------------------------------------------------------
// External review fix #3 — destination-vocabulary validation at projection
// --------------------------------------------------------------------------

#[test]
fn goose_off_column_skips_for_buzz_agent_destination() {
    // Regression: canonical column `off` is valid Goose but NOT a buzz-agent
    // effort. Switching a record with effort_level=off to buzz-agent must NOT
    // emit BUZZ_AGENT_THINKING_EFFORT=off — parse_thinking_effort rejects it and
    // the child exits 2. Invalid → skip as absent → no key emitted.
    let mut r = record();
    r.effort_level = Some("off".into());
    let launch = project_record_only(&r, Some(buzz_agent()));
    assert_eq!(
        launch.value, None,
        "foreign canonical `off` skipped for buzz-agent's vocabulary"
    );

    let mut launch_env = BTreeMap::new();
    launch.apply(&mut launch_env);
    assert_eq!(
        launch_env.get(BUZZ_AGENT_KEY),
        None,
        "no effort key emitted when the value is outside the destination vocabulary"
    );
}

#[test]
fn buzz_agent_minimal_column_skips_for_goose_destination() {
    // The reverse: `minimal` is a valid buzz-agent effort but invalid Goose, so
    // switching to Goose skips it as absent (already covered by normalization,
    // pinned here as the symmetric vocabulary case).
    let mut r = record();
    r.effort_level = Some("minimal".into());
    let launch = project_record_only(&r, Some(goose()));
    assert_eq!(launch.value, None);
}

#[test]
fn buzz_agent_accepts_its_own_distinct_efforts() {
    // buzz-agent keeps xhigh and max distinct (no Goose-style xhigh→max
    // collapse): both are valid and pass through unchanged.
    for v in ["xhigh", "max", "none", "minimal"] {
        let mut r = record();
        r.effort_level = Some(v.into());
        let launch = project_record_only(&r, Some(buzz_agent()));
        assert_eq!(
            launch.value.as_deref(),
            Some(v),
            "buzz-agent accepts `{v}` verbatim (no alias collapse)"
        );
    }
}

// --------------------------------------------------------------------------
// External review fix #4 — case-insensitive suppression / lookup
// --------------------------------------------------------------------------

#[test]
fn mixed_case_native_key_is_read_and_wins() {
    // Windows Command case-folds env names, so `goose_thinking_effort` is the
    // same variable as the canonical form. The tier reader must find it.
    let mut r = record();
    r.env_vars = env(&[("goose_thinking_effort", "low")]);
    r.effort_level = Some("high".into());
    let launch = project_record_only(&r, Some(goose()));
    assert_eq!(
        launch.value.as_deref(),
        Some("low"),
        "mixed-case record-native key is read and outranks the column"
    );
}

#[test]
fn duplicate_case_native_variants_resolve_to_windows_effective_value() {
    // Carl P2 (r8): both case spellings of a known native key in the record env.
    // Rust `Command` writes in `BTreeMap` order into a case-folded map; canonical
    // `GOOSE_THINKING_EFFORT` sorts before lowercase, so the lowercase `high` is
    // written last and wins. `get_ci` must select the LAST match, not exact-case.
    let mut r = record();
    r.env_vars = env(&[(GOOSE_KEY, "low"), ("goose_thinking_effort", "high")]);
    let launch = project_record_only(&r, Some(goose()));
    assert_eq!(
        launch.value.as_deref(),
        Some("high"),
        "known-runtime native lookup selects the last case variant Windows Command sets"
    );
    // Mutation: reverting `get_ci` to exact-first selects `low`.
}

#[test]
fn apply_strips_mixed_case_effort_keys() {
    // A hand-set mixed-case foreign effort key must be swept, not left to
    // shadow the projected value once Windows case-folds it at spawn.
    let mut r = record();
    r.effort_level = Some("high".into());
    let launch = project_record_only(&r, Some(goose()));

    let mut launch_env = env(&[
        ("Goose_Thinking_Effort", "stale"),
        ("buzz_acp_effort_level", "stale"),
        ("UNRELATED", "keep"),
    ]);
    launch.apply(&mut launch_env);

    // Only the canonical projected key remains; both mixed-case foreign keys
    // are gone.
    assert_eq!(launch_env.get(GOOSE_KEY).map(String::as_str), Some("high"));
    assert_eq!(launch_env.get("Goose_Thinking_Effort"), None);
    assert_eq!(launch_env.get("buzz_acp_effort_level"), None);
    assert_eq!(
        launch_env.get("UNRELATED").map(String::as_str),
        Some("keep")
    );
}

// Command-boundary strip and production-sequence tests are in the sibling module.
#[cfg(test)]
#[path = "effort_cmd_tests.rs"]
mod cmd_tests;
