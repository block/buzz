use std::collections::BTreeMap;

use super::{
    resolve_effective_model_provider, strip_empty_env_vars, validate_global_config,
    GlobalAgentConfig,
};
use crate::managed_agents::{BackendKind, ManagedAgentRecord, PersonaRecord, RespondTo};

fn config_with_env(pairs: &[(&str, &str)]) -> GlobalAgentConfig {
    GlobalAgentConfig {
        env_vars: pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        ..Default::default()
    }
}

// ── validate_global_config ────────────────────────────────────────────────────

#[test]
fn validate_accepts_valid_env_vars() {
    let config = config_with_env(&[("ANTHROPIC_API_KEY", "sk-test"), ("MY_CUSTOM_KEY", "value")]);
    assert!(validate_global_config(&config).is_ok());
}

#[test]
fn validate_rejects_reserved_key() {
    let config = config_with_env(&[("BUZZ_PRIVATE_KEY", "should-not-be-settable")]);
    let err = validate_global_config(&config).unwrap_err();
    assert!(
        err.contains("reserved"),
        "expected reserved-key error, got: {err}"
    );
}

#[test]
fn validate_rejects_derived_provider_model_key_goose_provider() {
    let config = config_with_env(&[("GOOSE_PROVIDER", "anthropic")]);
    let err = validate_global_config(&config).unwrap_err();
    assert!(
        err.contains("structured provider/model fields"),
        "expected derived-key error, got: {err}"
    );
}

#[test]
fn validate_rejects_derived_key_goose_model() {
    let config = config_with_env(&[("GOOSE_MODEL", "claude-opus-4")]);
    let err = validate_global_config(&config).unwrap_err();
    assert!(
        err.contains("structured provider/model fields"),
        "got: {err}"
    );
}

#[test]
fn validate_rejects_derived_key_buzz_agent_provider() {
    let config = config_with_env(&[("BUZZ_AGENT_PROVIDER", "anthropic")]);
    let err = validate_global_config(&config).unwrap_err();
    assert!(
        err.contains("structured provider/model fields"),
        "got: {err}"
    );
}

#[test]
fn validate_rejects_malformed_key() {
    let config = config_with_env(&[("has spaces", "val")]);
    let err = validate_global_config(&config).unwrap_err();
    assert!(
        err.contains("must match"),
        "expected malformed-key error, got: {err}"
    );
}

#[test]
fn validate_ignores_empty_values_for_reserved_key_check() {
    // A reserved key with an EMPTY value is a no-op (stripped at save time).
    // validate_global_config skips empty-value entries so it does not reject
    // an empty clear for a key that happens to share a name with a reserved key.
    let config = config_with_env(&[("BUZZ_PRIVATE_KEY", "")]);
    // Strip is done inside validate — empty values are stripped before checking.
    assert!(
        validate_global_config(&config).is_ok(),
        "empty value for reserved key should be treated as unset"
    );
}

// ── strip_empty_env_vars ──────────────────────────────────────────────────────

#[test]
fn strip_removes_empty_values_only() {
    let mut config = config_with_env(&[("KEY_A", "value"), ("KEY_B", ""), ("KEY_C", "other")]);
    strip_empty_env_vars(&mut config);
    assert_eq!(config.env_vars.len(), 2);
    assert!(config.env_vars.contains_key("KEY_A"));
    assert!(
        !config.env_vars.contains_key("KEY_B"),
        "empty value must be stripped"
    );
    assert!(config.env_vars.contains_key("KEY_C"));
}

#[test]
fn strip_is_idempotent_on_all_non_empty() {
    let mut config = config_with_env(&[("KEY_A", "v1"), ("KEY_B", "v2")]);
    let original = config.env_vars.clone();
    strip_empty_env_vars(&mut config);
    assert_eq!(config.env_vars, original);
}

// ── GlobalAgentConfig defaults ────────────────────────────────────────────────

#[test]
fn default_config_is_all_none_empty() {
    let config = GlobalAgentConfig::default();
    assert!(config.env_vars.is_empty());
    assert!(config.provider.is_none());
    assert!(config.model.is_none());
}

#[test]
fn roundtrip_serialization() {
    let config = GlobalAgentConfig {
        env_vars: BTreeMap::from([("ANTHROPIC_API_KEY".to_string(), "sk-test".to_string())]),
        provider: Some("anthropic".to_string()),
        model: Some("claude-opus-4".to_string()),
    };
    let json = serde_json::to_string(&config).expect("serialize");
    let back: GlobalAgentConfig = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(config, back);
}

