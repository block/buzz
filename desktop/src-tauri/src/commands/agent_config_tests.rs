//! Unit tests for `commands/agent_config.rs` (split to keep `agent_config.rs`
//! under the 1500-line file-size ratchet).
//!
//! Included via `#[path = "agent_config_tests.rs"] mod tests;` at the bottom of
//! `agent_config.rs`, so `use super::*` gives access to all items in that module.

use super::*;
use crate::managed_agents::config_bridge::types::ConfigOrigin;
use crate::managed_agents::{BackendKind, RespondTo};

use std::sync::Mutex;

static GOOSE_PATH_ROOT_LOCK: Mutex<()> = Mutex::new(());

/// Run a test body with GOOSE_PATH_ROOT set to a non-existent path so that the
/// goose config file read returns `None`. Restores the prior value on exit.
fn with_no_goose_config<T>(body: impl FnOnce() -> T) -> T {
    let _guard = GOOSE_PATH_ROOT_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let prior = std::env::var_os("GOOSE_PATH_ROOT");
    std::env::set_var("GOOSE_PATH_ROOT", "/nonexistent-buzz-test-path");
    let output = body();
    match prior {
        Some(value) => std::env::set_var("GOOSE_PATH_ROOT", value),
        None => std::env::remove_var("GOOSE_PATH_ROOT"),
    }
    output
}

fn goose_runtime() -> &'static KnownAcpRuntime {
    static RUNTIME: KnownAcpRuntime = KnownAcpRuntime {
        id: "goose",
        label: "Goose",
        commands: &["goose"],
        aliases: &[],
        avatar_url: "",
        mcp_command: None,
        mcp_hooks: false,
        underlying_cli: None,
        cli_install_commands: &[],
        cli_install_commands_windows: &[],
        adapter_install_commands: &[],
        cli_install_instructions_url: "",
        adapter_install_instructions_url: "",
        cli_install_hint: "",
        adapter_install_hint: "",
        skill_dir: None,
        supports_acp_model_switching: false,
        model_env_var: Some("GOOSE_MODEL"),
        provider_env_var: Some("GOOSE_PROVIDER"),
        provider_locked: false,
        default_env: &[],
        config_file_path: Some("~/.config/goose/config.yaml"),
        config_file_format: Some("yaml"),
        supports_acp_native_config: true,
        thinking_env_var: Some("GOOSE_THINKING_EFFORT"),
        effort_normalization: Some(&crate::managed_agents::GOOSE_EFFORT_NORMALIZATION),
        effort_accepted_values: None,
        max_tokens_env_var: Some("GOOSE_MAX_TOKENS"),
        context_limit_env_var: Some("GOOSE_CONTEXT_LIMIT"),
        max_rounds_env_var: None,
        required_normalized_fields: &["model", "provider"],
        login_hint: None,
        auth_probe_args: None,
    };
    &RUNTIME
}

