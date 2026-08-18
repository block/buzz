use super::*;

/// Helper: write a `personas.json` directly in `base_dir` (the migration
/// reads `base_dir/personas.json`, where `base_dir` is the `agents` dir).
fn write_base_personas(base_dir: &Path, records: &serde_json::Value) {
    std::fs::write(
        base_dir.join("personas.json"),
        serde_json::to_string_pretty(records).unwrap(),
    )
    .unwrap();
}

fn one_persona() -> serde_json::Value {
    serde_json::json!([{
        "id": "code-reviewer",
        "display_name": "Code Reviewer",
        "system_prompt": "You review code.",
        "is_builtin": false,
        "is_active": true,
        "name_pool": [],
        "env_vars": {},
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z"
    }])
}

#[test]
fn migrate_personas_writes_signed_retention_rows() {
    use crate::managed_agents::retention::{get_retained_personas, open_retention_db};

    let base = tempfile::tempdir().unwrap();
    write_base_personas(base.path(), &one_persona());
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();

    let migrated = migrate_personas_in_dir(base.path(), &keys).unwrap();
    assert_eq!(migrated, 1);

    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let rows = get_retained_personas(&conn, &pubkey).unwrap();
    assert_eq!(rows.len(), 1);
    // Row holds a real signed event for the owner — not a placeholder.
    assert_eq!(rows[0].pubkey, pubkey);
    let event: nostr::Event = nostr::JsonUtil::from_json(&rows[0].raw_event).unwrap();
    assert!(event.verify().is_ok());
    assert!(rows[0].pending_sync);
}

#[test]
fn migrate_personas_skips_builtins() {
    use crate::managed_agents::retention::{get_retained_personas, open_retention_db};

    let base = tempfile::tempdir().unwrap();
    write_base_personas(
        base.path(),
        &serde_json::json!([{
            "id": "builtin:solo",
            "display_name": "Solo",
            "system_prompt": "x",
            "is_builtin": true,
            "is_active": true,
            "name_pool": [],
            "env_vars": {},
            "created_at": "2025-01-01T00:00:00Z",
            "updated_at": "2025-01-01T00:00:00Z"
        }]),
    );
    let keys = nostr::Keys::generate();

    let migrated = migrate_personas_in_dir(base.path(), &keys).unwrap();
    assert_eq!(migrated, 0);

    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let rows = get_retained_personas(&conn, &keys.public_key().to_hex()).unwrap();
    assert!(rows.is_empty());
}

#[test]
fn migrate_personas_unchanged_second_run_is_noop() {
    let base = tempfile::tempdir().unwrap();
    write_base_personas(base.path(), &one_persona());
    let keys = nostr::Keys::generate();

    // First run retains; second run with identical personas re-retains
    // nothing — the per-coordinate content matches, so `pending_sync` is
    // not churned.
    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);
    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 0);
    assert!(!base.path().join("migration_state.json").exists());
}

#[test]
fn migrate_personas_new_persona_after_first_run_gets_retained() {
    use crate::managed_agents::retention::{get_retained_personas, open_retention_db};

    let base = tempfile::tempdir().unwrap();
    write_base_personas(base.path(), &one_persona());
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();

    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);

    // A persona added to personas.json after the first reconcile must be
    // picked up — the whole-store sentinel that previously short-circuited
    // this is gone.
    let mut two = one_persona();
    two.as_array_mut().unwrap().push(serde_json::json!({
        "id": "test-writer",
        "display_name": "Test Writer",
        "system_prompt": "You write tests.",
        "is_builtin": false,
        "is_active": true,
        "name_pool": [],
        "env_vars": {},
        "created_at": "2025-01-02T00:00:00Z",
        "updated_at": "2025-01-02T00:00:00Z"
    }));
    write_base_personas(base.path(), &two);

    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);

    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let rows = get_retained_personas(&conn, &pubkey).unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn migrate_personas_edited_persona_re_retains_pending() {
    use crate::managed_agents::retention::{get_retained_event, mark_synced, open_retention_db};
    use buzz_core_pkg::kind::KIND_PERSONA;

    let base = tempfile::tempdir().unwrap();
    write_base_personas(base.path(), &one_persona());
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();

    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);

    // Simulate the flush loop confirming the first publish.
    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let row = get_retained_event(&conn, KIND_PERSONA, &pubkey, "code-reviewer")
        .unwrap()
        .unwrap();
    mark_synced(
        &conn,
        KIND_PERSONA,
        &pubkey,
        "code-reviewer",
        row.created_at,
        &row.content,
    )
    .unwrap();
    drop(conn);

    // Editing the persona on disk must re-retain it as pending so the edit
    // reaches the relay on the next flush.
    let mut edited = one_persona();
    edited.as_array_mut().unwrap()[0]["system_prompt"] =
        serde_json::json!("You review code carefully.");
    write_base_personas(base.path(), &edited);

    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);

    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let row = get_retained_event(&conn, KIND_PERSONA, &pubkey, "code-reviewer")
        .unwrap()
        .unwrap();
    assert!(row.pending_sync);
    assert!(row.content.contains("carefully"));
}

