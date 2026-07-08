use super::*;
use crate::managed_agents::types::RespondTo;
use std::collections::BTreeMap;

fn record() -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: "p".repeat(64),
        name: "agent".into(),
        persona_id: None,
        private_key_nsec: "nsec1fake".into(),
        auth_tag: None,
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
        system_prompt: Some("You are a test agent.".into()),
        model: None,
        provider: None,
        persona_source_version: None,
        mcp_toolsets: None,
        env_vars: BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
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
        last_error_code: None,
        respond_to: Default::default(),
        respond_to_allowlist: vec![],
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        relay_mesh: None,
    }
}

fn persona(id: &str, runtime: Option<&str>, prompt: &str) -> PersonaRecord {
    PersonaRecord {
        id: id.into(),
        display_name: id.into(),
        avatar_url: None,
        system_prompt: prompt.into(),
        runtime: runtime.map(str::to_string),
        model: None,
        provider: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: BTreeMap::new(),
        created_at: "now".into(),
        updated_at: "now".into(),
    }
}

#[test]
fn hash_is_deterministic() {
    let rec = record();
    assert_eq!(
        spawn_config_hash(&rec, &[], "wss://ws.example"),
        spawn_config_hash(&rec, &[], "wss://ws.example")
    );
}

#[test]
fn materializing_runtime_keeps_hash_stable() {
    // Migration cutover invariant (Phase 1A): materializing the linked
    // persona's runtime onto the record must NOT change the spawn hash —
    // otherwise every running persona-linked agent would show a spurious
    // restart badge right after migration. Pre-migration the command resolves
    // through the persona fallback; post-migration through record.runtime.
    // Same persona, same runtime, same command → same hash.
    let personas = vec![persona("p1", Some("goose"), "Persona prompt.")];

    let mut pre = record();
    pre.persona_id = Some("p1".into());

    let mut post = pre.clone();
    post.runtime = Some("goose".into());

    assert_eq!(
        spawn_config_hash(&pre, &personas, "wss://ws.example"),
        spawn_config_hash(&post, &personas, "wss://ws.example")
    );
}

#[test]
fn record_env_var_edit_changes_hash() {
    let rec = record();
    let mut edited = record();
    edited
        .env_vars
        .insert("SOME_KEY".into(), "some-value".into());
    assert_ne!(
        spawn_config_hash(&rec, &[], "wss://ws.example"),
        spawn_config_hash(&edited, &[], "wss://ws.example")
    );
}

#[test]
fn record_prompt_edit_changes_hash() {
    let rec = record();
    let mut edited = record();
    edited.system_prompt = Some("Edited prompt.".into());
    assert_ne!(
        spawn_config_hash(&rec, &[], "wss://ws.example"),
        spawn_config_hash(&edited, &[], "wss://ws.example")
    );
}

#[test]
fn persona_runtime_edit_changes_hash() {
    // The harness command resolves live personas at spawn, so a persona
    // runtime change means a restart WOULD change what runs → badge trips.
    let mut rec = record();
    rec.persona_id = Some("pers".into());
    let before = [persona("pers", Some("goose"), "prompt")];
    let after = [persona("pers", Some("claude"), "prompt")];
    assert_ne!(
        spawn_config_hash(&rec, &before, "wss://ws.example"),
        spawn_config_hash(&rec, &after, "wss://ws.example")
    );
}

#[test]
fn persona_prompt_edit_changes_hash() {
    // Start/restore re-snapshot the persona prompt onto the record right
    // before spawning, so a persona prompt edit DOES apply on a plain
    // restart → the badge must trip.
    let mut rec = record();
    rec.persona_id = Some("pers".into());
    let before = [persona("pers", Some("goose"), "old prompt")];
    let after = [persona("pers", Some("goose"), "new prompt")];
    assert_ne!(
        spawn_config_hash(&rec, &before, "wss://ws.example"),
        spawn_config_hash(&rec, &after, "wss://ws.example")
    );
}

#[test]
fn workspace_relay_change_trips_hash_for_blank_record_relay() {
    // A blank record relay spawns against the active workspace relay, so a
    // workspace relay change means a restart would change what runs.
    let mut rec = record();
    rec.relay_url = String::new();
    assert_ne!(
        spawn_config_hash(&rec, &[], "wss://relay-a.example"),
        spawn_config_hash(&rec, &[], "wss://relay-b.example")
    );
}

#[test]
fn workspace_relay_change_ignored_for_pinned_record_relay() {
    // An explicit per-agent relay pins the agent regardless of workspace, so
    // a workspace relay change must NOT badge a pinned agent.
    let rec = record();
    assert_eq!(
        spawn_config_hash(&rec, &[], "wss://relay-a.example"),
        spawn_config_hash(&rec, &[], "wss://relay-b.example")
    );
}

