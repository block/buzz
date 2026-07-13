use super::*;
use crate::managed_agents::{
    agent_snapshot::{
        AgentSnapshotDefinition, AgentSnapshotMemory, AgentSnapshotMemoryEntry,
        AgentSnapshotProfile,
    },
    team_snapshot::{TeamSnapshotMeta, FORMAT_DISCRIMINATOR, FORMAT_VERSION},
};

fn member(name: &str) -> AgentSnapshot {
    AgentSnapshot {
        format: crate::managed_agents::agent_snapshot::FORMAT_DISCRIMINATOR.to_string(),
        version: crate::managed_agents::agent_snapshot::FORMAT_VERSION,
        definition: AgentSnapshotDefinition {
            name: name.to_string(),
            system_prompt: Some(format!("{name} prompt")),
            runtime: Some("goose".to_string()),
            model: None,
            provider: None,
            parallelism: Some(2),
            respond_to: Some("allowlist".to_string()),
            respond_to_allowlist: vec!["ab".repeat(32)],
            name_pool: vec![],
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
        },
        profile: AgentSnapshotProfile {
            display_name: name.to_string(),
            about: None,
            avatar_data_url: None,
            avatar_url: Some(format!("https://example.test/{name}.png")),
        },
        memory: AgentSnapshotMemory {
            level: MemoryLevel::None,
            entries: vec![],
        },
    }
}

fn snapshot(members: Vec<AgentSnapshot>) -> TeamSnapshot {
    TeamSnapshot {
        format: FORMAT_DISCRIMINATOR.to_string(),
        version: FORMAT_VERSION,
        team: TeamSnapshotMeta {
            name: "Review Team".to_string(),
            description: Some("Reviews changes".to_string()),
        },
        members,
    }
}

#[test]
fn team_export_round_trip_preserves_team_and_excludes_member_memory() {
    let definitions = vec![
        AgentDefinition {
            id: "alice".to_string(),
            display_name: "Alice".to_string(),
            avatar_url: None,
            system_prompt: "Alice prompt".to_string(),
            runtime: Some("goose".to_string()),
            model: None,
            provider: None,
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            env_vars: Default::default(),
            respond_to: None,
            respond_to_allowlist: vec![],
            parallelism: None,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
        },
        AgentDefinition {
            id: "bob".to_string(),
            display_name: "Bob".to_string(),
            avatar_url: None,
            system_prompt: "Bob prompt".to_string(),
            runtime: Some("goose".to_string()),
            model: None,
            provider: None,
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            env_vars: Default::default(),
            respond_to: None,
            respond_to_allowlist: vec![],
            parallelism: None,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
        },
    ];
    let team = TeamRecord {
        id: "review".to_string(),
        name: "Review Team".to_string(),
        description: Some("Reviews changes".to_string()),
        persona_ids: vec!["alice".to_string(), "bob".to_string()],
        is_builtin: false,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let bytes =
        encode_team_snapshot_json(&build_team_export_snapshot(&team, &definitions).unwrap())
            .unwrap();
    let decoded = decode_team_snapshot_from_bytes(&bytes).unwrap();

    assert_eq!(decoded.team.name, "Review Team");
    assert_eq!(decoded.team.description.as_deref(), Some("Reviews changes"));
    assert_eq!(decoded.members.len(), 2);
    assert!(decoded.members.iter().all(|member| {
        member.memory.level == MemoryLevel::None && member.memory.entries.is_empty()
    }));
}

#[test]
fn team_import_creates_definitions_without_instances_or_memory() {
    let mut memory_bearing = member("Alice");
    memory_bearing.memory = AgentSnapshotMemory {
        level: MemoryLevel::Everything,
        entries: vec![AgentSnapshotMemoryEntry {
            slug: "core".to_string(),
            body: "must remain inert".to_string(),
        }],
    };
    let decoded = decode_team_snapshot_from_bytes(
        &encode_team_snapshot_json(&snapshot(vec![memory_bearing, member("Bob")])).unwrap(),
    )
    .unwrap();
    let definitions = build_import_definitions(&decoded, false, "now").unwrap();
    let team = build_import_team(
        &decoded,
        definitions
            .iter()
            .map(|definition| definition.id.clone())
            .collect(),
        "now",
    )
    .unwrap();

    assert_eq!(definitions.len(), 2);
    assert_eq!(team.persona_ids.len(), 2);
    assert_eq!(
        team.persona_ids,
        definitions
            .iter()
            .map(|definition| definition.id.clone())
            .collect::<Vec<_>>()
    );
    assert!(definitions.iter().all(|definition| {
        definition.id.len() == 36
            && definition.source_team.is_none()
            && definition.env_vars.is_empty()
            && definition.respond_to_allowlist.is_empty()
    }));
    assert_eq!(definitions[0].system_prompt, "Alice prompt");
    // `AgentDefinition` has no key/auth/memory fields: the exact import plan
    // creates N definitions + one TeamRecord, never a managed instance/key.
}

#[test]
fn team_import_keeps_or_clears_every_member_allowlist_with_one_toggle() {
    let source = snapshot(vec![member("Alice"), member("Bob")]);
    let kept = build_import_definitions(&source, true, "now").unwrap();
    let cleared = build_import_definitions(&source, false, "now").unwrap();

    assert!(kept.iter().all(|definition| {
        definition.respond_to.as_deref() == Some("allowlist")
            && definition.respond_to_allowlist == vec!["ab".repeat(32)]
    }));
    assert!(cleared.iter().all(|definition| {
        definition.respond_to.is_none() && definition.respond_to_allowlist.is_empty()
    }));
}

#[test]
fn legacy_flat_team_and_pack_zip_return_actionable_error() {
    let old_flat = br#"{"version":1,"type":"team","name":"Old"}"#;
    for bytes in [old_flat.as_slice(), b"PK\x05\x06empty-pack".as_slice()] {
        let error = decode_team_snapshot_from_bytes(bytes).unwrap_err();
        assert_eq!(error, LEGACY_TEAM_ERROR);
    }
}

#[test]
fn canonical_team_json_is_accepted_without_extension_case_policy() {
    let bytes = encode_team_snapshot_json(&snapshot(vec![member("Alice")])).unwrap();
    // Preview/confirm intentionally decode content rather than file names, so
    // canonical lowercase and uppercase extensions reach this same safe path.
    assert!(decode_team_snapshot_from_bytes(&bytes).is_ok());
}
