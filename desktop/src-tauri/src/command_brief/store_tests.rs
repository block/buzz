use nostr::{EventBuilder, JsonUtil, Keys, Kind, Timestamp};
use rusqlite::Connection;
use tempfile::tempdir;

use buzz_core_pkg::command_brief::{
    build_command_brief_event, CommandBriefEventPayload, CommandBriefLifecycleState,
    CommandBriefWire, COMMAND_BRIEF_PAYLOAD_VERSION,
};

use super::store::{
    insert_spool_event, list_due_spool_events, mark_publish_failed, mark_published,
    open_command_brief_store, rearm_queued_spool_events, validate_command_brief_store_schema,
    SpoolInsert,
};

#[test]
fn current_store_schema_is_exact_and_rejects_weakened_claim_invariants() {
    let conn = Connection::open_in_memory().expect("memory");
    super::store::migrate_command_brief_store(&conn).expect("migrate");
    validate_command_brief_store_schema(&conn).expect("exact schema");
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("version");
    assert_eq!(version, 4);
    assert!(conn
        .execute(
            "INSERT INTO command_brief_schedule_claims(
                idempotency_key,schedule_id,local_date,timezone,state,deferred_reason,
                retry_count,transition_token,claimed_at,updated_at,run_id
             ) VALUES('wrong','daily-command-brief','2026-07-25','Australia/Sydney',
                      'deferred',NULL,0,NULL,1,1,'scheduled-bad')",
            [],
        )
        .is_err());
    conn.execute_batch(
        "DROP INDEX command_brief_schedule_deferred;
         CREATE INDEX command_brief_schedule_deferred
         ON command_brief_schedule_claims(updated_at);",
    )
    .expect("weaken index");
    assert!(validate_command_brief_store_schema(&conn).is_err());
}

#[test]
fn exact_schema_rejects_malformed_spool_missing_heads_and_weakened_schedule_or_indexes() {
    for mutation in [
        "DROP INDEX command_brief_spool_due;
         DROP TABLE command_brief_spool;
         CREATE TABLE command_brief_spool(event_id TEXT PRIMARY KEY);",
        "DROP TABLE command_brief_heads;",
        "DROP TABLE command_brief_schedule;
         CREATE TABLE command_brief_schedule (
             schedule_id TEXT PRIMARY KEY,
             classification TEXT NOT NULL,
             enabled INTEGER NOT NULL,
             local_time TEXT NOT NULL,
             timezone TEXT NOT NULL,
             catch_up_same_day INTEGER NOT NULL,
             concurrency INTEGER NOT NULL,
             updated_at INTEGER NOT NULL
         );",
        "DROP INDEX command_brief_spool_due;
         CREATE INDEX command_brief_spool_due
         ON command_brief_spool(next_retry_at);",
    ] {
        let conn = Connection::open_in_memory().expect("memory");
        super::store::migrate_command_brief_store(&conn).expect("migrate");
        conn.execute_batch(mutation).expect("weaken schema");
        assert!(
            validate_command_brief_store_schema(&conn).is_err(),
            "weakened production schema was accepted: {mutation}"
        );
    }
}

#[test]
fn transition_token_constraint_is_bounded_nonempty_and_control_free() {
    let conn = Connection::open_in_memory().expect("memory");
    super::store::migrate_command_brief_store(&conn).expect("migrate");
    for token in ["", &"x".repeat(257), "line\nbreak"] {
        assert!(
            conn.execute(
                "INSERT INTO command_brief_schedule_claims(
                    idempotency_key,schedule_id,local_date,timezone,state,deferred_reason,
                    retry_count,transition_token,claimed_at,updated_at,run_id
                 ) VALUES(?1,'daily-command-brief',?2,'UTC','deferred',
                          'local_state_unavailable',0,?3,1,1,?4)",
                (
                    format!(
                        "daily-command-brief:{}",
                        if token.is_empty() {
                            "2026-07-25"
                        } else if token.len() > 256 {
                            "2026-07-26"
                        } else {
                            "2026-07-27"
                        }
                    ),
                    if token.is_empty() {
                        "2026-07-25"
                    } else if token.len() > 256 {
                        "2026-07-26"
                    } else {
                        "2026-07-27"
                    },
                    token,
                    super::schedule::deterministic_run_id(&format!(
                        "daily-command-brief:{}",
                        if token.is_empty() {
                            "2026-07-25"
                        } else if token.len() > 256 {
                            "2026-07-26"
                        } else {
                            "2026-07-27"
                        }
                    )),
                ),
            )
            .is_err(),
            "invalid transition token reached durable state"
        );
    }
}