#[test]
fn respond_to_allowlist_edit_changes_hash() {
    let rec = record();
    let mut edited = record();
    edited.respond_to = RespondTo::Allowlist;
    edited.respond_to_allowlist = vec!["a".repeat(64)];
    assert_ne!(
        spawn_config_hash(&rec, &[], "wss://ws.example"),
        spawn_config_hash(&edited, &[], "wss://ws.example")
    );
}

#[test]
fn allowlist_ignored_when_mode_is_not_allowlist() {
    // Spawn only sets BUZZ_ACP_RESPOND_TO_ALLOWLIST in allowlist mode, so
    // editing the (dormant) list under owner-only must not badge.
    let rec = record();
    let mut edited = record();
    edited.respond_to_allowlist = vec!["a".repeat(64)];
    assert_eq!(
        spawn_config_hash(&rec, &[], "wss://ws.example"),
        spawn_config_hash(&edited, &[], "wss://ws.example")
    );
}

#[test]
fn allowlist_normalization_equivalent_edits_do_not_change_hash() {
    // The env receives the normalized list (trim/lowercase/dedup), so edits
    // that normalize to the same value must not badge.
    let mut rec = record();
    rec.respond_to = RespondTo::Allowlist;
    rec.respond_to_allowlist = vec!["a".repeat(64)];
    let mut edited = rec.clone();
    edited.respond_to_allowlist = vec![
        format!(" {} ", "A".repeat(64)), // whitespace + case
        "a".repeat(64),                  // duplicate
    ];
    assert_eq!(
        spawn_config_hash(&rec, &[], "wss://ws.example"),
        spawn_config_hash(&edited, &[], "wss://ws.example")
    );
}

#[test]
fn allowlist_content_edit_still_changes_hash() {
    let mut rec = record();
    rec.respond_to = RespondTo::Allowlist;
    rec.respond_to_allowlist = vec!["a".repeat(64)];
    let mut edited = rec.clone();
    edited.respond_to_allowlist = vec!["b".repeat(64)];
    assert_ne!(
        spawn_config_hash(&rec, &[], "wss://ws.example"),
        spawn_config_hash(&edited, &[], "wss://ws.example")
    );
}

#[test]
fn explicit_default_max_turn_duration_does_not_change_hash() {
    // Spawn writes BUZZ_ACP_MAX_TURN_DURATION with the default filled in, so
    // None → Some(default) is the same spawned value and must not badge.
    let rec = record();
    let mut edited = record();
    edited.max_turn_duration_seconds =
        Some(crate::managed_agents::types::DEFAULT_AGENT_MAX_TURN_DURATION_SECONDS);
    assert_eq!(
        spawn_config_hash(&rec, &[], "wss://ws.example"),
        spawn_config_hash(&edited, &[], "wss://ws.example")
    );
}

#[test]
fn non_default_max_turn_duration_changes_hash() {
    let rec = record();
    let mut edited = record();
    edited.max_turn_duration_seconds = Some(42);
    assert_ne!(
        spawn_config_hash(&rec, &[], "wss://ws.example"),
        spawn_config_hash(&edited, &[], "wss://ws.example")
    );
}

#[test]
fn explicit_default_toolsets_do_not_change_hash() {
    // Spawn falls BUZZ_TOOLSETS back to the default set, so None → an
    // explicit copy of the default is the same spawned value.
    let rec = record();
    let mut edited = record();
    edited.mcp_toolsets = Some(crate::managed_agents::types::DEFAULT_MCP_TOOLSETS.to_string());
    assert_eq!(
        spawn_config_hash(&rec, &[], "wss://ws.example"),
        spawn_config_hash(&edited, &[], "wss://ws.example")
    );
}

#[test]
fn non_default_toolsets_change_hash() {
    let rec = record();
    let mut edited = record();
    edited.mcp_toolsets = Some("default,canvas".to_string());
    assert_ne!(
        spawn_config_hash(&rec, &[], "wss://ws.example"),
        spawn_config_hash(&edited, &[], "wss://ws.example")
    );
}

#[test]
fn non_spawn_bookkeeping_fields_do_not_change_hash() {
    // updated_at / runtime_pid / last_* are lifecycle bookkeeping, not spawn
    // inputs — routine record saves must not trip the badge.
    let rec = record();
    let mut edited = record();
    edited.updated_at = "later".into();
    edited.runtime_pid = Some(12345);
    edited.last_started_at = Some("later".into());
    edited.last_exit_code = Some(0);
    assert_eq!(
        spawn_config_hash(&rec, &[], "wss://ws.example"),
        spawn_config_hash(&edited, &[], "wss://ws.example")
    );
}
