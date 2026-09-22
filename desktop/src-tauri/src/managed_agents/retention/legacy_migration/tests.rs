use super::*;
use crate::managed_agents::retention::{
    get_pending_sync, get_retained_event, retain_event, scoped_retention_db_path,
    tombstone_retention_d_tag,
};
use buzz_core_pkg::kind::KIND_PERSONA;

const KIND_DELETE: u32 = 5;
const OWNER: &str = "a1b2c3";

fn pending_tombstone(d_tag: &str) -> RetainedEvent {
    RetainedEvent {
        kind: KIND_DELETE,
        pubkey: OWNER.to_string(),
        d_tag: tombstone_retention_d_tag(KIND_PERSONA, d_tag),
        content: String::new(),
        created_at: 1_700_000_000,
        raw_event: format!(r#"{{"kind":5,"d":"{d_tag}"}}"#),
        pending_sync: true,
    }
}

fn seed_legacy(base_dir: &Path, events: &[RetainedEvent]) {
    let conn = open_retention_db(&legacy_retention_db_path(base_dir)).unwrap();
    for event in events {
        retain_event(&conn, event).unwrap();
    }
}

fn scope_path(base_dir: &Path, relay: &str) -> PathBuf {
    let path = scoped_retention_db_path(base_dir, relay, OWNER);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    path
}

#[test]
fn test_pending_legacy_tombstone_migrates_into_the_active_scope_and_stays_pending() {
    let dir = tempfile::tempdir().unwrap();
    seed_legacy(dir.path(), &[pending_tombstone("retired-agent")]);
    let scope = scope_path(dir.path(), "wss://a.example");

    let copied = migrate_legacy_retention_db(dir.path(), &scope, OWNER).unwrap();

    assert_eq!(copied, 1);
    let conn = open_retention_db(&scope).unwrap();
    let migrated = get_retained_event(
        &conn,
        KIND_DELETE,
        OWNER,
        &tombstone_retention_d_tag(KIND_PERSONA, "retired-agent"),
    )
    .unwrap()
    .expect("legacy tombstone lands in the scoped db");
    assert!(
        migrated.pending_sync,
        "the tombstone must still be queued for the flush loop"
    );
    assert_eq!(
        migrated.raw_event,
        pending_tombstone("retired-agent").raw_event
    );
    assert_eq!(get_pending_sync(&conn).unwrap().len(), 1);
}

#[test]
fn test_repeat_migration_of_the_same_scope_copies_nothing_further() {
    let dir = tempfile::tempdir().unwrap();
    seed_legacy(dir.path(), &[pending_tombstone("retired-agent")]);
    let scope = scope_path(dir.path(), "wss://a.example");

    assert_eq!(
        migrate_legacy_retention_db(dir.path(), &scope, OWNER).unwrap(),
        1
    );

    // Simulate the flush loop clearing the row, then boot again: the marker
    // must stop the legacy row from being resurrected as pending.
    let conn = open_retention_db(&scope).unwrap();
    conn.execute("UPDATE persona_events SET pending_sync = 0", [])
        .unwrap();
    drop(conn);

    assert_eq!(
        migrate_legacy_retention_db(dir.path(), &scope, OWNER).unwrap(),
        0
    );
    let conn = open_retention_db(&scope).unwrap();
    assert!(
        get_pending_sync(&conn).unwrap().is_empty(),
        "a published row must not be re-queued by a second migration pass"
    );
}

#[test]
fn test_second_community_does_not_receive_another_communitys_legacy_rows() {
    let dir = tempfile::tempdir().unwrap();
    seed_legacy(dir.path(), &[pending_tombstone("retired-agent")]);
    let first = scope_path(dir.path(), "wss://a.example");
    let second = scope_path(dir.path(), "wss://b.example");

    assert_eq!(
        migrate_legacy_retention_db(dir.path(), &first, OWNER).unwrap(),
        1
    );
    assert_eq!(
        migrate_legacy_retention_db(dir.path(), &second, OWNER).unwrap(),
        0,
        "legacy rows belong to exactly one relay scope"
    );

    let conn = open_retention_db(&second).unwrap();
    assert!(get_pending_sync(&conn).unwrap().is_empty());
}

#[test]
fn test_rows_authored_by_another_identity_are_left_behind() {
    let dir = tempfile::tempdir().unwrap();
    let mut foreign = pending_tombstone("someone-elses");
    foreign.pubkey = "ffffff".to_string();
    seed_legacy(dir.path(), &[pending_tombstone("mine"), foreign]);
    let scope = scope_path(dir.path(), "wss://a.example");

    assert_eq!(
        migrate_legacy_retention_db(dir.path(), &scope, OWNER).unwrap(),
        1
    );

    let conn = open_retention_db(&scope).unwrap();
    let pending = get_pending_sync(&conn).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].pubkey, OWNER);
}

#[test]
fn test_post_upgrade_scoped_row_is_not_overwritten_by_its_legacy_ancestor() {
    let dir = tempfile::tempdir().unwrap();
    let legacy_head = RetainedEvent {
        kind: KIND_PERSONA,
        pubkey: OWNER.to_string(),
        d_tag: "reviewer".to_string(),
        content: r#"{"display_name":"Old"}"#.to_string(),
        created_at: 1_700_000_000,
        raw_event: r#"{"content":"old"}"#.to_string(),
        pending_sync: true,
    };
    seed_legacy(dir.path(), &[legacy_head]);
    let scope = scope_path(dir.path(), "wss://a.example");

    // An edit made after the upgrade already occupies the coordinate.
    let conn = open_retention_db(&scope).unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_PERSONA,
            pubkey: OWNER.to_string(),
            d_tag: "reviewer".to_string(),
            content: r#"{"display_name":"New"}"#.to_string(),
            created_at: 1_700_000_500,
            raw_event: r#"{"content":"new"}"#.to_string(),
            pending_sync: true,
        },
    )
    .unwrap();
    drop(conn);

    migrate_legacy_retention_db(dir.path(), &scope, OWNER).unwrap();

    let conn = open_retention_db(&scope).unwrap();
    let row = get_retained_event(&conn, KIND_PERSONA, OWNER, "reviewer")
        .unwrap()
        .unwrap();
    assert_eq!(row.created_at, 1_700_000_500);
    assert_eq!(row.raw_event, r#"{"content":"new"}"#);
}

#[test]
fn test_absent_legacy_database_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let scope = scope_path(dir.path(), "wss://a.example");

    assert_eq!(
        migrate_legacy_retention_db(dir.path(), &scope, OWNER).unwrap(),
        0
    );
    assert!(!legacy_retention_db_path(dir.path()).exists());
}
