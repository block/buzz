use crate::managed_agents::known_acp_runtime;

#[test]
fn marker_entry_is_namespaced_by_instance_id() {
    // The spawn stamp and the sweep matcher must produce identical bytes;
    // both go through sprout_marker_entry, so this pins the on-the-wire
    // format and guards against a dev build (`...app.dev`) matching a
    // release build's (`...app`) agents.
    assert_eq!(
        super::sprout_marker_entry("xyz.block.sprout.app"),
        b"SPROUT_MANAGED_AGENT=xyz.block.sprout.app".to_vec()
    );
    assert_ne!(
        super::sprout_marker_entry("xyz.block.sprout.app"),
        super::sprout_marker_entry("xyz.block.sprout.app.dev")
    );
}

#[test]
fn sprout_agent_has_mcp_hooks() {
    let p = known_acp_runtime("sprout-agent").expect("should resolve");
    assert!(p.mcp_hooks);
    assert_eq!(p.mcp_command, Some("sprout-dev-mcp"));
}

#[test]
fn databricks_defaults_empty_in_oss_build() {
    // OSS (and normal test) builds set neither SPROUT_BUILD_DATABRICKS_*,
    // so nothing is baked in and no DATABRICKS_* is injected on spawn.
    assert!(super::build_databricks_defaults().is_empty());
}

#[test]
fn sprout_agent_resolved_via_path() {
    assert!(known_acp_runtime("/usr/local/bin/sprout-agent").is_some_and(|p| p.mcp_hooks));
}

#[test]
fn goose_has_no_mcp_hooks() {
    let p = known_acp_runtime("goose").expect("should resolve");
    assert!(!p.mcp_hooks);
    assert_eq!(p.mcp_command, None);
}

#[test]
fn unknown_command_returns_none() {
    assert!(known_acp_runtime("custom-agent").is_none());
}

// ── build_respond_to_env tests ───────────────────────────────────────

use super::build_respond_to_env;
use crate::managed_agents::types::{ManagedAgentRecord, RespondTo};

/// Construct a minimal record fixture for env-building tests. Only the
/// fields read by `build_respond_to_env` matter here.
fn fixture(
    respond_to: RespondTo,
    allowlist: Vec<String>,
    auth_tag: Option<String>,
) -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: "p".into(),
        name: "n".into(),
        persona_id: None,
        private_key_nsec: "nsec1fake".into(),
        auth_tag,
        relay_url: "ws://localhost:3000".into(),
        avatar_url: None,
        acp_command: "sprout-acp".into(),
        agent_command: "goose".into(),
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 320,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        mcp_toolsets: None,
        env_vars: std::collections::BTreeMap::new(),
        start_on_app_launch: false,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        persona_pack_path: None,
        persona_name_in_pack: None,
        created_at: "now".into(),
        updated_at: "now".into(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        respond_to,
        respond_to_allowlist: allowlist,
        relay_mesh: None,
    }
}

#[test]
fn build_env_owner_only_sets_mode_and_removes_others() {
    let rec = fixture(RespondTo::OwnerOnly, vec![], Some("tag".into()));
    let (set, remove) = build_respond_to_env(&rec, Some("owner")).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("SPROUT_ACP_RESPOND_TO").map(String::as_str),
        Some("owner-only")
    );
    assert!(!set_map.contains_key("SPROUT_ACP_RESPOND_TO_ALLOWLIST"));
    assert!(remove.contains(&"SPROUT_ACP_RESPOND_TO_ALLOWLIST"));
    // auth_tag is present → no AGENT_OWNER fallback fires.
    assert!(remove.contains(&"SPROUT_ACP_AGENT_OWNER"));
}

#[test]
fn build_env_allowlist_sets_both_envs_and_joins() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let rec = fixture(
        RespondTo::Allowlist,
        vec![a.clone(), b.clone()],
        Some("tag".into()),
    );
    let (set, _remove) = build_respond_to_env(&rec, Some("owner")).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("SPROUT_ACP_RESPOND_TO").map(String::as_str),
        Some("allowlist")
    );
    assert_eq!(
        set_map
            .get("SPROUT_ACP_RESPOND_TO_ALLOWLIST")
            .map(String::as_str),
        Some(format!("{a},{b}").as_str()),
    );
}

#[test]
fn build_env_anyone_omits_allowlist_var() {
    let rec = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    let (set, remove) = build_respond_to_env(&rec, Some("owner")).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("SPROUT_ACP_RESPOND_TO").map(String::as_str),
        Some("anyone")
    );
    assert!(!set_map.contains_key("SPROUT_ACP_RESPOND_TO_ALLOWLIST"));
    assert!(remove.contains(&"SPROUT_ACP_RESPOND_TO_ALLOWLIST"));
}

#[test]
fn build_env_legacy_record_without_auth_tag_emits_agent_owner() {
    let rec = fixture(RespondTo::OwnerOnly, vec![], None);
    let (set, remove) = build_respond_to_env(&rec, Some("ownerhex")).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("SPROUT_ACP_AGENT_OWNER").map(String::as_str),
        Some("ownerhex")
    );
    assert!(!remove.contains(&"SPROUT_ACP_AGENT_OWNER"));
}