fn agent_record() -> ManagedAgentRecord {
    ManagedAgentRecord {
        description: None,
        pubkey: "agent".to_string(),
        name: "Agent".to_string(),
        persona_id: Some("persona-1".to_string()),
        private_key_nsec: "".to_string(),
        auth_tag: None,
        relay_url: "ws://localhost:3000".to_string(),
        avatar_url: None,
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
        env_vars: Default::default(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: BackendKind::Local,
        backend_agent_id: None,
        provider_policy_pending: false,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "".to_string(),
        updated_at: "".to_string(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: RespondTo::OwnerOnly,
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
        team_catalog_source: None,
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

fn persona_with_model(model: &str) -> AgentDefinition {
    AgentDefinition {
        description: None,
        id: "persona-1".to_string(),
        display_name: "Persona".to_string(),
        avatar_url: None,
        system_prompt: "You are a persona.".to_string(),
        runtime: None,
        model: Some(model.to_string()),
        provider: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        env_vars: Default::default(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "".to_string(),
        updated_at: "".to_string(),
    }
}

/// A post-spawn session cache whose live model is `current_model` and whose
/// `model_overridden` flag records whether a `SwitchModel` control signal set
/// it (the live-switch signal).
fn session_cache(current_model: &str, model_overridden: bool) -> SessionConfigCache {
    SessionConfigCache {
        config_options: vec![],
        available_modes: vec![],
        available_models: vec![],
        current_model: Some(current_model.to_string()),
        model_overridden,
        goose_native_config: None,
        captured_at: "".to_string(),
    }
}

/// Definition-authoritative: a stale materialized `record.model` on a
/// linked instance must never outrank (or even be consulted against) the
/// linked persona's model. `update_managed_agent` already blocks writing
/// model/provider/prompt for linked instances, so a non-`None` value here
/// can only be leftover snapshot bytes from before a persona edit — the
/// panel must report the persona's current model, tagged `PersonaDefault`,
/// not the stale byte as `BuzzExplicit`.
#[test]
fn linked_stale_record_model_never_outranks_persona_model() {
    let mut record = agent_record();
    record.model = Some("stale-explicit-model".to_string());
    let personas = vec![persona_with_model("persona-model")];

    let surface = resolve_config_surface(
        record,
        &personas,
        Some(goose_runtime()),
        None,
        &Default::default(),
        None,
    );

    let model = surface.normalized.model.as_ref().expect("model resolved");
    assert_eq!(model.value.as_deref(), Some("persona-model"));
    assert_eq!(model.origin, ConfigOrigin::PersonaDefault);
}

/// Definition-authoritative, blank-definition case: a linked instance
/// whose persona has no model of its own must fall through to the global
/// default, tagged `GlobalDefault` — mirroring
/// `effective_config::resolve_linked`'s `None => global` arm. A stale
/// materialized record model must not shadow this fallthrough either.
#[test]
fn linked_blank_definition_model_falls_through_to_global_default() {
    let mut record = agent_record();
    record.model = Some("stale-explicit-model".to_string());
    let mut persona = persona_with_model("unused");
    persona.model = None;
    let personas = vec![persona];
    let global = crate::managed_agents::GlobalAgentConfig {
        model: Some("global-model".to_string()),
        ..Default::default()
    };

    let surface = resolve_config_surface(
        record,
        &personas,
        Some(goose_runtime()),
        None,
        &global,
        None,
    );

    let model = surface.normalized.model.as_ref().expect("model resolved");
    assert_eq!(model.value.as_deref(), Some("global-model"));
    assert_eq!(model.origin, ConfigOrigin::GlobalDefault);
}

/// A definition-less (no `persona_id`) instance's own explicit model IS
/// authoritative — the stale-record clearing above is scoped to linked
/// instances only.
#[test]
fn definition_less_explicit_record_model_keeps_buzz_explicit_origin() {
    let mut record = agent_record();
    record.persona_id = None;
    record.model = Some("explicit-model".to_string());
    let personas = vec![persona_with_model("persona-model")];

    let surface = resolve_config_surface(
        record,
        &personas,
        Some(goose_runtime()),
        None,
        &Default::default(),
        None,
    );

    let model = surface.normalized.model.as_ref().expect("model resolved");
    assert_eq!(model.value.as_deref(), Some("explicit-model"));
    assert_eq!(model.origin, ConfigOrigin::BuzzExplicit);
}

/// Part A — pending-pick: a genuine-explicit pick X with a divergent live
/// model Y but `model_overridden == false` (the live switch is not yet
/// applied — a restart is pending) must keep X as the primary and must NOT
/// surface Y as an override row. The live `acp_model` does not win. This
/// FAILS against a let-live-acp-win variant (one that dropped the
/// `model_overridden` gate), so it is not vacuous.
#[test]
fn pending_pick_keeps_explicit_x_and_does_not_surface_live_y() {
    let mut record = agent_record();
    record.persona_id = None;
    record.model = Some("model-x".to_string());
    let personas: Vec<AgentDefinition> = vec![];
    let cache = session_cache("model-y", false);

    let surface = resolve_config_surface(
        record,
        &personas,
        Some(goose_runtime()),
        Some(&cache),
        &Default::default(),
        None,
    );
    let model = surface.normalized.model.expect("model resolved");

    assert_eq!(model.value.as_deref(), Some("model-x"));
    assert_eq!(model.origin, ConfigOrigin::BuzzExplicit);
    assert_ne!(model.origin, ConfigOrigin::RuntimeOverride);
    assert_ne!(model.overridden_value.as_deref(), Some("model-y"));
}

/// W2 — genuine-explicit live switch: record.model = X, no persona,
/// `model_overridden == true`, live model = Y. The live Y must render as the
/// primary with a `RuntimeOverride` origin and X as the secondary tagged
/// `BuzzExplicit` (its true source — NOT `PersonaDefault`). FAILS against the
/// shipped no-persona early-return, which left X as primary and Y struck.
#[test]
fn genuine_explicit_live_switch_renders_y_over_x_buzz_explicit_secondary() {
    let mut record = agent_record();
    record.persona_id = None;
    record.model = Some("model-x".to_string());
    let personas: Vec<AgentDefinition> = vec![];
    let cache = session_cache("model-y", true);

    let surface = resolve_config_surface(
        record,
        &personas,
        Some(goose_runtime()),
        Some(&cache),
        &Default::default(),
        None,
    );
    let model = surface.normalized.model.expect("model resolved");

    assert_eq!(model.value.as_deref(), Some("model-y"));
    assert_eq!(model.origin, ConfigOrigin::RuntimeOverride);
    assert_eq!(model.overridden_value.as_deref(), Some("model-x"));
    assert_eq!(model.overridden_origin, Some(ConfigOrigin::BuzzExplicit));
}

/// Y==X collision: a genuine-explicit agent live-switches to the SAME value
/// it already had. There is no real divergence, so the field must be a clean
/// single value with NO secondary row and origin matching the baseline (not
/// RuntimeOverride). FAILS against a naive `return base` that would leak the
/// `AcpConfigOption` row `build_model_field` populates, and against the
/// prior implementation that stamped `RuntimeOverride` on the equal-value arm.
///
/// `with_no_goose_config` suppresses the goose config file read so that the
/// fall-through to normal resolution cannot pick up a local `~/.config/goose/config.yaml`
/// model as a spurious secondary — the test is about tier precedence, not the
/// local developer's goose install.
#[test]
fn genuine_explicit_live_switch_to_same_model_yields_clean_field() {
    let mut record = agent_record();
    record.persona_id = None;
    record.model = Some("model-x".to_string());
    let personas: Vec<AgentDefinition> = vec![];
    let cache = session_cache("model-x", true);

    let surface = with_no_goose_config(|| {
        resolve_config_surface(
            record,
            &personas,
            Some(goose_runtime()),
            Some(&cache),
            &Default::default(),
            None,
        )
    });
    let model = surface.normalized.model.expect("model resolved");

    assert_eq!(model.value.as_deref(), Some("model-x"));
    // Equal-value switch must NOT stamp RuntimeOverride — baseline origin wins.
    assert_eq!(model.origin, ConfigOrigin::BuzzExplicit);
    assert_ne!(model.origin, ConfigOrigin::RuntimeOverride);
    assert_eq!(model.overridden_value, None);
    assert_eq!(model.overridden_origin, None);
}

/// Persona parity (regression): a persona-linked agent with no explicit
/// record model that live-switches still renders the persona model as the
/// secondary tagged `PersonaDefault` — the typed-baseline change must NOT
/// regress the persona arm to a different origin.
#[test]
fn persona_linked_live_switch_keeps_persona_default_secondary() {
    let record = agent_record();
    let personas = vec![persona_with_model("persona-model")];
    let cache = session_cache("model-y", true);

    let surface = resolve_config_surface(
        record,
        &personas,
        Some(goose_runtime()),
        Some(&cache),
        &Default::default(),
        None,
    );
    let model = surface.normalized.model.expect("model resolved");

    assert_eq!(model.value.as_deref(), Some("model-y"));
    assert_eq!(model.origin, ConfigOrigin::RuntimeOverride);
    assert_eq!(model.overridden_value.as_deref(), Some("persona-model"));
    assert_eq!(model.overridden_origin, Some(ConfigOrigin::PersonaDefault));
}

/// Fix 2 regression: a global-default-only agent (no record model, no
/// persona model, but global has a model) that live-switches mid-session
/// must render the global model as the secondary tagged `GlobalDefault`.
/// Before the fix, `baseline` was `None` in the `!had_model` arm when
/// persona has no model, so `read_config_surface` had no secondary to
/// surface. Fails against pre-fix code where the baseline arm returned
/// `None` when `!had_model && persona_model.is_none() && model_overridden`.
#[test]
fn global_default_live_switch_renders_global_model_as_secondary_global_default() {
    // Record has no model, no persona, global provides the model.
    let mut record = agent_record();
    record.persona_id = None;
    // record.model = None (set by agent_record())
    let personas: Vec<AgentDefinition> = vec![];
    let cache = session_cache("model-y", true);
    let global = crate::managed_agents::GlobalAgentConfig {
        model: Some("global-model".to_string()),
        ..Default::default()
    };

    let surface = resolve_config_surface(
        record,
        &personas,
        Some(goose_runtime()),
        Some(&cache),
        &global,
        None,
    );
    let model = surface.normalized.model.expect("model resolved");

    // Live model wins as primary.
    assert_eq!(model.value.as_deref(), Some("model-y"));
    assert_eq!(model.origin, ConfigOrigin::RuntimeOverride);
    // Global model surfaces as secondary, tagged GlobalDefault.
    assert_eq!(
        model.overridden_value.as_deref(),
        Some("global-model"),
        "global model must be the override baseline secondary"
    );
    assert_eq!(
        model.overridden_origin,
        Some(ConfigOrigin::GlobalDefault),
        "override baseline origin must be GlobalDefault, not PersonaDefault or BuzzExplicit"
    );
}

// ── Snapshot constructor tests (build_inherited_tiers) ──────────────────────
//
// These test the sanitized snapshot constructor — the command-boundary
// function that builds InheritedConfigTiers from raw persona/global data.

/// Orphaned persona link: a record whose persona_id references a non-existent
/// persona should produce empty persona tiers (not a panic), and the panel
/// still renders from the record and global tiers.
#[test]
fn orphaned_persona_link_yields_empty_persona_tiers() {
    let mut record = agent_record();
    record.persona_id = Some("missing-persona".to_string());
    // No personas in the list — dangling link.
    let personas: Vec<AgentDefinition> = vec![];
    let global = crate::managed_agents::GlobalAgentConfig {
        model: Some("global-model".to_string()),
        ..Default::default()
    };

    let tiers = build_inherited_tiers(record.persona_id.as_deref(), None, &personas, &global);

    // Persona tier is empty — the orphan yields no persona inheritance.
    assert!(tiers.persona_env.is_empty());
    assert!(tiers.persona_model.is_none());
    assert!(tiers.persona_provider.is_none());
    assert!(tiers.persona_prompt.is_none());
    // Global tiers are unaffected.
    assert_eq!(tiers.global_model.as_deref(), Some("global-model"));
}

/// Reserved key in persona env is stripped by sanitization — it must never
/// reach the reader or the display surface.
#[test]
fn reserved_key_in_inherited_persona_env_is_stripped() {
    let mut persona = persona_with_model("model");
    // BUZZ_PRIVATE_KEY is a reserved key — must be stripped.
    persona
        .env_vars
        .insert("BUZZ_PRIVATE_KEY".to_string(), "nsec-secret".to_string());
    // A safe key — must survive.
    persona
        .env_vars
        .insert("GOOSE_MODEL".to_string(), "persona-model".to_string());
    let personas = vec![persona];
    let global = crate::managed_agents::GlobalAgentConfig::default();

    let tiers = build_inherited_tiers(Some("persona-1"), None, &personas, &global);

    assert!(
        !tiers.persona_env.contains_key("BUZZ_PRIVATE_KEY"),
        "reserved key must be stripped from persona env tier"
    );
    assert!(
        tiers.persona_env.contains_key("GOOSE_MODEL"),
        "safe key must survive sanitization"
    );
}

/// `sanitize_inherited_env` strips reserved keys from a definition-env-shaped
/// map. This pins the shared sanitization contract for definition_env — the
/// same function is applied to all three env tiers (persona, global, definition)
/// at the command boundary.
#[test]
fn reserved_key_in_definition_env_shaped_map_is_stripped_by_sanitize() {
    // Exercise sanitize_inherited_env directly with a definition-env-shaped map.
    let mut raw = std::collections::BTreeMap::new();
    raw.insert("BUZZ_PRIVATE_KEY".to_string(), "nsec-secret".to_string());
    raw.insert("GOOSE_MODEL".to_string(), "harness-model".to_string());

    let sanitized = sanitize_inherited_env(&raw);

    assert!(
        !sanitized.contains_key("BUZZ_PRIVATE_KEY"),
        "reserved key must be stripped by sanitize_inherited_env"
    );
    assert!(
        sanitized.contains_key("GOOSE_MODEL"),
        "safe key must survive sanitize_inherited_env"
    );
}

/// Malformed key in global env is stripped by sanitization — keys must be
/// POSIX-shaped (`[A-Za-z_][A-Za-z0-9_]*`).
#[test]
fn malformed_key_in_inherited_global_env_is_stripped() {
    let mut global = crate::managed_agents::GlobalAgentConfig::default();
    // Key with an `=` — would bypass env-var security if passed to spawn.
    global
        .env_vars
        .insert("BAD=KEY".to_string(), "value".to_string());
    // A valid key — must survive.
    global
        .env_vars
        .insert("GOOSE_PROVIDER".to_string(), "anthropic".to_string());

    let tiers = build_inherited_tiers(None, None, &[], &global);

    assert!(
        !tiers.global_env.contains_key("BAD=KEY"),
        "malformed key must be stripped from global env tier"
    );
    assert!(
        tiers.global_env.contains_key("GOOSE_PROVIDER"),
        "valid key must survive sanitization"
    );
}

// ── get_baked_build_env / is_secret_key tests ──────────────────────────

/// Build a `BakedEnvEntry` vec from a synthetic map, mirroring what
/// `get_baked_build_env()` does. Used to test masking without relying on
/// compile-time `option_env!` vars (OSS builds have empty `baked_build_env`).
fn baked_env_from_map(map: &[(&str, &str)]) -> Vec<BakedEnvEntry> {
    map.iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| {
            let masked = !super::is_safe_to_reveal(k);
            BakedEnvEntry {
                key: k.to_string(),
                value: if masked {
                    "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}".to_string()
                } else {
                    v.to_string()
                },
                masked,
            }
        })
        .collect()
}

#[test]
fn baked_env_non_secret_key_shows_real_value() {
    let entries = baked_env_from_map(&[("BUZZ_AGENT_PROVIDER", "databricks_v2")]);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].key, "BUZZ_AGENT_PROVIDER");
    assert_eq!(entries[0].value, "databricks_v2");
    assert!(!entries[0].masked);
}

#[test]
fn baked_env_api_key_is_masked() {
    let entries = baked_env_from_map(&[("ANTHROPIC_API_KEY", "sk-secret")]);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].value, "••••••");
    assert!(entries[0].masked);
}

