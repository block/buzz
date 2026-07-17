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
        env_vars: BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: None,
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
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
        relay_mesh: None,
    }
}

fn persona(id: &str, runtime: Option<&str>, prompt: &str) -> AgentDefinition {
    AgentDefinition {
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
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "now".into(),
        updated_at: "now".into(),
    }
}

#[test]
fn hash_is_deterministic() {
    let rec = record();
    assert_eq!(
        spawn_config_hash(&rec, &[], &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&rec, &[], &[], "wss://ws.example", &Default::default())
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
        spawn_config_hash(
            &pre,
            &personas,
            &[],
            "wss://ws.example",
            &Default::default()
        ),
        spawn_config_hash(
            &post,
            &personas,
            &[],
            "wss://ws.example",
            &Default::default()
        )
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
        spawn_config_hash(&rec, &[], &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&edited, &[], &[], "wss://ws.example", &Default::default())
    );
}

#[test]
fn record_prompt_edit_changes_hash() {
    let rec = record();
    let mut edited = record();
    edited.system_prompt = Some("Edited prompt.".into());
    assert_ne!(
        spawn_config_hash(&rec, &[], &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&edited, &[], &[], "wss://ws.example", &Default::default())
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
        spawn_config_hash(&rec, &before, &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&rec, &after, &[], "wss://ws.example", &Default::default())
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
        spawn_config_hash(&rec, &before, &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&rec, &after, &[], "wss://ws.example", &Default::default())
    );
}

#[test]
fn workspace_relay_change_trips_hash_for_blank_record_relay() {
    // A blank record relay spawns against the active workspace relay, so a
    // workspace relay change means a restart would change what runs.
    let mut rec = record();
    rec.relay_url = String::new();
    assert_ne!(
        spawn_config_hash(&rec, &[], &[], "wss://relay-a.example", &Default::default()),
        spawn_config_hash(&rec, &[], &[], "wss://relay-b.example", &Default::default())
    );
}

#[test]
fn workspace_relay_change_ignored_for_pinned_record_relay() {
    // An explicit per-agent relay pins the agent regardless of workspace, so
    // a workspace relay change must NOT badge a pinned agent.
    let rec = record();
    assert_eq!(
        spawn_config_hash(&rec, &[], &[], "wss://relay-a.example", &Default::default()),
        spawn_config_hash(&rec, &[], &[], "wss://relay-b.example", &Default::default())
    );
}

#[test]
fn respond_to_allowlist_edit_changes_hash() {
    let rec = record();
    let mut edited = record();
    edited.respond_to = RespondTo::Allowlist;
    edited.respond_to_allowlist = vec!["a".repeat(64)];
    assert_ne!(
        spawn_config_hash(&rec, &[], &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&edited, &[], &[], "wss://ws.example", &Default::default())
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
        spawn_config_hash(&rec, &[], &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&edited, &[], &[], "wss://ws.example", &Default::default())
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
        spawn_config_hash(&rec, &[], &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&edited, &[], &[], "wss://ws.example", &Default::default())
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
        spawn_config_hash(&rec, &[], &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&edited, &[], &[], "wss://ws.example", &Default::default())
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
        spawn_config_hash(&rec, &[], &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&edited, &[], &[], "wss://ws.example", &Default::default())
    );
}

#[test]
fn non_default_max_turn_duration_changes_hash() {
    let rec = record();
    let mut edited = record();
    edited.max_turn_duration_seconds = Some(42);
    assert_ne!(
        spawn_config_hash(&rec, &[], &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&edited, &[], &[], "wss://ws.example", &Default::default())
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
        spawn_config_hash(&rec, &[], &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&edited, &[], &[], "wss://ws.example", &Default::default())
    );
}