#[test]
fn operational_reopen_is_cheap_while_explicit_validation_remains_fail_closed() {
    let directory = tempdir().expect("temp");
    let path = directory.path().join("brief.db");
    let conn = open_command_brief_store(&path).expect("create");
    conn.execute_batch("DROP TABLE command_brief_heads;")
        .expect("weaken after startup validation");
    drop(conn);

    let reopened = open_command_brief_store(&path).expect("cheap operational reopen");
    assert!(validate_command_brief_store_schema(&reopened).is_err());
}

#[test]
fn version_three_store_is_migrated_and_accepted_as_exact_version_four() {
    let conn = Connection::open_in_memory().expect("memory");
    super::store::migrate_command_brief_store(&conn).expect("initial schema");
    conn.execute_batch(
        "DROP INDEX command_brief_schedule_deferred;
         DROP TABLE command_brief_schedule_claims;
         CREATE TABLE command_brief_schedule_claims (
             idempotency_key TEXT PRIMARY KEY,
             schedule_id TEXT NOT NULL,
             local_date TEXT NOT NULL,
             timezone TEXT NOT NULL,
             state TEXT NOT NULL CHECK (state IN ('claimed','deferred','started','completed')),
             deferred_reason TEXT,
             retry_count INTEGER NOT NULL DEFAULT 0 CHECK (retry_count BETWEEN 0 AND 8),
             transition_token TEXT,
             claimed_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             run_id TEXT,
             UNIQUE (schedule_id,local_date)
         );
         CREATE INDEX command_brief_schedule_deferred
         ON command_brief_schedule_claims(state,retry_count,updated_at);
         INSERT INTO command_brief_schedule_claims VALUES(
             'daily-command-brief:2026-07-25','daily-command-brief','2026-07-25',
             'Australia/Sydney','started',NULL,0,NULL,1,2,'old-random-run');",
    )
    .expect("version three claims");
    conn.pragma_update(None, "user_version", 3)
        .expect("simulate prior version");
    super::store::migrate_command_brief_store(&conn).expect("v3 to v4");
    validate_command_brief_store_schema(&conn).expect("accepted current schema");
    let run_id: String = conn
        .query_row(
            "SELECT run_id FROM command_brief_schedule_claims",
            [],
            |row| row.get(0),
        )
        .expect("migrated run");
    assert_eq!(
        run_id,
        "scheduled-7df2f1c8a188345d60cb98a3aad7e88c0a572dc8ae29f10c084c0ebddbd42ed8"
    );
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("version");
    assert_eq!(version, 4);
}

fn event_at(owner: &Keys, run_id: &str, previous: Option<String>, created_at: u64) -> nostr::Event {
    let event = build_command_brief_event(
        owner,
        &CommandBriefEventPayload {
            version: COMMAND_BRIEF_PAYLOAD_VERSION,
            classification: "OFFICIAL".into(),
            run_id: run_id.into(),
            schedule_id: "daily".into(),
            lifecycle_state: CommandBriefLifecycleState::Completed,
            occurred_at: "2026-07-25T06:00:00Z".into(),
            frozen_snapshot_id: "snapshot-1".into(),
            final_brief: Some(
                CommandBriefWire::try_from(super::types_tests::brief_value())
                    .expect("strict brief"),
            ),
            failure: None,
            previous_lifecycle_event_id: previous,
        },
    )
    .expect("event");
    EventBuilder::new(event.kind, event.content)
        .tags(event.tags)
        .allow_self_tagging()
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(owner)
        .expect("event at time")
}