#[test]
fn baked_env_token_key_is_masked() {
    let entries = baked_env_from_map(&[("GITHUB_TOKEN", "ghp_secret")]);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].masked);
}

#[test]
fn baked_env_secret_key_is_masked() {
    let entries = baked_env_from_map(&[("MY_DB_SECRET", "s3cr3t")]);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].masked);
}

#[test]
fn baked_env_password_key_is_masked() {
    let entries = baked_env_from_map(&[("DB_PASSWORD", "hunter2")]);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].masked);
}

#[test]
fn baked_env_empty_value_filtered_out() {
    let entries = baked_env_from_map(&[("BUZZ_AGENT_PROVIDER", "")]);
    assert!(entries.is_empty());
}

#[test]
fn baked_env_mixed_keys_correct_masking() {
    let entries = baked_env_from_map(&[
        ("BUZZ_AGENT_PROVIDER", "databricks_v2"),
        ("BUZZ_AGENT_MODEL", "goose-claude-opus-4-8"),
        ("DATABRICKS_HOST", "https://example.com"),
        ("DATABRICKS_TOKEN", "dapi-secret"),
    ]);
    assert_eq!(entries.len(), 4);

    let provider = entries
        .iter()
        .find(|e| e.key == "BUZZ_AGENT_PROVIDER")
        .unwrap();
    assert_eq!(provider.value, "databricks_v2");
    assert!(!provider.masked);

    let model = entries
        .iter()
        .find(|e| e.key == "BUZZ_AGENT_MODEL")
        .unwrap();
    assert_eq!(model.value, "goose-claude-opus-4-8");
    assert!(!model.masked);

    let host = entries.iter().find(|e| e.key == "DATABRICKS_HOST").unwrap();
    assert_eq!(host.value, "https://example.com");
    assert!(!host.masked);

    let token = entries
        .iter()
        .find(|e| e.key == "DATABRICKS_TOKEN")
        .unwrap();
    assert_eq!(token.value, "••••••");
    assert!(token.masked);
}