#[test]
fn resnapshot_does_not_clobber_record_quad_with_definition_absent_quad() {
    // B5 hash row 3: the prospective re-snapshot copies ONLY
    // prompt/model/provider/env from the linked definition. An instance
    // whose owner hand-set respond_to/allowlist/parallelism must
    // hash identically whether or not its definition carries a quad —
    // activation of the definition-level defaults must never reach through
    // spawn and overwrite instance state.
    let quadless_definition = vec![persona("p1", Some("goose"), "Persona prompt.")];

    let mut rec = record();
    rec.persona_id = Some("p1".into());
    rec.respond_to = RespondTo::Allowlist;
    rec.respond_to_allowlist = vec!["a".repeat(64)];
    rec.parallelism = 4;

    let mut definition_with_quad = quadless_definition.clone();
    definition_with_quad[0].respond_to = Some("anyone".into());
    definition_with_quad[0].parallelism = Some(8);

    assert_eq!(
        spawn_config_hash(
            &rec,
            &quadless_definition,
            &[],
            "wss://ws.example",
            &Default::default()
        ),
        spawn_config_hash(
            &rec,
            &definition_with_quad,
            &[],
            "wss://ws.example",
            &Default::default()
        ),
        "definition quad must not leak into the spawn hash of an existing instance"
    );
}

#[test]
fn empty_prompt_hashes_like_absent_prompt() {
    // B5 hash row 2 foundation: Some("") and None spawn identically (env var
    // absent either way), so they must hash equal — a backfilled prompt-less
    // record re-snapshots to Some("") and must not trip the badge.
    let mut absent = record();
    absent.system_prompt = None;
    let mut empty = record();
    empty.system_prompt = Some(String::new());
    assert_eq!(
        spawn_config_hash(&absent, &[], &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&empty, &[], &[], "wss://ws.example", &Default::default()),
    );
}

/// (a) A definition-runtime edit must change spawn_config_hash for a
/// materialized, override-free record — the prospective re-snapshot now
/// copies the persona's runtime onto the record before hashing.
#[test]
fn definition_runtime_edit_changes_hash_for_materialized_record() {
    let mut rec = record();
    rec.persona_id = Some("pers".into());
    rec.runtime = Some("goose".into()); // materialized runtime on instance

    let before = [persona("pers", Some("goose"), "prompt")];
    let after = [persona("pers", Some("claude"), "prompt")];
    assert_ne!(
        spawn_config_hash(&rec, &before, &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&rec, &after, &[], "wss://ws.example", &Default::default()),
        "definition runtime edit must badge a materialized, override-free instance"
    );
}

/// (c) An explicit agent_command_override (ladder step 1) must beat a
/// changed definition runtime — the badge must NOT fire for a pinned instance.
#[test]
fn agent_command_override_beats_definition_runtime_change() {
    let mut rec = record();
    rec.persona_id = Some("pers".into());
    rec.runtime = Some("goose".into()); // materialized runtime
    rec.agent_command_override = Some("goose".into()); // explicit per-instance pin

    let before = [persona("pers", Some("goose"), "prompt")];
    let after = [persona("pers", Some("claude"), "prompt")];
    assert_eq!(
        spawn_config_hash(&rec, &before, &[], "wss://ws.example", &Default::default()),
        spawn_config_hash(&rec, &after, &[], "wss://ws.example", &Default::default()),
        "explicit override must win regardless of definition runtime change"
    );
}

/// (d) When the linked definition is absent the prospective re-snapshot is
/// skipped entirely: the materialized runtime must still affect the hash.
#[test]
fn missing_definition_leaves_materialized_runtime_in_hash() {
    let mut rec = record();
    rec.persona_id = Some("missing".into());
    // Use a non-default runtime: `None` now resolves to bundled Goose, so a
    // Goose-vs-None fixture would correctly hash to the same spawn command.
    rec.runtime = Some("claude".into()); // materialized runtime

    let no_personas: &[AgentDefinition] = &[];

    let mut no_runtime = rec.clone();
    no_runtime.runtime = None;

    assert_ne!(
        spawn_config_hash(
            &rec,
            no_personas,
            &[],
            "wss://ws.example",
            &Default::default()
        ),
        spawn_config_hash(
            &no_runtime,
            no_personas,
            &[],
            "wss://ws.example",
            &Default::default()
        ),
        "materialized runtime must still affect hash when definition is absent"
    );
}

#[test]
fn effective_spawn_prompt_matches_hash_semantics() {
    // The env write and the hash share effective_spawn_prompt — this row
    // pins the helper's own semantics so a refactor of either caller cannot
    // silently diverge from the contract.
    let mut r = record();
    r.system_prompt = Some(String::new());
    assert_eq!(
        effective_spawn_prompt(&r),
        None,
        "empty collapses to absent"
    );
    r.system_prompt = Some("real".into());
    assert_eq!(effective_spawn_prompt(&r).as_deref(), Some("real"));
}