#[test]
fn migrate_personas_no_file_is_noop() {
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 0);
}

/// F8: a future-dated retained head must be SUPERSEDED on a changed-content
/// migration, not silently skipped by `retain_event`'s `>=` guard. Without the
/// monotonic `created_at` bump the rebuilt event lands at `now <= head`, the
/// upsert's `WHERE excluded.created_at >= ...` drops the UPDATE, and `migrated`
/// over-reports. The bump (max(now, head+1)) guarantees supersession.
#[test]
fn migrate_personas_supersedes_future_dated_head() {
    use crate::managed_agents::retention::{
        get_retained_event, open_retention_db, retain_event, RetainedEvent,
    };
    use buzz_core_pkg::kind::KIND_PERSONA;

    let base = tempfile::tempdir().unwrap();
    write_base_personas(base.path(), &one_persona());
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();

    // First migrate retains the persona at ~now.
    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);

    // Force the retained head far into the future, simulating a clock-skewed or
    // same-second `max(now, head+1)` interactive bump.
    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let head = get_retained_event(&conn, KIND_PERSONA, &pubkey, "code-reviewer")
        .unwrap()
        .unwrap();
    let future = nostr::Timestamp::now().as_secs() as i64 + 100_000;
    retain_event(
        &conn,
        &RetainedEvent {
            created_at: future,
            pending_sync: false,
            ..head
        },
    )
    .unwrap();

    // Change the persona body on disk, then migrate again.
    let mut edited = one_persona();
    edited.as_array_mut().unwrap()[0]["system_prompt"] =
        serde_json::json!("You review code very carefully.");
    write_base_personas(base.path(), &edited);

    assert_eq!(
        migrate_personas_in_dir(base.path(), &keys).unwrap(),
        1,
        "changed content over a future-dated head must report a real migration"
    );

    let row = get_retained_event(&conn, KIND_PERSONA, &pubkey, "code-reviewer")
        .unwrap()
        .unwrap();
    // The new body actually landed (not silently skipped) ...
    assert!(
        row.content.contains("very carefully"),
        "changed body must supersede the future-dated head, not be dropped"
    );
    // ... at a created_at strictly past the future head (monotonic bump) ...
    assert_eq!(row.created_at, future + 1);
    // ... and is queued for republish.
    assert!(row.pending_sync, "superseding row must be pending_sync");
}

fn write_base_teams(base_dir: &Path, records: &serde_json::Value) {
    std::fs::write(
        base_dir.join("teams.json"),
        serde_json::to_string_pretty(records).unwrap(),
    )
    .unwrap();
}

/// F8 for the team migration site — same supersede guarantee as personas.
#[test]
fn migrate_teams_supersedes_future_dated_head() {
    use crate::managed_agents::retention::{
        get_retained_event, open_retention_db, retain_event, RetainedEvent,
    };
    use buzz_core_pkg::kind::KIND_TEAM;

    let base = tempfile::tempdir().unwrap();
    let team = serde_json::json!([{
        "id": "my-team",
        "name": "My Team",
        "description": "first",
        "persona_ids": ["code-reviewer"],
        "is_builtin": false,
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z"
    }]);
    write_base_teams(base.path(), &team);
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();

    assert_eq!(migrate_teams_in_dir(base.path(), &keys).unwrap(), 1);

    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let head = get_retained_event(&conn, KIND_TEAM, &pubkey, "my-team")
        .unwrap()
        .unwrap();
    let future = nostr::Timestamp::now().as_secs() as i64 + 100_000;
    retain_event(
        &conn,
        &RetainedEvent {
            created_at: future,
            pending_sync: false,
            ..head
        },
    )
    .unwrap();

    let mut edited = team.clone();
    edited.as_array_mut().unwrap()[0]["description"] = serde_json::json!("second");
    write_base_teams(base.path(), &edited);

    assert_eq!(migrate_teams_in_dir(base.path(), &keys).unwrap(), 1);

    let row = get_retained_event(&conn, KIND_TEAM, &pubkey, "my-team")
        .unwrap()
        .unwrap();
    assert!(row.content.contains("second"));
    assert_eq!(row.created_at, future + 1);
    assert!(row.pending_sync);
}