#[test]
fn default_global_config_serializes_all_fields() {
    // IPC contract: the frontend TS type declares env_vars/provider/model as
    // non-optional. A bare `{}` (old skip_serializing_if behaviour) caused an
    // `Object.entries` crash on the undefined value. All three fields must
    // always be present in the serialized form.
    let config = GlobalAgentConfig::default();
    let json = serde_json::to_string(&config).expect("serialize");
    assert!(
        json.contains("\"env_vars\""),
        "serialized JSON must always include env_vars; got: {json}"
    );
    assert!(
        json.contains("\"provider\""),
        "serialized JSON must always include provider; got: {json}"
    );
    assert!(
        json.contains("\"model\""),
        "serialized JSON must always include model; got: {json}"
    );
}

// ── resolve_effective_model_provider ─────────────────────────────────────────

fn bare_record() -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: "agent".to_string(),
        name: "Agent".to_string(),
        persona_id: None,
        private_key_nsec: "".to_string(),
        auth_tag: None,
        relay_url: "ws://localhost:3000".to_string(),
        avatar_url: None,
        acp_command: "buzz-acp".to_string(),
        agent_command: "goose".to_string(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: "".to_string(),
        turn_timeout_seconds: 300,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        mcp_toolsets: None,
        env_vars: BTreeMap::new(),
        start_on_app_launch: false,
        runtime_pid: None,
        backend: BackendKind::Local,
        backend_agent_id: None,
        provider_binary_path: None,
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
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        relay_mesh: None,
        auto_restart_on_config_change: false,
    }
}

fn persona(id: &str, model: Option<&str>, provider: Option<&str>) -> PersonaRecord {
    PersonaRecord {
        id: id.to_string(),
        display_name: "Test Persona".to_string(),
        avatar_url: None,
        system_prompt: "".to_string(),
        runtime: None,
        model: model.map(str::to_string),
        provider: provider.map(str::to_string),
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: BTreeMap::new(),
        created_at: "".to_string(),
        updated_at: "".to_string(),
    }
}

/// Tier 1 — agent record wins: record has explicit model/provider; they must
/// outrank both the linked persona and the global defaults. Fails against any
/// implementation that prefers global or persona over the record.
#[test]
fn resolve_agent_record_wins_over_persona_and_global() {
    let mut record = bare_record();
    record.persona_id = Some("p1".to_string());
    record.model = Some("record-model".to_string());
    record.provider = Some("record-provider".to_string());
    let personas = vec![persona(
        "p1",
        Some("persona-model"),
        Some("persona-provider"),
    )];
    let global = GlobalAgentConfig {
        model: Some("global-model".to_string()),
        provider: Some("global-provider".to_string()),
        ..Default::default()
    };

    let (model, provider) = resolve_effective_model_provider(&record, &personas, &global);

    assert_eq!(model, Some("record-model"), "record model must win");
    assert_eq!(
        provider,
        Some("record-provider"),
        "record provider must win"
    );
}

/// Tier 2 — persona fallback: record has no model/provider; the linked
/// persona's values must be used. Fails against an implementation that skips
/// persona lookup and returns global or None directly.
#[test]
fn resolve_persona_fallback_when_record_has_none() {
    let mut record = bare_record();
    record.persona_id = Some("p1".to_string());
    // record.model and record.provider are None
    let personas = vec![persona(
        "p1",
        Some("persona-model"),
        Some("persona-provider"),
    )];
    let global = GlobalAgentConfig {
        model: Some("global-model".to_string()),
        provider: Some("global-provider".to_string()),
        ..Default::default()
    };

    let (model, provider) = resolve_effective_model_provider(&record, &personas, &global);

    assert_eq!(
        model,
        Some("persona-model"),
        "persona model must be used when record has none"
    );
    assert_eq!(
        provider,
        Some("persona-provider"),
        "persona provider must be used when record has none"
    );
}

/// Tier 3 — global fallback: record and persona both have no model/provider;
/// global defaults must fill in. This is the core bug Fix 1 addresses — a
/// global-only agent was Ready per readiness but spawned without model/provider.
/// Fails against the pre-fix runtime.rs spawn path that read only record.model.
#[test]
fn resolve_global_fallback_when_record_and_persona_have_none() {
    let mut record = bare_record();
    record.persona_id = Some("p1".to_string());
    // record.model / provider = None; persona.model / provider = None
    let personas = vec![persona("p1", None, None)];
    let global = GlobalAgentConfig {
        model: Some("global-model".to_string()),
        provider: Some("global-provider".to_string()),
        ..Default::default()
    };

    let (model, provider) = resolve_effective_model_provider(&record, &personas, &global);

    assert_eq!(
        model,
        Some("global-model"),
        "global model must be used when record and persona have none"
    );
    assert_eq!(
        provider,
        Some("global-provider"),
        "global provider must be used when record and persona have none"
    );
}

