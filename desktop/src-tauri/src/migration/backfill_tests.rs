use super::backfill_standalone_agents_in_dir;
use crate::managed_agents::spawn_hash::spawn_config_hash;
use crate::managed_agents::{AgentDefinition, ManagedAgentRecord};
use crate::migration::test_support::{read_agents_json, write_agents_json};
use std::path::Path;

fn standalone_agent_json(name: &str, pubkey: &str, prompt: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "pubkey": pubkey,
        "relay_url": "ws://localhost:3000",
        "acp_command": "buzz-acp",
        "agent_command": "goose",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 320,
        "parallelism": 4,
        "system_prompt": prompt,
        "model": "gpt-x",
        "provider": "openai",
        "respond_to": "anyone",
        "env_vars": { "API_KEY": "secret" },
        "start_on_app_launch": true,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "last_started_at": null,
        "last_stopped_at": null,
        "last_exit_code": null,
        "last_error": null
    })
}

fn load_typed(dir: &Path) -> Vec<ManagedAgentRecord> {
    let content = std::fs::read_to_string(dir.join("agents").join("managed-agents.json")).unwrap();
    serde_json::from_str(&content).unwrap()
}

fn base(dir: &Path) -> std::path::PathBuf {
    dir.join("agents")
}

#[test]
fn backfill_links_standalone_agent_to_manufactured_definition() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "a".repeat(64);
    write_agents_json(
        dir.path(),
        &serde_json::json!([standalone_agent_json(
            "Solo",
            &pubkey,
            Some("You are Solo.")
        )]),
    );

    let backfilled = backfill_standalone_agents_in_dir(&base(dir.path())).unwrap();
    assert_eq!(backfilled, 1);

    let records = load_typed(dir.path());
    assert_eq!(records.len(), 2, "instance + manufactured definition");

    let instance = records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    assert_eq!(instance.persona_id.as_deref(), Some(pubkey.as_str()));
    assert!(instance.persona_source_version.is_some());

    let definition = records.iter().find(|r| r.pubkey.is_empty()).unwrap();
    assert_eq!(definition.slug.as_deref(), Some(pubkey.as_str()));
    assert_eq!(definition.system_prompt.as_deref(), Some("You are Solo."));
    assert_eq!(definition.model.as_deref(), Some("gpt-x"));
    // Env COPIED (B5 pin): later instances inherit a working config.
    assert_eq!(
        definition.env_vars.get("API_KEY").map(String::as_str),
        Some("secret")
    );
    // Instance quad copied up as the definition's defaults.
    assert_eq!(definition.definition_respond_to.as_deref(), Some("anyone"));
    assert_eq!(definition.definition_parallelism, Some(4));

    // The recorded version matches the definition's actual content hash —
    // the drift badge starts clean.
    let view = definition.to_definition_view().unwrap();
    let expected = crate::managed_agents::persona_events::persona_content_hash(
        &crate::managed_agents::persona_events::persona_event_content(&view),
    );
    assert_eq!(
        instance.persona_source_version.as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn backfilled_definition_carries_prompt_present_even_if_empty() {
    // LOAD-BEARING (B5 gates): old readers hard-fail on an absent prompt. A
    // prompt-less backfilled definition would leave a wiped old device with
    // no heal source, permanently. `AgentDefinition.system_prompt` is a plain
    // String and the outbound projection wraps it in `Some` unconditionally
    // — this row pins that chain against refactors.
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "b".repeat(64);
    write_agents_json(
        dir.path(),
        &serde_json::json!([standalone_agent_json("NoPrompt", &pubkey, None)]),
    );

    backfill_standalone_agents_in_dir(&base(dir.path())).unwrap();

    let records = load_typed(dir.path());
    let definition = records.iter().find(|r| r.pubkey.is_empty()).unwrap();
    let view: AgentDefinition = definition.to_definition_view().unwrap();
    assert_eq!(view.system_prompt, "", "empty, not absent");
    let content = crate::managed_agents::persona_events::persona_event_content(&view);
    assert_eq!(
        content.system_prompt.as_deref(),
        Some(""),
        "wire projection must carry Some(\"\") — the old-reader heal source"
    );
}

#[test]
fn backfill_of_promptless_record_keeps_spawn_hash_stable() {
    // B5 hash row 2: pre-backfill the record hashes prompt None; post-backfill
    // the prospective re-snapshot pulls Some("") from the manufactured
    // definition. The spawn layer treats an empty prompt as no prompt (env
    // absent either way), so the hash must not move — otherwise every
    // prompt-less standalone agent lights the restart badge on upgrade.
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "c".repeat(64);
    write_agents_json(
        dir.path(),
        &serde_json::json!([standalone_agent_json("NoPrompt", &pubkey, None)]),
    );

    let pre_records = load_typed(dir.path());
    let pre_instance = pre_records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    let hash_before = spawn_config_hash(pre_instance, &[], "wss://ws.example", &Default::default());

    backfill_standalone_agents_in_dir(&base(dir.path())).unwrap();

    let post_records = load_typed(dir.path());
    let post_instance = post_records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    let personas: Vec<AgentDefinition> = post_records
        .iter()
        .filter_map(|r| r.to_definition_view())
        .collect();
    let hash_after = spawn_config_hash(
        post_instance,
        &personas,
        "wss://ws.example",
        &Default::default(),
    );

    assert_eq!(
        hash_before, hash_after,
        "backfill must not flip the restart badge for prompt-less agents"
    );
}

#[test]
fn backfill_of_prompted_record_keeps_spawn_hash_stable() {
    // The general no-behavior-change rail: a standalone agent WITH config
    // must also hash identically across backfill (the definition snapshots
    // the record's own values, so the re-snapshot writes back what is
    // already there).
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "d".repeat(64);
    write_agents_json(
        dir.path(),
        &serde_json::json!([standalone_agent_json(
            "Solo",
            &pubkey,
            Some("You are Solo.")
        )]),
    );

    let pre_records = load_typed(dir.path());
    let pre_instance = pre_records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    let hash_before = spawn_config_hash(pre_instance, &[], "wss://ws.example", &Default::default());

    backfill_standalone_agents_in_dir(&base(dir.path())).unwrap();

    let post_records = load_typed(dir.path());
    let post_instance = post_records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    let personas: Vec<AgentDefinition> = post_records
        .iter()
        .filter_map(|r| r.to_definition_view())
        .collect();
    let hash_after = spawn_config_hash(
        post_instance,
        &personas,
        "wss://ws.example",
        &Default::default(),
    );

    assert_eq!(hash_before, hash_after);
}

#[test]
fn second_run_is_a_no_op_and_preserves_pristine_backup() {
    // B5 gates: double-run idempotence + create-if-absent .bak. Run 1
    // migrates; run 2 must change nothing and must NOT clobber the pristine
    // pre-migration backup with a half-migrated snapshot.
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "e".repeat(64);
    write_agents_json(
        dir.path(),
        &serde_json::json!([standalone_agent_json("Solo", &pubkey, Some("P"))]),
    );
    let pristine = std::fs::read_to_string(base(dir.path()).join("managed-agents.json")).unwrap();

    assert_eq!(
        backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
        1
    );
    let after_first =
        std::fs::read_to_string(base(dir.path()).join("managed-agents.json")).unwrap();

    assert_eq!(
        backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
        0,
        "second run is a no-op"
    );
    let after_second =
        std::fs::read_to_string(base(dir.path()).join("managed-agents.json")).unwrap();
    assert_eq!(after_first, after_second, "store untouched by re-run");

    let bak =
        std::fs::read_to_string(base(dir.path()).join("managed-agents.json.pre-backfill.bak"))
            .unwrap();
    assert_eq!(
        bak, pristine,
        "backup is the PRE-migration state, never clobbered"
    );
}

#[test]
fn definitions_and_linked_records_are_untouched() {
    // Already-linked instances and existing definitions pass through
    // byte-identical; a store with nothing to backfill takes no backup.
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "f".repeat(64);
    let mut linked = standalone_agent_json("Linked", &pubkey, Some("P"));
    linked["persona_id"] = serde_json::json!("some-definition");
    write_agents_json(dir.path(), &serde_json::json!([linked]));

    assert_eq!(
        backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
        0
    );
    assert!(
        !base(dir.path())
            .join("managed-agents.json.pre-backfill.bak")
            .exists(),
        "no work, no backup"
    );
    let records = read_agents_json(dir.path());
    assert_eq!(records.len(), 1, "nothing manufactured");
}

#[test]
fn slug_collision_fails_loudly_per_record_and_continues() {
    // A pre-existing definition improbably slugged as an agent's pubkey:
    // that record is skipped (logged), the rest proceed.
    let dir = tempfile::tempdir().unwrap();
    let colliding = "1".repeat(64);
    let clean = "2".repeat(64);
    let mut definition = standalone_agent_json("Def", "", Some("P"));
    definition["slug"] = serde_json::json!(colliding.clone());
    definition["pubkey"] = serde_json::json!("");
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition,
            standalone_agent_json("Collides", &colliding, Some("P")),
            standalone_agent_json("Clean", &clean, Some("P")),
        ]),
    );

    assert_eq!(
        backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
        1,
        "collision skipped, clean record backfilled"
    );
    let records = load_typed(dir.path());
    let collided = records.iter().find(|r| r.pubkey == colliding).unwrap();
    assert_eq!(collided.persona_id, None, "collided record left standalone");
    let clean_rec = records.iter().find(|r| r.pubkey == clean).unwrap();
    assert_eq!(clean_rec.persona_id.as_deref(), Some(clean.as_str()));
}
