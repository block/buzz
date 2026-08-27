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

/// A retained persona head whose disk record was deleted (a tombstone whose
/// atomic purge+enqueue rolled back) is an orphan: boot's positive legs
/// enumerate disk records and never revisit it, so only the deletion sweep can
/// retract it. The sweep must enqueue a kind:5 tombstone and purge the head.
#[test]
fn deletion_reconcile_tombstones_orphan_persona_head() {
    use crate::managed_agents::retention::{get_retained_event, open_retention_db};
    use buzz_core_pkg::kind::{KIND_DELETION, KIND_PERSONA};

    let base = tempfile::tempdir().unwrap();
    write_base_personas(base.path(), &one_persona());
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();
    let db_path = base.path().join("retention.db");

    // Positive leg retains the head, then the disk record is deleted.
    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);
    write_base_personas(base.path(), &serde_json::json!([]));

    assert_eq!(
        reconcile_deleted_heads_at(base.path(), &keys, &db_path).unwrap(),
        1
    );

    let conn = open_retention_db(&db_path).unwrap();
    // The 30175 head is purged and a kind:5 tombstone is enqueued for it.
    assert!(
        get_retained_event(&conn, KIND_PERSONA, &pubkey, "code-reviewer")
            .unwrap()
            .is_none(),
        "the orphan head must be purged"
    );
    let tombstone_d_tag =
        crate::managed_agents::retention::tombstone_retention_d_tag(KIND_PERSONA, "code-reviewer");
    let tombstone = get_retained_event(&conn, KIND_DELETION, &pubkey, &tombstone_d_tag)
        .unwrap()
        .expect("a kind:5 tombstone is enqueued for the orphan");
    assert!(
        tombstone.pending_sync,
        "the tombstone is queued for publish"
    );
}

/// A head whose disk record still exists is NOT an orphan: the sweep must leave
/// it alone. This is the guard that keeps the negative leg from retracting live
/// state right after the positive leg retained it.
#[test]
fn deletion_reconcile_leaves_live_head_untouched() {
    use crate::managed_agents::retention::{get_retained_event, open_retention_db};
    use buzz_core_pkg::kind::{KIND_DELETION, KIND_PERSONA};

    let base = tempfile::tempdir().unwrap();
    write_base_personas(base.path(), &one_persona());
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();
    let db_path = base.path().join("retention.db");

    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);

    // The disk record is still present, so nothing is orphaned.
    assert_eq!(
        reconcile_deleted_heads_at(base.path(), &keys, &db_path).unwrap(),
        0
    );

    let conn = open_retention_db(&db_path).unwrap();
    assert!(
        get_retained_event(&conn, KIND_PERSONA, &pubkey, "code-reviewer")
            .unwrap()
            .is_some(),
        "a live head must survive the deletion sweep"
    );
    let tombstone_d_tag =
        crate::managed_agents::retention::tombstone_retention_d_tag(KIND_PERSONA, "code-reviewer");
    assert!(
        get_retained_event(&conn, KIND_DELETION, &pubkey, &tombstone_d_tag)
            .unwrap()
            .is_none(),
        "no tombstone may be enqueued for a live head"
    );
}

/// A malformed `managed-agents.json` must fail loud (and be preserved as
/// `.invalid`) — never read as empty and orphan every persona and agent head.
/// This is the hard rider: a truncated file must never trigger tombstones.
#[test]
fn deletion_reconcile_malformed_store_fails_loud_without_tombstoning() {
    use crate::managed_agents::retention::{get_retained_event, open_retention_db};
    use buzz_core_pkg::kind::{KIND_DELETION, KIND_PERSONA};

    let base = tempfile::tempdir().unwrap();
    write_base_personas(base.path(), &one_persona());
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();
    let db_path = base.path().join("retention.db");

    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);
    // Truncate managed-agents.json to invalid JSON AFTER the head is retained.
    std::fs::write(base.path().join("managed-agents.json"), b"{ truncated").unwrap();

    let err = reconcile_deleted_heads_at(base.path(), &keys, &db_path)
        .expect_err("a malformed store must fail loud");
    assert!(
        err.contains("managed-agents.json"),
        "error names the store: {err}"
    );
    assert!(
        base.path().join("managed-agents.json.invalid").exists(),
        "the malformed store is preserved as .invalid"
    );

    let conn = open_retention_db(&db_path).unwrap();
    assert!(
        get_retained_event(&conn, KIND_PERSONA, &pubkey, "code-reviewer")
            .unwrap()
            .is_some(),
        "a malformed store must NOT orphan a live head"
    );
    let tombstone_d_tag =
        crate::managed_agents::retention::tombstone_retention_d_tag(KIND_PERSONA, "code-reviewer");
    assert!(
        get_retained_event(&conn, KIND_DELETION, &pubkey, &tombstone_d_tag)
            .unwrap()
            .is_none(),
        "a fail-loud abort must enqueue no tombstones"
    );
}

/// A retained 30177 managed-agent head with NO local disk record is the NORMAL
/// cross-device state — inbound sync retains an agent's head on device B
/// without minting a local record, because agents carry device-local secrets
/// that can't come from a relay event. The deletion sweep must therefore leave
/// it untouched: no kind:5 tombstone, no kind:9035 archive, and the head
/// survives. Sweeping it would delete every device-A agent at device B's boot.
#[test]
fn deletion_reconcile_leaves_managed_agent_head_untouched() {
    use crate::managed_agents::retention::{
        get_retained_event, open_retention_db, retain_event, RetainedEvent,
    };
    use buzz_core_pkg::kind::{KIND_DELETION, KIND_IA_ARCHIVE_REQUEST, KIND_MANAGED_AGENT};

    // A valid 32-byte x-only pubkey hex — the 30177 d_tag is the agent pubkey.
    const AGENT_PUBKEY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();
    let db_path = base.path().join("retention.db");

    // Device B: a 30177 head retained via inbound sync, with no disk record and
    // no managed-agents.json at all (the store is absent on a fresh device).
    let conn = open_retention_db(&db_path).unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_MANAGED_AGENT,
            pubkey: pubkey.clone(),
            d_tag: AGENT_PUBKEY.to_string(),
            content: r#"{"name":"Agent"}"#.to_string(),
            created_at: 1_700_000_000,
            raw_event: r#"{"id":"seed"}"#.to_string(),
            pending_sync: false,
        },
    )
    .unwrap();
    drop(conn);

    // No persona/team records either, so the sweep tombstones nothing.
    assert_eq!(
        reconcile_deleted_heads_at(base.path(), &keys, &db_path).unwrap(),
        0
    );

    let conn = open_retention_db(&db_path).unwrap();
    assert!(
        get_retained_event(&conn, KIND_MANAGED_AGENT, &pubkey, AGENT_PUBKEY)
            .unwrap()
            .is_some(),
        "the device-A agent head must survive device B's boot sweep"
    );
    let tombstone_d_tag = crate::managed_agents::retention::tombstone_retention_d_tag(
        KIND_MANAGED_AGENT,
        AGENT_PUBKEY,
    );
    assert!(
        get_retained_event(&conn, KIND_DELETION, &pubkey, &tombstone_d_tag)
            .unwrap()
            .is_none(),
        "no kind:5 tombstone may be enqueued for a device-local-absent agent"
    );
    assert!(
        get_retained_event(&conn, KIND_IA_ARCHIVE_REQUEST, &pubkey, AGENT_PUBKEY)
            .unwrap()
            .is_none(),
        "no kind:9035 archive may be enqueued for a device-local-absent agent"
    );
}
