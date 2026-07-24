use nostr::{JsonUtil, Keys};
use rusqlite::Connection;
use tempfile::tempdir;

use buzz_core_pkg::command_brief::{
    build_command_brief_event, CommandBriefEventPayload, CommandBriefLifecycleState,
    COMMAND_BRIEF_PAYLOAD_VERSION,
};

use super::store::{
    insert_spool_event, list_due_spool_events, mark_publish_failed, mark_published,
    open_command_brief_store, SpoolInsert,
};

fn event(owner: &Keys, run_id: &str, previous: Option<String>) -> nostr::Event {
    build_command_brief_event(
        owner,
        &CommandBriefEventPayload {
            version: COMMAND_BRIEF_PAYLOAD_VERSION,
            classification: "OFFICIAL".into(),
            run_id: run_id.into(),
            schedule_id: "daily".into(),
            lifecycle_state: CommandBriefLifecycleState::Completed,
            occurred_at: "2026-07-25T06:00:00Z".into(),
            frozen_snapshot_id: "snapshot-1".into(),
            final_brief: Some(serde_json::json!({
                "classification": "OFFICIAL",
                "runId": run_id
            })),
            failure: None,
            previous_lifecycle_event_id: previous,
        },
    )
    .expect("event")
}

fn insert(event: &nostr::Event, run_id: &str, previous: Option<&str>) -> SpoolInsert {
    SpoolInsert {
        owner_pubkey: event.pubkey.to_hex(),
        run_id: run_id.to_string(),
        event_id: event.id.to_hex(),
        status: "completed".to_string(),
        previous_event_id: previous.map(str::to_string),
        encrypted_payload: event.content.clone(),
        raw_event: event.as_json(),
        created_at: 1_000,
    }
}

#[test]
fn store_uses_wal_atomic_schema_and_idempotent_append_only_inserts() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("command-brief.db");
    let conn = open_command_brief_store(&path).expect("open");
    let mode: String = conn
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .expect("mode");
    assert_eq!(mode.to_ascii_lowercase(), "wal");
    let owner = Keys::generate();
    let first = event(&owner, "run-1", None);
    assert!(insert_spool_event(&conn, insert(&first, "run-1", None)).expect("insert"));
    assert!(!insert_spool_event(&conn, insert(&first, "run-1", None)).expect("idempotent"));

    let conflicting = event(&owner, "run-1", None);
    assert!(insert_spool_event(&conn, insert(&conflicting, "run-1", None)).is_err());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM command_brief_spool", [], |row| row
            .get::<_, i64>(0))
            .expect("count"),
        1
    );
}

#[test]
fn predecessor_chain_never_overwrites_and_retry_state_is_bounded() {
    let conn = Connection::open_in_memory().expect("memory");
    super::store::migrate_command_brief_store(&conn).expect("migrate");
    let owner = Keys::generate();
    let first = event(&owner, "run-1", None);
    insert_spool_event(&conn, insert(&first, "run-1", None)).expect("first");
    let second = event(&owner, "run-1", Some(first.id.to_hex()));
    assert!(
        insert_spool_event(&conn, insert(&second, "run-1", Some(&first.id.to_hex())))
            .expect("second")
    );
    let wrong = event(&owner, "run-1", Some("a".repeat(64)));
    assert!(insert_spool_event(&conn, insert(&wrong, "run-1", Some(&"a".repeat(64)))).is_err());
    mark_published(
        &conn,
        &owner.public_key().to_hex(),
        &first.id.to_hex(),
        1_500,
    )
    .expect("first published");

    for attempt in 0..20 {
        mark_publish_failed(
            &conn,
            &second.pubkey.to_hex(),
            &second.id.to_hex(),
            2_000 + attempt,
        )
        .expect("retry");
    }
    let (retry_count, next_retry_at, state): (i64, i64, String) = conn
        .query_row(
            "SELECT retry_count,next_retry_at,publish_state FROM command_brief_spool
             WHERE event_id=?1",
            [&second.id.to_hex()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("retry row");
    assert_eq!(retry_count, 8);
    assert_eq!(state, "queued");
    assert!(next_retry_at <= 2_000 + 19 + 3_600);
    assert!(
        list_due_spool_events(&conn, &second.pubkey.to_hex(), i64::MAX, 64)
            .expect("due")
            .is_empty(),
        "retry-exhausted rows are retained but not auto-republished"
    );
}

#[test]
fn published_event_is_marked_by_exact_owner_and_id() {
    let conn = Connection::open_in_memory().expect("memory");
    super::store::migrate_command_brief_store(&conn).expect("migrate");
    let owner = Keys::generate();
    let first = event(&owner, "run-1", None);
    insert_spool_event(&conn, insert(&first, "run-1", None)).expect("first");
    mark_published(
        &conn,
        &owner.public_key().to_hex(),
        &first.id.to_hex(),
        3_000,
    )
    .expect("published");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM command_brief_spool WHERE publish_state = 'published'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(count, 1);
}

#[test]
fn concurrent_successors_have_one_predecessor_winner() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("command-brief.db");
    let owner = Keys::generate();
    let first = event(&owner, "run-1", None);
    let conn = open_command_brief_store(&path).expect("open");
    insert_spool_event(&conn, insert(&first, "run-1", None)).expect("first");
    drop(conn);

    let left = event(&owner, "run-1", Some(first.id.to_hex()));
    let right = event(&owner, "run-1", Some(first.id.to_hex()));
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let spawn = |candidate: nostr::Event| {
        let path = path.clone();
        let predecessor = first.id.to_hex();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let conn = open_command_brief_store(&path).expect("open worker");
            barrier.wait();
            insert_spool_event(&conn, insert(&candidate, "run-1", Some(&predecessor)))
        })
    };
    let left = spawn(left);
    let right = spawn(right);
    barrier.wait();
    let results = [
        left.join().expect("left join"),
        right.join().expect("right join"),
    ];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let conn = open_command_brief_store(&path).expect("reopen");
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM command_brief_spool WHERE run_id='run-1'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("count"),
        2
    );
}
