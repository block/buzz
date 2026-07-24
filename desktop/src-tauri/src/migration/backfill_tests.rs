use super::backfill_standalone_agents_in_dir;
use crate::managed_agents::spawn_hash::spawn_config_hash;
use crate::managed_agents::{
    persona_events::{persona_content_hash, persona_event_content},
    AgentDefinition, ManagedAgentRecord, DEFAULT_AGENT_PARALLELISM,
};
use crate::migration::test_support::{read_agents_json, write_agents_json};
use std::collections::BTreeMap;
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

fn folded_definition_json(
    slug: &str,
    name: &str,
    prompt: &str,
    provider: &str,
) -> serde_json::Value {
    serde_json::to_value(
        AgentDefinition {
            id: slug.to_string(),
            display_name: name.to_string(),
            avatar_url: Some("data:image/png;base64,Zm9sZGVk".to_string()),
            system_prompt: prompt.to_string(),
            runtime: Some("goose".to_string()),
            model: Some("gpt-x".to_string()),
            provider: Some(provider.to_string()),
            name_pool: vec!["Sparrow".to_string(), "Robin".to_string()],
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            env_vars: BTreeMap::from([("API_KEY".to_string(), "secret".to_string())]),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: "2025-12-01T00:00:00Z".to_string(),
            updated_at: "2025-12-02T00:00:00Z".to_string(),
        }
        .into_agent_record(),
    )
    .unwrap()
}

fn standalone_with_legacy_defaults(name: &str, pubkey: &str, prompt: &str) -> serde_json::Value {
    let mut standalone = standalone_agent_json(name, pubkey, Some(prompt));
    standalone["avatar_url"] = serde_json::json!("https://media.example/avatar.png");
    standalone["agent_command_override"] = serde_json::json!("goose");
    standalone["env_vars"] = serde_json::json!({
        "API_KEY": "instance-override",
        "INSTANCE_ONLY": "override"
    });
    standalone["respond_to"] = serde_json::json!("owner-only");
    standalone["parallelism"] = serde_json::json!(DEFAULT_AGENT_PARALLELISM);
    standalone
}

#[test]
fn backfill_adopts_matching_folded_definition_without_manufacturing_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "0".repeat(64);
    let folded_slug = "legacy-persona-id";
    let mut standalone = standalone_with_legacy_defaults("Solo", &pubkey, "You are Solo.");
    standalone
        .as_object_mut()
        .unwrap()
        .remove("agent_command_override");
    standalone["runtime"] = serde_json::json!("goose");
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            folded_definition_json(folded_slug, "Solo", "You are Solo.", "openai"),
            standalone,
        ]),
    );

    let before = load_typed(dir.path());
    let before_instance = before.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    let before_definitions: Vec<AgentDefinition> = before
        .iter()
        .filter_map(ManagedAgentRecord::to_definition_view)
        .collect();
    let hash_before = spawn_config_hash(
        before_instance,
        &before_definitions,
        &[],
        "wss://ws.example",
        &Default::default(),
    );

    let backfilled = backfill_standalone_agents_in_dir(&base(dir.path())).unwrap();
    assert_eq!(backfilled, 1);

    let records = load_typed(dir.path());
    assert_eq!(
        records.len(),
        2,
        "matching fold output should be adopted, not duplicated"
    );
    let instance = records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    assert_eq!(instance.persona_id.as_deref(), Some(folded_slug));

    let definitions: Vec<_> = records.iter().filter(|r| r.pubkey.is_empty()).collect();
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].slug.as_deref(), Some(folded_slug));
    assert_eq!(
        definitions[0].created_at, "2025-12-01T00:00:00Z",
        "adoption must preserve the existing definition"
    );
    assert_eq!(
        definitions[0].avatar_url.as_deref(),
        Some("data:image/png;base64,Zm9sZGVk")
    );
    assert_eq!(
        instance.avatar_url.as_deref(),
        Some("https://media.example/avatar.png"),
        "the instance keeps its uploaded avatar URL"
    );
    assert_eq!(
        instance.env_vars.get("INSTANCE_ONLY").map(String::as_str),
        Some("override"),
        "instance overrides remain on the instance"
    );
    assert_eq!(
        instance.env_vars.get("API_KEY").map(String::as_str),
        Some("instance-override"),
        "instance values continue to win over definition env"
    );

    let adopted = definitions[0].to_definition_view().unwrap();
    let expected_source_version = persona_content_hash(&persona_event_content(&adopted));
    assert_eq!(
        instance.persona_source_version.as_deref(),
        Some(expected_source_version.as_str())
    );
    let after_definitions: Vec<AgentDefinition> = records
        .iter()
        .filter_map(ManagedAgentRecord::to_definition_view)
        .collect();
    let hash_after = spawn_config_hash(
        instance,
        &after_definitions,
        &[],
        "wss://ws.example",
        &Default::default(),
    );
    assert_eq!(
        hash_before, hash_after,
        "adoption must keep the effective spawn configuration stable"
    );
}