/// F1 picker direct-write invariant: a stale record-native `GOOSE_THINKING_EFFORT`
/// (launch-projection tier 1, ABOVE the canonical column) must not survive a
/// picker write. Setting effort `high` through the picker path both writes the
/// column and sweeps the stale alias, so the reader and the launch projection
/// both resolve `high` — not the stale `low`. Deleting the sweep in
/// `apply_picker_effort_level` re-breaks this: the projection would emit `low`.
#[test]
fn picker_write_sweeps_stale_record_native_effort_alias() {
    let mut record = agent_record();
    record
        .env_vars
        .insert("GOOSE_THINKING_EFFORT".to_string(), "low".to_string());

    super::apply_picker_effort_level(&mut record, Some("high".to_string()));

    // The stale record-native alias is gone; only the column carries the value.
    assert!(
        !record.env_vars.contains_key("GOOSE_THINKING_EFFORT"),
        "stale record-native effort alias must be swept by the picker write"
    );
    assert_eq!(record.effort_level.as_deref(), Some("high"));

    // Reader: the panel resolves the just-set value, not the stale alias.
    let surface = with_no_goose_config(|| {
        resolve_config_surface(
            record.clone(),
            &[],
            Some(goose_runtime()),
            None,
            &Default::default(),
            None,
        )
    });
    let effort = surface
        .normalized
        .thinking_effort
        .expect("picker-set effort must resolve");
    assert_eq!(effort.value.as_deref(), Some("high"));

    // Launch projection: the spawned child receives the picker value.
    let launch = crate::managed_agents::config_bridge::effort::effort_launch_projection(
        &record,
        Some(goose_runtime()),
        &[],
        None,
        &std::collections::BTreeMap::new(),
        None,
        &std::collections::BTreeMap::new(),
    );
    assert_eq!(launch.value.as_deref(), Some("high"));
}

