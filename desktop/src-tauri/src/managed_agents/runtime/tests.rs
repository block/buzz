use crate::managed_agents::known_acp_runtime;

// ── buffer_contains_identifier tests ────────────────────────────────────

#[test]
fn identifier_prefix_does_not_match_longer_id() {
    // DMG identifier should NOT match inside a dev desktop's config JSON.
    let buf = br#""identifier":"xyz.block.buzz.app.dev""#;
    let id = b"xyz.block.buzz.app";
    assert!(!super::buffer_contains_identifier(buf, id));
}

#[test]
fn identifier_prefix_does_not_match_worktree_slug() {
    // Main dev identifier should NOT match inside a worktree desktop's buffer.
    let buf = br#""identifier":"xyz.block.buzz.app.dev.my-branch""#;
    let id = b"xyz.block.buzz.app.dev";
    assert!(!super::buffer_contains_identifier(buf, id));
}

#[test]
fn identifier_exact_match_with_quote_boundary() {
    // Exact match followed by closing quote — should match.
    let buf = br#""identifier":"xyz.block.buzz.app.dev""#;
    let id = b"xyz.block.buzz.app.dev";
    assert!(super::buffer_contains_identifier(buf, id));
}

#[test]
fn identifier_match_with_null_boundary() {
    // In KERN_PROCARGS2, entries are null-delimited.
    let mut buf = b"BUZZ_MANAGED_AGENT=xyz.block.buzz.app.dev".to_vec();
    buf.push(0);
    buf.extend_from_slice(b"OTHER_VAR=value");
    let id = b"xyz.block.buzz.app.dev";
    assert!(super::buffer_contains_identifier(&buf, id));
}

#[test]
fn identifier_exact_match_at_end_of_buffer() {
    // Exact match with end-of-buffer as the boundary — Thufir's case 1.
    let buf = b"xyz.block.buzz.app.dev";
    let id = b"xyz.block.buzz.app.dev";
    assert!(super::buffer_contains_identifier(buf, id));
}

#[test]
fn longer_id_matches_when_short_prefix_also_present() {
    // Searching for the longer ID finds it even when a shorter prefix token
    // appears earlier — Thufir's "longer-of-prefix must match" case.
    let mut buf = b"xyz.block.buzz.app".to_vec();
    buf.push(0);
    buf.extend_from_slice(br#""identifier":"xyz.block.buzz.app.dev""#);
    let id = b"xyz.block.buzz.app.dev";
    assert!(super::buffer_contains_identifier(&buf, id));
}

#[test]
fn identifier_empty_returns_false() {
    let buf = b"anything";
    assert!(!super::buffer_contains_identifier(buf, b""));
}

// ── marker_entry tests ──────────────────────────────────────────────────

#[test]
fn marker_entry_is_namespaced_by_instance_id() {
    // The spawn stamp and the sweep matcher must produce identical bytes;
    // both go through buzz_marker_entry, so this pins the on-the-wire
    // format and guards against a dev build (`...app.dev`) matching a
    // release build's (`...app`) agents.
    assert_eq!(
        super::buzz_marker_entry("xyz.block.buzz.app"),
        b"BUZZ_MANAGED_AGENT=xyz.block.buzz.app".to_vec()
    );
    assert_ne!(
        super::buzz_marker_entry("xyz.block.buzz.app"),
        super::buzz_marker_entry("xyz.block.buzz.app.dev")
    );
}

#[test]
fn buzz_agent_has_mcp_hooks() {
    let p = known_acp_runtime("buzz-agent").expect("should resolve");
    assert!(p.mcp_hooks);
    assert_eq!(p.mcp_command, Some("buzz-dev-mcp"));
}

#[test]
fn databricks_defaults_empty_in_oss_build() {
    // OSS (and normal test) builds set neither BUZZ_BUILD_DATABRICKS_*,
    // so nothing is baked in and no DATABRICKS_* is injected on spawn.
    assert!(super::build_databricks_defaults().is_empty());
}

#[test]
fn buzz_agent_resolved_via_path() {
    assert!(known_acp_runtime("/usr/local/bin/buzz-agent").is_some_and(|p| p.mcp_hooks));
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
        acp_command: "buzz-acp".into(),
        agent_command: "goose".into(),
        agent_command_override: None,
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
        persona_team_dir: None,
        persona_name_in_team: None,
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
        set_map.get("BUZZ_ACP_RESPOND_TO").map(String::as_str),
        Some("owner-only")
    );
    assert!(!set_map.contains_key("BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
    assert!(remove.contains(&"BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
    // auth_tag is present → no AGENT_OWNER fallback fires.
    assert!(remove.contains(&"BUZZ_ACP_AGENT_OWNER"));
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
        set_map.get("BUZZ_ACP_RESPOND_TO").map(String::as_str),
        Some("allowlist")
    );
    assert_eq!(
        set_map
            .get("BUZZ_ACP_RESPOND_TO_ALLOWLIST")
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
        set_map.get("BUZZ_ACP_RESPOND_TO").map(String::as_str),
        Some("anyone")
    );
    assert!(!set_map.contains_key("BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
    assert!(remove.contains(&"BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
}

#[test]
fn build_env_legacy_record_without_auth_tag_emits_agent_owner() {
    let rec = fixture(RespondTo::OwnerOnly, vec![], None);
    let (set, remove) = build_respond_to_env(&rec, Some("ownerhex")).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("BUZZ_ACP_AGENT_OWNER").map(String::as_str),
        Some("ownerhex")
    );
    assert!(!remove.contains(&"BUZZ_ACP_AGENT_OWNER"));
}

#[test]
fn build_env_legacy_record_without_owner_hex_removes_agent_owner() {
    // No owner available to forward → make sure we don't inherit a leaked
    // env var from the parent.
    let rec = fixture(RespondTo::OwnerOnly, vec![], None);
    let (_set, remove) = build_respond_to_env(&rec, None).unwrap();
    assert!(remove.contains(&"BUZZ_ACP_AGENT_OWNER"));
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
        source_team: None,
        source_team_persona_slug: None,
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
    // buzz-agent has supports_acp_model_switching=true but we still inject
    // the model env var because ACP model switching is post-bootstrap
    let vars = runtime_metadata_env_vars(
        Some("BUZZ_AGENT_MODEL"),
        Some("BUZZ_AGENT_PROVIDER"),
        false,
        Some("goose-claude-4-6-opus"),
        Some("databricks"),
    );
    assert_eq!(
        vars,
        vec![
            ("BUZZ_AGENT_MODEL", "goose-claude-4-6-opus"),
            ("BUZZ_AGENT_PROVIDER", "databricks"),
        ]
    );
}