#[test]
fn backfill_does_not_adopt_definition_with_different_provider() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "3".repeat(64);
    let folded_slug = "different-provider";
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            folded_definition_json(folded_slug, "Solo", "You are Solo.", "anthropic"),
            standalone_with_legacy_defaults("Solo", &pubkey, "You are Solo."),
        ]),
    );

    assert_eq!(
        backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
        1
    );
    let records = load_typed(dir.path());
    assert_eq!(
        records.len(),
        3,
        "a semantic mismatch must keep the folded definition separate"
    );
    let instance = records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    assert_eq!(instance.persona_id.as_deref(), Some(pubkey.as_str()));
    assert!(records
        .iter()
        .any(|record| record.slug.as_deref() == Some(folded_slug)));
    assert!(records
        .iter()
        .any(|record| record.slug.as_deref() == Some(pubkey.as_str())));
}

#[test]
fn backfill_does_not_adopt_when_definition_would_add_effective_env() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "9".repeat(64);
    let mut standalone = standalone_with_legacy_defaults("Solo", &pubkey, "You are Solo.");
    standalone["env_vars"] = serde_json::json!({ "INSTANCE_ONLY": "override" });
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            folded_definition_json("credentialed-persona", "Solo", "You are Solo.", "openai"),
            standalone,
        ]),
    );

    assert_eq!(
        backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
        1
    );
    let records = load_typed(dir.path());
    assert_eq!(records.len(), 3);
    let instance = records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    assert_eq!(instance.persona_id.as_deref(), Some(pubkey.as_str()));
}

#[test]
fn backfill_adopts_when_optional_definition_config_is_absent() {
    for (field, marker) in [("model", "c"), ("provider", "d")] {
        let dir = tempfile::tempdir().unwrap();
        let pubkey = marker.repeat(64);
        let folded_slug = format!("no-{field}");
        let mut definition =
            folded_definition_json(&folded_slug, "Solo", "You are Solo.", "openai");
        definition[field] = serde_json::Value::Null;
        write_agents_json(
            dir.path(),
            &serde_json::json!([
                definition,
                standalone_with_legacy_defaults("Solo", &pubkey, "You are Solo."),
            ]),
        );

        assert_eq!(
            backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
            1,
            "{field}"
        );
        let records = load_typed(dir.path());
        assert_eq!(records.len(), 2, "{field}");
        let instance = records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
        assert_eq!(
            instance.persona_id.as_deref(),
            Some(folded_slug.as_str()),
            "{field}"
        );
    }
}

#[test]
fn backfill_does_not_adopt_definition_with_divergent_known_runtime_override() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "6".repeat(64);
    let folded_slug = "goose-persona";
    let mut standalone = standalone_with_legacy_defaults("Solo", &pubkey, "You are Solo.");
    standalone["agent_command_override"] = serde_json::json!("claude-agent-acp");
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            folded_definition_json(folded_slug, "Solo", "You are Solo.", "openai"),
            standalone,
        ]),
    );

    assert_eq!(
        backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
        1
    );
    let records = load_typed(dir.path());
    assert_eq!(records.len(), 3);
    let instance = records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    assert_eq!(instance.persona_id.as_deref(), Some(pubkey.as_str()));
    assert_eq!(
        instance.agent_command_override.as_deref(),
        Some("claude-agent-acp")
    );
}

#[test]
fn backfill_uses_same_runtime_override_before_stale_legacy_command() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "e".repeat(64);
    let folded_slug = "goose-persona";
    let mut standalone = standalone_with_legacy_defaults("Solo", &pubkey, "You are Solo.");
    standalone["agent_command"] = serde_json::json!("claude-agent-acp");
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            folded_definition_json(folded_slug, "Solo", "You are Solo.", "openai"),
            standalone,
        ]),
    );

    assert_eq!(
        backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
        1
    );
    let records = load_typed(dir.path());
    assert_eq!(records.len(), 2);
    let instance = records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    assert_eq!(instance.persona_id.as_deref(), Some(folded_slug));
    assert_eq!(instance.agent_command_override.as_deref(), Some("goose"));
}