/// Tier 4 — no persona linked: record.persona_id is None, record has no
/// model/provider; global defaults must still fill in (persona lookup skipped).
#[test]
fn resolve_global_fallback_when_no_persona_linked() {
    let record = bare_record(); // persona_id = None, model/provider = None
    let personas: Vec<PersonaRecord> = vec![];
    let global = GlobalAgentConfig {
        model: Some("global-model".to_string()),
        provider: Some("global-provider".to_string()),
        ..Default::default()
    };

    let (model, provider) = resolve_effective_model_provider(&record, &personas, &global);

    assert_eq!(model, Some("global-model"));
    assert_eq!(provider, Some("global-provider"));
}

/// All-None: no source provides model/provider → both must be None.
/// Guards against a resolver that synthesizes phantom defaults.
#[test]
fn resolve_all_none_when_no_source_provides_values() {
    let record = bare_record(); // persona_id = None, model/provider = None
    let personas: Vec<PersonaRecord> = vec![];
    let global = GlobalAgentConfig::default(); // model/provider = None

    let (model, provider) = resolve_effective_model_provider(&record, &personas, &global);

    assert_eq!(
        model, None,
        "must return None when no source provides a model"
    );
    assert_eq!(
        provider, None,
        "must return None when no source provides a provider"
    );
}

/// Partial tier — record has model but not provider; persona has provider but
/// not model; global has both. Each field resolves independently through the
/// three-tier chain.
#[test]
fn resolve_each_field_resolves_independently_through_tiers() {
    let mut record = bare_record();
    record.persona_id = Some("p1".to_string());
    record.model = Some("record-model".to_string());
    // record.provider = None → falls through to persona
    let personas = vec![persona("p1", None, Some("persona-provider"))];
    // persona.model = None → global fills model if record also had none, but
    // record has model here so global is not needed for model.
    let global = GlobalAgentConfig {
        model: Some("global-model".to_string()),
        provider: Some("global-provider".to_string()),
        ..Default::default()
    };

    let (model, provider) = resolve_effective_model_provider(&record, &personas, &global);

    assert_eq!(model, Some("record-model"), "record wins for model");
    assert_eq!(
        provider,
        Some("persona-provider"),
        "persona wins for provider when record has none"
    );
}

// ── IPC serialization ─────────────────────────────────────────────────────────

/// A fully-populated `GlobalAgentConfig` must round-trip through JSON without
/// loss.
#[test]
fn populated_global_config_round_trips() {
    let original = GlobalAgentConfig {
        env_vars: [("ANTHROPIC_API_KEY".to_string(), "sk-test".to_string())]
            .into_iter()
            .collect(),
        provider: Some("anthropic".to_string()),
        model: Some("claude-opus-4-5".to_string()),
    };
    let json = serde_json::to_string(&original).expect("serialization must not fail");
    let decoded: GlobalAgentConfig =
        serde_json::from_str(&json).expect("deserialization must not fail");
    assert_eq!(
        decoded, original,
        "populated config must round-trip losslessly"
    );
}

// ── record_agent_command runtime resolution (regression) ─────────────────────

/// When a record carries `runtime: Some("claude")` and the linked persona has
/// `runtime: Some("goose")`, `record_agent_command` must use the RECORD runtime
/// (`"claude-agent-acp"`) — not the persona runtime (`"goose"`).
///
/// This is the invariant that `collect_respawn_candidates` / the under-lock
/// re-check in `restart_setup_listener_agent` rely on: the NotReady→Ready
/// evaluation must use the runtime the agent actually spawns with.
#[test]
fn record_runtime_wins_over_persona_runtime_for_command_resolution() {
    let mut record = bare_record();
    record.runtime = Some("claude".to_string());
    record.persona_id = Some("p1".to_string());

    let persona = PersonaRecord {
        id: "p1".to_string(),
        display_name: "Goose persona".to_string(),
        avatar_url: None,
        system_prompt: "".to_string(),
        runtime: Some("goose".to_string()),
        model: None,
        provider: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: BTreeMap::new(),
        created_at: "".to_string(),
        updated_at: "".to_string(),
    };

    let cmd = crate::managed_agents::record_agent_command(&record, &[persona]);

    // record.runtime = "claude" → primary command is "claude-agent-acp"
    // (NOT the persona runtime "goose" → "goose")
    assert_eq!(
        cmd, "claude-agent-acp",
        "record runtime must override persona runtime in command resolution"
    );
}