#[test]
fn readiness_rearms_a_bounded_exhausted_row_without_hot_looping() {
    let conn = Connection::open_in_memory().expect("memory");
    super::store::migrate_command_brief_store(&conn).expect("migrate");
    let owner = Keys::generate();
    let event = event(&owner, "run-1", None);
    insert_spool_event(&conn, insert(&event, "run-1", None)).expect("insert");
    for now in 0..8 {
        mark_publish_failed(&conn, &owner.public_key().to_hex(), &event.id.to_hex(), now)
            .expect("failed attempt");
    }
    assert!(
        list_due_spool_events(&conn, &owner.public_key().to_hex(), i64::MAX, 64)
            .expect("due")
            .is_empty()
    );

    assert_eq!(
        rearm_queued_spool_events(&conn, &owner.public_key().to_hex(), 9_000, 64).expect("rearm"),
        1
    );
    let due = list_due_spool_events(&conn, &owner.public_key().to_hex(), 9_000, 64).expect("due");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].event_id, event.id.to_hex());
    assert_eq!(due[0].retry_count, 0);
    assert_eq!(due[0].next_retry_at, 9_000);
}

fn event(owner: &Keys, run_id: &str, previous: Option<String>) -> nostr::Event {
    event_at(owner, run_id, previous, 1_000)
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
        created_at: event.created_at.as_secs() as i64,
    }
}

#[test]
fn admission_rejects_wrong_kind_created_at_mismatch_and_noncanonical_envelope() {
    let conn = Connection::open_in_memory().expect("memory");
    super::store::migrate_command_brief_store(&conn).expect("migrate");
    let owner = Keys::generate();
    let canonical = event(&owner, "run-1", None);

    let mut mismatched_time = insert(&canonical, "run-1", None);
    mismatched_time.created_at += 1;
    assert!(insert_spool_event(&conn, mismatched_time).is_err());

    let wrong_kind = EventBuilder::new(Kind::Custom(44_211), canonical.content.clone())
        .tags(canonical.tags.clone())
        .allow_self_tagging()
        .custom_created_at(canonical.created_at)
        .sign_with_keys(&owner)
        .expect("wrong kind");
    assert!(insert_spool_event(&conn, insert(&wrong_kind, "run-1", None)).is_err());

    let extra_tag = EventBuilder::new(canonical.kind, canonical.content.clone())
        .tags(
            canonical
                .tags
                .iter()
                .cloned()
                .chain([nostr::Tag::parse(["extra", "value"]).expect("extra")]),
        )
        .allow_self_tagging()
        .custom_created_at(canonical.created_at)
        .sign_with_keys(&owner)
        .expect("extra tag");
    assert!(insert_spool_event(&conn, insert(&extra_tag, "run-1", None)).is_err());
}

#[test]
fn monotonic_transactional_head_ignores_wall_clock_and_rejects_branches() {
    let conn = Connection::open_in_memory().expect("memory");
    super::store::migrate_command_brief_store(&conn).expect("migrate");
    let owner = Keys::generate();
    let first = event_at(&owner, "run-1", None, 2_000);
    insert_spool_event(&conn, insert(&first, "run-1", None)).expect("first");

    let backdated = event_at(&owner, "run-1", Some(first.id.to_hex()), 1_000);
    insert_spool_event(&conn, insert(&backdated, "run-1", Some(&first.id.to_hex())))
        .expect("backdated successor");
    assert_eq!(
        super::store::latest_event_id(&conn, &owner.public_key().to_hex(), "run-1")
            .expect("head")
            .as_deref(),
        Some(backdated.id.to_hex().as_str())
    );

    let branch = event_at(&owner, "run-1", Some(first.id.to_hex()), 3_000);
    assert!(insert_spool_event(&conn, insert(&branch, "run-1", Some(&first.id.to_hex()))).is_err());

    let next = event_at(&owner, "run-1", Some(backdated.id.to_hex()), 500);
    insert_spool_event(&conn, insert(&next, "run-1", Some(&backdated.id.to_hex())))
        .expect("next references monotonic head");
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
