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
            instructions: Some("Be thorough.".to_string()),
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
        instructions: Some("Be thorough.".to_string()),
        persona_ids: vec!["alice".to_string(), "bob".to_string()],
        is_builtin: false,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    // Export with no instances and MemoryLevel::None — memory stays empty.
    let bytes = encode_team_snapshot_json(
        &build_team_export_snapshot(
            &team,
            &definitions,
            &[],
            MemoryLevel::None,
            &std::collections::HashMap::new(),
        )
        .unwrap(),
    )
    .unwrap();
    let decoded = decode_team_snapshot_from_bytes(&bytes).unwrap();

    assert_eq!(decoded.team.name, "Review Team");
    assert_eq!(decoded.team.description.as_deref(), Some("Reviews changes"));
    assert_eq!(decoded.team.instructions.as_deref(), Some("Be thorough."));
    assert_eq!(decoded.members.len(), 2);
    assert!(decoded.members.iter().all(|member| {
        member.memory.level == MemoryLevel::None && member.memory.entries.is_empty()
    }));
}

#[test]
fn team_export_with_instance_and_memory_level_uses_supplied_entries() {
    let definitions = vec![AgentDefinition {
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
    }];
    let team = TeamRecord {
        id: "t1".to_string(),
        name: "Team".to_string(),
        description: None,
        instructions: None,
        persona_ids: vec!["alice".to_string()],
        is_builtin: false,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    // Build a fake instance record tied to this team+persona.
    let instance = ManagedAgentRecord {
        pubkey: "a".repeat(64),
        name: "Alice".to_string(),
        display_name: None,
        slug: None,
        persona_id: Some("alice".to_string()),
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: None,
        acp_command: crate::managed_agents::DEFAULT_ACP_COMMAND.to_string(),
        agent_command: String::new(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 0,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: crate::managed_agents::DEFAULT_AGENT_PARALLELISM,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: Default::default(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: crate::managed_agents::BackendKind::Local,
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: Some("t1".to_string()),
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: crate::managed_agents::RespondTo::default(),
        respond_to_allowlist: vec![],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: vec![],
        definition_parallelism: None,
        relay_mesh: None,
        runtime: None,
        name_pool: vec![],
    };

    let mut memory_map = std::collections::HashMap::new();
    memory_map.insert(
        "alice".to_string(),
        vec![
            AgentSnapshotMemoryEntry {
                slug: "core".to_string(),
                body: "Alice is an expert reviewer.".to_string(),
            },
            AgentSnapshotMemoryEntry {
                slug: "mem/pref".to_string(),
                body: "prefers concise answers".to_string(),
            },
        ],
    );

    // MemoryLevel::Everything with an instance → entries should appear.
    let snap = build_team_export_snapshot(
        &team,
        &definitions,
        std::slice::from_ref(&instance),
        MemoryLevel::Everything,
        &memory_map,
    )
    .unwrap();
    assert_eq!(snap.members[0].memory.level, MemoryLevel::Everything);
    assert_eq!(snap.members[0].memory.entries.len(), 2);

    // MemoryLevel::None → entries stripped regardless.
    let snap_none = build_team_export_snapshot(
        &team,
        &definitions,
        std::slice::from_ref(&instance),
        MemoryLevel::None,
        &memory_map,
    )
    .unwrap();
    assert_eq!(snap_none.members[0].memory.level, MemoryLevel::None);
    assert!(snap_none.members[0].memory.entries.is_empty());

    // No instance → entries stripped even with Everything level.
    let snap_no_instance = build_team_export_snapshot(
        &team,
        &definitions,
        &[],
        MemoryLevel::Everything,
        &memory_map,
    )
    .unwrap();
    assert_eq!(snap_no_instance.members[0].memory.level, MemoryLevel::None);
    assert!(snap_no_instance.members[0].memory.entries.is_empty());
}

#[test]
fn team_import_definitions_are_built_for_all_members() {
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
    assert_eq!(team.instructions.as_deref(), Some("Be thorough."));
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

#[test]
fn parse_memory_level_round_trips_all_variants() {
    assert_eq!(parse_memory_level("none").unwrap(), MemoryLevel::None);
    assert_eq!(parse_memory_level("").unwrap(), MemoryLevel::None);
    assert_eq!(parse_memory_level("core").unwrap(), MemoryLevel::Core);
    assert_eq!(
        parse_memory_level("everything").unwrap(),
        MemoryLevel::Everything
    );
    assert!(parse_memory_level("bad").is_err());
}

// ── Rollback pattern tests ─────────────────────────────────────────────
//
// These test the file-level rollback mechanics used by confirm_team_snapshot_import
// Phase 3: snapshot files (or note absent), attempt writes, restore on failure.
// The Tauri AppHandle is not available in unit tests, so we exercise the same
// atomic_write + snapshot + restore operations directly on tempdir files.

#[cfg(unix)]
#[test]
fn rollback_restores_existing_agent_store_after_failed_write() {
    let dir = tempfile::tempdir().unwrap();
    let agents_path = dir.path().join("managed-agents.json");
    let original = b"[{\"pubkey\":\"existing\"}]";
    std::fs::write(&agents_path, original).unwrap();

    // Snapshot — should succeed for existing file.
    let snapshot = match std::fs::read(&agents_path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => panic!("unexpected read error: {e}"),
    };
    assert!(snapshot.is_some());

    // Simulate a write that changed the file.
    std::fs::write(&agents_path, b"[{\"pubkey\":\"imported\"}]").unwrap();

    // Rollback: restore from snapshot.
    let restore = match &snapshot {
        Some(bytes) => {
            crate::managed_agents::storage::atomic_write_json_restricted(&agents_path, bytes)
        }
        None => match std::fs::remove_file(&agents_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other.map_err(|e| e.to_string()),
        },
    };
    assert!(restore.is_ok());
    assert_eq!(std::fs::read(&agents_path).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn rollback_deletes_file_that_was_absent_before_import() {
    let dir = tempfile::tempdir().unwrap();
    let agents_path = dir.path().join("managed-agents.json");

    // Snapshot — file does not exist.
    let snapshot = match std::fs::read(&agents_path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => panic!("unexpected read error: {e}"),
    };
    assert!(snapshot.is_none());

    // Simulate a write that created the file.
    std::fs::write(&agents_path, b"[{\"pubkey\":\"orphan\"}]").unwrap();
    assert!(agents_path.exists());

    // Rollback: remove the file (restore "absent" state).
    let restore = match &snapshot {
        Some(bytes) => {
            crate::managed_agents::storage::atomic_write_json_restricted(&agents_path, bytes)
        }
        None => match std::fs::remove_file(&agents_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other.map_err(|e| e.to_string()),
        },
    };
    assert!(restore.is_ok());
    assert!(
        !agents_path.exists(),
        "absent-store rollback must delete the file"
    );
}

#[cfg(unix)]
#[test]
fn rollback_absent_file_treats_already_absent_as_success() {
    let dir = tempfile::tempdir().unwrap();
    let agents_path = dir.path().join("managed-agents.json");

    // File was absent before import and is still absent (e.g. the write that
    // would have created it also failed). Rollback must succeed.
    let restore = match std::fs::remove_file(&agents_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other.map_err(|e| e.to_string()),
    };
    assert!(
        restore.is_ok(),
        "removing an already-absent file must succeed"
    );
}

#[cfg(unix)]
#[test]
fn snapshot_read_error_on_unreadable_file_is_surfaced() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let agents_path = dir.path().join("managed-agents.json");
    std::fs::write(&agents_path, b"content").unwrap();
    std::fs::set_permissions(&agents_path, std::fs::Permissions::from_mode(0o000)).unwrap();

    // A non-NotFound read error must be surfaced, not collapsed to None.
    let result = match std::fs::read(&agents_path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("failed to snapshot agent store: {e}")),
    };

    // Restore permissions for cleanup.
    std::fs::set_permissions(&agents_path, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert!(
        result.is_err(),
        "permission error must not be collapsed to None"
    );
}

#[cfg(unix)]
#[test]
fn rollback_aggregates_multiple_errors() {
    // Simulate the error aggregation pattern used in the rollback closure.
    let original_err = "save_teams failed".to_string();
    let mut errors = vec![original_err.clone()];

    // Simulate a keyring cleanup failure.
    errors.push("keyring cleanup pubkey-1: keyring unreachable".to_string());

    // Simulate a disk restore failure.
    errors.push("agent store restore: permission denied".to_string());

    let combined = errors.join("; ");
    assert!(combined.contains("save_teams failed"));
    assert!(combined.contains("keyring cleanup pubkey-1"));
    assert!(combined.contains("agent store restore"));
    assert_eq!(
        combined.matches(';').count(),
        2,
        "three errors joined by two semicolons"
    );
}

/// Full-sequence failure injection at the teams-write boundary.
///
/// Exercises the complete confirm_team_snapshot_import phase-3 rollback path:
/// snapshot both stores → write agents (succeeds) → write teams (fails via
/// read-only dir) → rollback keyring + agents store + teams store → assert
/// exact pre-import disk state.
///
/// Keyring cleanup is exercised via a result-returning closure that mirrors
/// `try_delete_agent_key`'s signature. The real keyring seam is not called
/// here because `system-keyring` (default feature) accesses the OS keychain,
/// which blocks on authorization prompts in headless/CI environments.
/// The `try_delete_agent_key` function itself is integration-tested through
/// the `#[ignore]` keychain tests in `secret_store.rs`.
#[cfg(unix)]
#[test]
fn full_rollback_at_teams_boundary_existing_agents_store() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let agents_path = dir.path().join("managed-agents.json");
    let teams_path = dir.path().join("teams.json");

    // Pre-import: agents store exists, teams store absent.
    let original_agents = b"[{\"pubkey\":\"pre-existing\"}]";
    std::fs::write(&agents_path, original_agents).unwrap();

    // Snapshot both stores (mirrors production NotFound-aware reads).
    let agents_snap = match std::fs::read(&agents_path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => panic!("agents snapshot: {e}"),
    };
    let teams_snap = match std::fs::read(&teams_path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => panic!("teams snapshot: {e}"),
    };
    assert!(agents_snap.is_some());
    assert!(teams_snap.is_none());

    // Phase-3 write 1: agents store committed.
    crate::managed_agents::storage::atomic_write_json_restricted(
        &agents_path,
        b"[{\"pubkey\":\"imported\"}]",
    )
    .unwrap();
    assert_ne!(
        std::fs::read(&agents_path).unwrap().as_slice(),
        original_agents,
        "agents store must be changed by phase-3 write"
    );

    // Phase-3 write 2: teams write FAILS (read-only dir injection).
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let teams_err =
        crate::managed_agents::storage::atomic_write_json(&teams_path, b"[{\"id\":\"team-1\"}]");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(teams_err.is_err(), "teams write must fail in read-only dir");

    // Full rollback (mirrors production rollback_agents + teams restore).
    // Keyring cleanup: use a test-safe closure returning Ok(()) — the same
    // contract as try_delete_agent_key on a no-keyring-backend host.
    let minted_pubkeys = ["minted-aaa", "minted-bbb"];
    let mut errors = vec![teams_err.unwrap_err()];
    let try_delete_key = |_pk: &str| -> Result<(), String> { Ok(()) };

    for pk in &minted_pubkeys {
        if let Err(e) = try_delete_key(pk) {
            errors.push(format!("keyring cleanup {pk}: {e}"));
        }
    }
    let agents_restore = match &agents_snap {
        Some(bytes) => {
            crate::managed_agents::storage::atomic_write_json_restricted(&agents_path, bytes)
        }
        None => match std::fs::remove_file(&agents_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other.map_err(|e| e.to_string()),
        },
    };
    if let Err(e) = agents_restore {
        errors.push(format!("agent store restore: {e}"));
    }
    let teams_restore = match &teams_snap {
        Some(bytes) => crate::managed_agents::storage::atomic_write_json(&teams_path, bytes),
        None => match std::fs::remove_file(&teams_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other.map_err(|e| e.to_string()),
        },
    };
    if let Err(e) = teams_restore {
        errors.push(format!("teams store restore: {e}"));
    }

    // Exact pre-import disk state restored.
    assert_eq!(
        std::fs::read(&agents_path).unwrap(),
        original_agents,
        "agents store must be restored to original content"
    );
    assert!(!teams_path.exists(), "teams store must remain absent");
    assert_eq!(
        errors.len(),
        1,
        "only the teams-write error; keyring + disk rollback succeeded"
    );
}

/// Variant: agents store was absent before import (fresh install).
///
/// Exercises the NEW production control flow after the read-only loader fix:
/// load_teams_readonly (pre-commit, no write) → agents commit (creates file) →
/// save_teams fails → full rollback (keyring + delete created agents file +
/// teams absent-restore) → both files absent.
///
/// This specifically tests the fix for the Thufir-found bypass where
/// `load_teams()` was secretly a writer on absent files — a failure inside
/// that hidden write would `?`-return without calling `rollback_agents`.
/// With `load_teams_readonly` pre-commit, that path no longer exists.
#[cfg(unix)]
#[test]
fn full_rollback_at_teams_boundary_absent_agents_store() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let agents_path = dir.path().join("managed-agents.json");
    let teams_path = dir.path().join("teams.json");

    // Pre-import: BOTH stores absent (fresh install).
    let agents_snap = match std::fs::read(&agents_path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => panic!("agents snapshot: {e}"),
    };
    let teams_snap = match std::fs::read(&teams_path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => panic!("teams snapshot: {e}"),
    };
    assert!(agents_snap.is_none());
    assert!(teams_snap.is_none());

    // Pre-read teams via the read-only loader (mirrors production pre-commit).
    // On a fresh install with both files absent, this returns the merged
    // built-ins without writing. File must still be absent after the call.
    let teams = crate::managed_agents::load_teams_readonly(&teams_path).unwrap();
    assert!(
        !teams_path.exists(),
        "load_teams_readonly must not create the file"
    );

    // Phase-3 write 1: import CREATES agents store.
    std::fs::write(&agents_path, b"[{\"pubkey\":\"orphan\"}]").unwrap();
    assert!(agents_path.exists());

    // Phase-3 write 2: save_teams FAILS (read-only dir injection).
    // In production this is save_teams(&app, &teams) — we model the same
    // atomic_write_json call that save_teams delegates to.
    let mut teams_to_save = teams;
    teams_to_save.push(crate::managed_agents::TeamRecord {
        id: "team-1".to_string(),
        name: "Imported".to_string(),
        description: None,
        instructions: None,
        persona_ids: vec![],
        is_builtin: false,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    });
    let payload = serde_json::to_vec_pretty(&teams_to_save).unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let teams_err = crate::managed_agents::storage::atomic_write_json(&teams_path, &payload);
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(teams_err.is_err());

    // Full rollback: keyring cleanup (test-safe) + delete created agents file
    // + teams absent-restore.
    let mut errors = vec![teams_err.unwrap_err()];
    let try_delete_key = |_pk: &str| -> Result<(), String> { Ok(()) };
    if let Err(e) = try_delete_key("orphan") {
        errors.push(format!("keyring cleanup: {e}"));
    }
    let agents_restore = match &agents_snap {
        None => match std::fs::remove_file(&agents_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other.map_err(|e| e.to_string()),
        },
        Some(bytes) => {
            crate::managed_agents::storage::atomic_write_json_restricted(&agents_path, bytes)
        }
    };
    if let Err(e) = agents_restore {
        errors.push(format!("agent store restore: {e}"));
    }
    let teams_restore = match &teams_snap {
        None => match std::fs::remove_file(&teams_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other.map_err(|e| e.to_string()),
        },
        Some(bytes) => crate::managed_agents::storage::atomic_write_json(&teams_path, bytes),
    };
    if let Err(e) = teams_restore {
        errors.push(format!("teams store restore: {e}"));
    }

    // Exact pre-import state: both files absent.
    assert!(
        !agents_path.exists(),
        "rollback must delete file created by import on fresh install"
    );
    assert!(!teams_path.exists());
    assert_eq!(errors.len(), 1, "only the teams-write error");
}

#[test]
fn explicit_import_backend_is_applied() {
    let backend = BackendKind::Provider {
        id: "test".to_string(),
        config: serde_json::json!({"name": "imported"}),
    };
    assert_eq!(import_backend_or_local(Some(backend.clone())), backend);
}

#[test]
fn absent_import_backend_defaults_local() {
    assert_eq!(import_backend_or_local(None), BackendKind::Local);
}