#[test]
fn baked_env_thinking_effort_is_unmasked() {
    // BUZZ_AGENT_THINKING_EFFORT is a non-secret enum — must not be masked.
    let entries = baked_env_from_map(&[("BUZZ_AGENT_THINKING_EFFORT", "medium")]);
    assert_eq!(entries.len(), 1);
    let effort = entries
        .iter()
        .find(|e| e.key == "BUZZ_AGENT_THINKING_EFFORT")
        .unwrap();
    assert_eq!(effort.value, "medium");
    assert!(!effort.masked);
}

#[test]
fn baked_env_thinking_summary_is_unmasked() {
    // BUZZ_AGENT_THINKING_SUMMARY is a non-secret enum — must not be masked.
    let entries = baked_env_from_map(&[("BUZZ_AGENT_THINKING_SUMMARY", "detailed")]);
    assert_eq!(entries.len(), 1);
    let summary = entries
        .iter()
        .find(|e| e.key == "BUZZ_AGENT_THINKING_SUMMARY")
        .unwrap();
    assert_eq!(summary.value, "detailed");
    assert!(!summary.masked);
}

#[test]
fn baked_env_allowlist_is_case_insensitive() {
    // Known-safe keys — case-insensitive match must allow them.
    assert!(super::is_safe_to_reveal("buzz_agent_provider"));
    assert!(super::is_safe_to_reveal("BUZZ_AGENT_PROVIDER"));
    assert!(super::is_safe_to_reveal("buzz_agent_model"));
    assert!(super::is_safe_to_reveal("BUZZ_AGENT_MODEL"));
    assert!(super::is_safe_to_reveal("buzz_agent_thinking_effort"));
    assert!(super::is_safe_to_reveal("BUZZ_AGENT_THINKING_EFFORT"));
    assert!(super::is_safe_to_reveal("buzz_agent_thinking_summary"));
    assert!(super::is_safe_to_reveal("BUZZ_AGENT_THINKING_SUMMARY"));
    assert!(super::is_safe_to_reveal("databricks_host"));
    assert!(super::is_safe_to_reveal("DATABRICKS_HOST"));
    assert!(super::is_safe_to_reveal("databricks_model"));
    assert!(super::is_safe_to_reveal("DATABRICKS_MODEL"));
    // Keys NOT in the allowlist — masked regardless of naming pattern.
    assert!(!super::is_safe_to_reveal("my_api_key"));
    assert!(!super::is_safe_to_reveal("GITHUB_TOKEN"));
    assert!(!super::is_safe_to_reveal("DB_SECRET"));
    assert!(!super::is_safe_to_reveal("DB_PASSWORD"));
    // Bare names that old heuristic (contains("_TOKEN") etc.) would have missed.
    assert!(!super::is_safe_to_reveal("APIKEY"));
    assert!(!super::is_safe_to_reveal("TOKEN"));
    assert!(!super::is_safe_to_reveal("SECRET"));
    assert!(!super::is_safe_to_reveal("PASSWORD"));
    assert!(!super::is_safe_to_reveal("PRIVATE_KEY"));
    // Unknown key → masked by default.
    assert!(!super::is_safe_to_reveal("SOME_UNKNOWN_KEY"));
}

/// F3 (Desktop-parsing half): the `models` block emitted by an applied live
/// switch — taken from the post-switch snapshot in `pool.rs` — must parse to the
/// target model as current. Pairs with the pool test
/// `test_applied_switch_caches_target_model_not_pre_switch`, which proves the
/// emitted block already carries `currentModelId=model-b`.
#[test]
fn live_switch_models_from_post_switch_snapshot_parses_target_current() {
    let models = serde_json::json!({
        "currentModelId": "model-b",
        "availableModels": [{"modelId": "model-a"}, {"modelId": "model-b"}],
    });
    let (available, current) = parse_models(Some(&models));
    assert_eq!(current.as_deref(), Some("model-b"));
    assert_eq!(available.len(), 2);
}

/// F3 (Desktop-parsing half): a Null `models` block — emitted when a successful
/// switch's target response omits `models` — must parse to no current model, so
/// the pre-switch model is never revived in the cache.
#[test]
fn live_switch_null_models_parses_to_no_current_model() {
    let (available, current) = parse_models(Some(&serde_json::Value::Null));
    assert!(
        current.is_none(),
        "Null models must not surface any current model"
    );
    assert!(available.is_empty());
}