#[test]
fn backfill_adopts_legacy_command_without_runtime_or_override() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "f".repeat(64);
    let folded_slug = "legacy-goose-persona";
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            folded_definition_json(folded_slug, "Solo", "You are Solo.", "openai"),
            standalone_agent_json("Solo", &pubkey, Some("You are Solo.")),
        ]),
    );

    assert_eq!(
        backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
        1
    );
    let records = load_typed(dir.path());
    assert_eq!(records.len(), 2);
    let instance = records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    assert_eq!(instance.persona_id.as_deref(), Some(folded_slug));
    assert_eq!(instance.runtime, None);
    assert_eq!(instance.agent_command_override, None);
    assert_eq!(instance.agent_command, "goose");
}

#[test]
fn backfill_does_not_adopt_definition_for_relay_mesh_agent() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "7".repeat(64);
    let mut standalone = standalone_with_legacy_defaults("Solo", &pubkey, "You are Solo.");
    standalone["relay_mesh"] = serde_json::json!({ "model_ref": "Qwen3" });
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            folded_definition_json("non-mesh-persona", "Solo", "You are Solo.", "openai"),
            standalone,
        ]),
    );

    assert_eq!(
        backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
        1
    );
    let records = load_typed(dir.path());
    assert_eq!(records.len(), 3);
    let instance = records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    assert_eq!(instance.persona_id.as_deref(), Some(pubkey.as_str()));
    assert_eq!(
        instance
            .relay_mesh
            .as_ref()
            .map(|config| config.model_ref.as_str()),
        Some("Qwen3")
    );
}

#[test]
fn backfill_does_not_adopt_ambiguous_matching_definitions() {
    let dir = tempfile::tempdir().unwrap();
    let ambiguous_pubkey = "4".repeat(64);
    let clean_pubkey = "8".repeat(64);
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            folded_definition_json("legacy-persona-one", "Solo", "You are Solo.", "openai"),
            folded_definition_json("legacy-persona-two", "Solo", "You are Solo.", "openai"),
            standalone_with_legacy_defaults("Solo", &ambiguous_pubkey, "You are Solo."),
            standalone_with_legacy_defaults("Other", &clean_pubkey, "You are Other."),
        ]),
    );

    assert_eq!(
        backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
        1,
        "ambiguous record skipped while unrelated record proceeds"
    );
    let records = load_typed(dir.path());
    assert_eq!(
        records.len(),
        5,
        "ambiguous matches must not manufacture another definition"
    );
    let ambiguous = records
        .iter()
        .find(|record| record.pubkey == ambiguous_pubkey)
        .unwrap();
    assert_eq!(ambiguous.persona_id, None);
    let clean = records
        .iter()
        .find(|record| record.pubkey == clean_pubkey)
        .unwrap();
    assert_eq!(clean.persona_id.as_deref(), Some(clean_pubkey.as_str()));
}

#[test]
fn backfill_adopts_definition_without_overwriting_instance_behavior() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "5".repeat(64);
    let folded_slug = "legacy-defaults";
    let mut standalone = standalone_with_legacy_defaults("Solo", &pubkey, "You are Solo.");
    standalone["respond_to"] = serde_json::json!("anyone");
    standalone["parallelism"] = serde_json::json!(4);
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            folded_definition_json(folded_slug, "Solo", "You are Solo.", "openai"),
            standalone,
        ]),
    );

    assert_eq!(
        backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
        1
    );
    let records = load_typed(dir.path());
    assert_eq!(records.len(), 2);
    let instance = records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    assert_eq!(instance.persona_id.as_deref(), Some(folded_slug));
    assert_eq!(instance.respond_to.as_str(), "anyone");
    assert_eq!(instance.parallelism, 4);
}

#[test]
fn backfill_does_not_adopt_conflicting_explicit_behavior_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "b".repeat(64);
    let mut definition =
        folded_definition_json("allowlist-persona", "Solo", "You are Solo.", "openai");
    definition["definition_respond_to"] = serde_json::json!("allowlist");
    definition["definition_respond_to_allowlist"] = serde_json::json!(["a".repeat(64)]);
    definition["definition_parallelism"] = serde_json::json!(1);
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition,
            standalone_with_legacy_defaults("Solo", &pubkey, "You are Solo."),
        ]),
    );

    assert_eq!(
        backfill_standalone_agents_in_dir(&base(dir.path())).unwrap(),
        1
    );
    let records = load_typed(dir.path());
    assert_eq!(records.len(), 3);
    let instance = records.iter().find(|r| !r.pubkey.is_empty()).unwrap();
    assert_eq!(instance.persona_id.as_deref(), Some(pubkey.as_str()));
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
    let hash_before = spawn_config_hash(
        pre_instance,
        &[],
        &[],
        "wss://ws.example",
        &Default::default(),
    );

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
        &[],
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
    let hash_before = spawn_config_hash(
        pre_instance,
        &[],
        &[],
        "wss://ws.example",
        &Default::default(),
    );

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
        &[],
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