#[test]
fn build_env_legacy_record_without_owner_hex_removes_agent_owner() {
    // No owner available to forward → make sure we don't inherit a leaked
    // env var from the parent.
    let rec = fixture(RespondTo::OwnerOnly, vec![], None);
    let (_set, remove) = build_respond_to_env(&rec, None).unwrap();
    assert!(remove.contains(&"SPROUT_ACP_AGENT_OWNER"));
}

#[test]
fn build_env_rejects_corrupted_allowlist() {
    let rec = fixture(
        RespondTo::Allowlist,
        vec!["not-hex".into()],
        Some("tag".into()),
    );
    assert!(build_respond_to_env(&rec, Some("owner")).is_err());
}

#[test]
fn build_env_rejects_empty_allowlist_in_allowlist_mode() {
    let rec = fixture(RespondTo::Allowlist, vec![], Some("tag".into()));
    let err = build_respond_to_env(&rec, Some("owner")).unwrap_err();
    assert!(err.contains("at least one pubkey"));
}

// ── resolve_effective_prompt_model_provider tests ───────────────────

fn persona(id: &str, prompt: &str, model: Option<&str>) -> crate::managed_agents::PersonaRecord {
    persona_with_provider(id, prompt, model, None)
}

fn persona_with_provider(
    id: &str,
    prompt: &str,
    model: Option<&str>,
    provider: Option<&str>,
) -> crate::managed_agents::PersonaRecord {
    crate::managed_agents::PersonaRecord {
        id: id.to_string(),
        display_name: id.to_string(),
        avatar_url: None,
        system_prompt: prompt.to_string(),
        runtime: None,
        model: model.map(str::to_string),
        provider: provider.map(str::to_string),
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_pack: None,
        source_pack_persona_slug: None,
        env_vars: std::collections::BTreeMap::new(),
        created_at: "2026-06-09T00:00:00Z".to_string(),
        updated_at: "2026-06-09T00:00:00Z".to_string(),
    }
}

#[test]
fn linked_persona_wins_over_record_snapshot() {
    let personas = vec![persona_with_provider(
        "p1",
        "fresh",
        Some("m-fresh"),
        Some("anthropic"),
    )];
    let (prompt, model, provider) = super::resolve_effective_prompt_model_provider(
        Some("p1"),
        &personas,
        Some("stale".into()),
        Some("m-stale".into()),
    );
    assert_eq!(prompt.as_deref(), Some("fresh"));
    assert_eq!(model.as_deref(), Some("m-fresh"));
    assert_eq!(provider.as_deref(), Some("anthropic"));
}

#[test]
fn no_persona_id_falls_back_to_record() {
    let personas = vec![persona("p1", "fresh", Some("m-fresh"))];
    let (prompt, model, provider) = super::resolve_effective_prompt_model_provider(
        None,
        &personas,
        Some("record".into()),
        Some("m-record".into()),
    );
    assert_eq!(prompt.as_deref(), Some("record"));
    assert_eq!(model.as_deref(), Some("m-record"));
    assert_eq!(provider, None);
}

#[test]
fn deleted_persona_falls_back_to_record() {
    let personas = vec![persona("p1", "fresh", None)];
    let (prompt, model, provider) = super::resolve_effective_prompt_model_provider(
        Some("gone"),
        &personas,
        Some("record".into()),
        Some("m-record".into()),
    );
    assert_eq!(prompt.as_deref(), Some("record"));
    assert_eq!(model.as_deref(), Some("m-record"));
    assert_eq!(provider, None);
}

#[test]
fn persona_with_no_model_clears_stale_record_model() {
    let personas = vec![persona("p1", "fresh", None)];
    let (prompt, model, _provider) = super::resolve_effective_prompt_model_provider(
        Some("p1"),
        &personas,
        Some("stale".into()),
        Some("m-stale".into()),
    );
    assert_eq!(prompt.as_deref(), Some("fresh"));
    assert_eq!(model, None);
}

// ── runtime_metadata_env_vars tests ─────────────────────────────────────

use super::runtime_metadata_env_vars;

#[test]
fn runtime_metadata_env_vars_injects_model_and_provider() {
    let vars = runtime_metadata_env_vars(
        Some("GOOSE_MODEL"),
        Some("GOOSE_PROVIDER"),
        false,
        Some("gpt-4o"),
        Some("openai"),
    );
    assert_eq!(
        vars,
        vec![("GOOSE_MODEL", "gpt-4o"), ("GOOSE_PROVIDER", "openai")]
    );
}

#[test]
fn runtime_metadata_env_vars_skips_provider_when_locked() {
    let vars = runtime_metadata_env_vars(
        None, // claude has no model_env_var
        None, // claude has no provider_env_var
        true, // provider_locked = true
        Some("claude-opus-4-7"),
        Some("anthropic"),
    );
    assert!(vars.is_empty());
}

#[test]
fn runtime_metadata_env_vars_injects_model_even_with_acp_model_switching() {
    // sprout-agent has supports_acp_model_switching=true but we still inject
    // the model env var because ACP model switching is post-bootstrap
    let vars = runtime_metadata_env_vars(
        Some("SPROUT_AGENT_MODEL"),
        Some("SPROUT_AGENT_PROVIDER"),
        false,
        Some("goose-claude-4-6-opus"),
        Some("databricks"),
    );
    assert_eq!(
        vars,
        vec![
            ("SPROUT_AGENT_MODEL", "goose-claude-4-6-opus"),
            ("SPROUT_AGENT_PROVIDER", "databricks"),
        ]
    );
}
