use super::*;

#[test]
fn retention_scope_is_stable_and_separates_relay_and_owner() {
    let base = Path::new("/tmp/buzz-retention-test");
    let owner_a = "a".repeat(64);
    let owner_b = "b".repeat(64);
    let community_a = scoped_retention_db_path(base, "wss://a.example/", &owner_a);
    assert_eq!(
        community_a,
        scoped_retention_db_path(base, "wss://a.example", &owner_a)
    );
    assert_ne!(
        community_a,
        scoped_retention_db_path(base, "wss://b.example", &owner_a)
    );
    assert_ne!(
        community_a,
        scoped_retention_db_path(base, "wss://a.example", &owner_b)
    );
}

#[test]
fn test_arrival_relay_matching_agrees_with_database_identity() {
    let base = Path::new("/tmp/buzz-retention-test");
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let scope = |relay: &str| RetentionScope {
        db_path: scoped_retention_db_path(base, relay, &owner),
        relay_url: relay.to_string(),
        owner_keys: keys.clone(),
    };
    let community_a = scoped_retention_db_path(base, "wss://a.example", &owner);

    // "Same relay" and "same database" must never disagree: every URL the
    // match accepts has to hash to the scope's own db path, and every URL it
    // rejects has to hash somewhere else.
    for equivalent in ["wss://a.example", "wss://a.example/", " wss://a.example "] {
        assert_eq!(
            scope_for_arrival(scope("wss://a.example"), equivalent).map(|scope| scope.db_path),
            Some(community_a.clone()),
            "{equivalent}"
        );
        assert_eq!(
            scoped_retention_db_path(base, equivalent, &owner),
            community_a,
            "{equivalent}"
        );
    }

    assert!(
        scope_for_arrival(scope("wss://b.example"), "wss://a.example").is_none(),
        "an event from community A must not be filed while community B is active"
    );
    assert_ne!(
        scoped_retention_db_path(base, "wss://b.example", &owner),
        community_a
    );
}

#[test]
fn concurrent_open_waits_for_initialization_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retention.db");
    let first = open_retention_db(&path).unwrap();
    first.execute_batch("BEGIN EXCLUSIVE").unwrap();

    let second_path = path.clone();
    let second = std::thread::spawn(move || open_retention_db(&second_path));
    std::thread::sleep(std::time::Duration::from_millis(100));
    first.execute_batch("COMMIT").unwrap();

    assert!(second.join().unwrap().is_ok());
}

fn test_db() -> Connection {
    open_retention_db(Path::new(":memory:")).unwrap()
}

fn sample_event() -> RetainedEvent {
    RetainedEvent {
        kind: 30175,
        pubkey: "abc123".to_string(),
        d_tag: "test-persona".to_string(),
        content: r#"{"display_name":"Test"}"#.to_string(),
        created_at: 1000,
        raw_event: r#"{"id":"..."}"#.to_string(),
        pending_sync: true,
    }
}

#[test]
fn inbound_preflight_does_not_consume_event_before_commit() {
    let conn = test_db();
    let mut inbound = sample_event();
    inbound.pending_sync = false;

    assert_eq!(
        inbound_event_outcome(&conn, &inbound).unwrap(),
        InboundOutcome::Applied
    );
    assert!(
        get_retained_event(&conn, inbound.kind, &inbound.pubkey, &inbound.d_tag)
            .unwrap()
            .is_none()
    );
    // A failed store/runtime apply can replay the same head because the
    // preflight did not advance retention.
    assert_eq!(
        inbound_event_outcome(&conn, &inbound).unwrap(),
        InboundOutcome::Applied
    );
    assert_eq!(
        retain_inbound_event(&conn, &inbound).unwrap(),
        InboundOutcome::Applied
    );
    assert_eq!(
        inbound_event_outcome(&conn, &inbound).unwrap(),
        InboundOutcome::Skipped
    );
}

#[test]
fn retain_and_retrieve() {
    let conn = test_db();
    let event = sample_event();
    retain_event(&conn, &event).unwrap();

    let results = get_retained_personas(&conn, "abc123").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].d_tag, "test-persona");
    assert_eq!(results[0].created_at, 1000);
    assert!(results[0].pending_sync);
}

#[test]
fn tombstone_retention_keys_are_distinct_across_kinds() {
    // A persona slug, team id, and agent pubkey that all happen to equal
    // "shared" must occupy DISTINCT kind:5 rows so one tombstone's pending
    // publish never clobbers another's (F2c).
    let conn = test_db();
    for target_kind in [30175u32, 30176, 30177] {
        retain_event(
            &conn,
            &RetainedEvent {
                kind: 5,
                pubkey: "owner".to_string(),
                d_tag: tombstone_retention_d_tag(target_kind, "shared"),
                content: String::new(),
                created_at: 1000,
                raw_event: format!("{{\"k\":{target_kind}}}"),
                pending_sync: true,
            },
        )
        .unwrap();
    }
    // Three distinct rows survive — no PK collision clobbered any of them.
    for target_kind in [30175u32, 30176, 30177] {
        let row = get_retained_event(
            &conn,
            5,
            "owner",
            &tombstone_retention_d_tag(target_kind, "shared"),
        )
        .unwrap();
        assert!(
            row.is_some(),
            "tombstone for kind {target_kind} was clobbered"
        );
    }
}

#[test]
fn upsert_replaces_newer() {
    let conn = test_db();
    let mut event = sample_event();
    retain_event(&conn, &event).unwrap();

    event.content = r#"{"display_name":"Updated"}"#.to_string();
    event.created_at = 2000;
    retain_event(&conn, &event).unwrap();

    let results = get_retained_personas(&conn, "abc123").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].created_at, 2000);
    assert!(results[0].content.contains("Updated"));
}

#[test]
fn upsert_ignores_older() {
    let conn = test_db();
    let mut event = sample_event();
    event.created_at = 2000;
    retain_event(&conn, &event).unwrap();

    event.content = r#"{"display_name":"Old"}"#.to_string();
    event.created_at = 1000;
    retain_event(&conn, &event).unwrap();

    let results = get_retained_personas(&conn, "abc123").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].created_at, 2000);
    assert!(!results[0].content.contains("Old"));
}

#[test]
fn pending_sync_query() {
    let conn = test_db();
    let mut event = sample_event();
    event.pending_sync = true;
    retain_event(&conn, &event).unwrap();

    let mut event2 = sample_event();
    event2.d_tag = "other".to_string();
    event2.pending_sync = false;
    retain_event(&conn, &event2).unwrap();

    let pending = get_pending_sync(&conn).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].d_tag, "test-persona");
}

#[test]
fn test_mark_synced_matching_row_clears_flag() {
    let conn = test_db();
    let event = sample_event();
    retain_event(&conn, &event).unwrap();

    mark_synced(&conn, 30175, "abc123", "test-persona", 1000, &event.content).unwrap();

    let pending = get_pending_sync(&conn).unwrap();
    assert!(pending.is_empty());

    let results = get_retained_personas(&conn, "abc123").unwrap();
    assert_eq!(results.len(), 1);
    assert!(!results[0].pending_sync);
}

#[test]
fn test_mark_synced_stale_version_leaves_flag_set() {
    let conn = test_db();
    let published = sample_event();
    retain_event(&conn, &published).unwrap();

    // A newer edit lands at the same coordinate before the flush loop
    // clears the version it published.
    let mut newer = sample_event();
    newer.content = r#"{"display_name":"Edited"}"#.to_string();
    newer.created_at = 2000;
    retain_event(&conn, &newer).unwrap();

    // Clearing against the OLD version must not touch the newer pending row.
    mark_synced(
        &conn,
        30175,
        "abc123",
        "test-persona",
        1000,
        &published.content,
    )
    .unwrap();

    let pending = get_pending_sync(&conn).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].created_at, 2000);
}

#[test]
fn test_delete_retained_event_removes_row() {
    let conn = test_db();
    retain_event(&conn, &sample_event()).unwrap();

    delete_retained_event(&conn, 30175, "abc123", "test-persona").unwrap();

    assert!(get_retained_event(&conn, 30175, "abc123", "test-persona")
        .unwrap()
        .is_none());
}

#[test]
fn test_delete_retained_event_missing_row_is_noop() {
    let conn = test_db();
    delete_retained_event(&conn, 30175, "abc123", "nonexistent").unwrap();
}

#[test]
fn has_retained_personas_works() {
    let conn = test_db();
    assert!(!has_retained_personas(&conn, "abc123").unwrap());

    let event = sample_event();
    retain_event(&conn, &event).unwrap();

    assert!(has_retained_personas(&conn, "abc123").unwrap());
    assert!(!has_retained_personas(&conn, "other").unwrap());
}

#[test]
fn get_retained_event_by_coordinate() {
    let conn = test_db();
    let event = sample_event();
    retain_event(&conn, &event).unwrap();

    let found = get_retained_event(&conn, 30175, "abc123", "test-persona").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().d_tag, "test-persona");

    let not_found = get_retained_event(&conn, 30175, "abc123", "nonexistent").unwrap();
    assert!(not_found.is_none());
}

#[test]
fn idempotent_retain_same_timestamp() {
    let conn = test_db();
    let event = sample_event();
    retain_event(&conn, &event).unwrap();
    retain_event(&conn, &event).unwrap();

    let results = get_retained_personas(&conn, "abc123").unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn inbound_no_local_row_applies() {
    let conn = test_db();
    let mut event = sample_event();
    event.pending_sync = false;

    assert_eq!(
        retain_inbound_event(&conn, &event).unwrap(),
        InboundOutcome::Applied
    );

    let row = get_retained_event(&conn, 30175, "abc123", "test-persona")
        .unwrap()
        .unwrap();
    assert_eq!(row.created_at, 1000);
    assert!(!row.pending_sync);
}

#[test]
fn inbound_equal_second_skips_and_preserves_pending() {
    let conn = test_db();
    // Pending local edit at t=1000.
    let local = sample_event();
    retain_event(&conn, &local).unwrap();

    // Inbound at the SAME second with different content.
    let inbound = RetainedEvent {
        content: r#"{"display_name":"Remote"}"#.to_string(),
        pending_sync: false,
        ..sample_event()
    };
    assert_eq!(
        retain_inbound_event(&conn, &inbound).unwrap(),
        InboundOutcome::Skipped
    );

    // Local pending row is untouched: flag preserved, content unchanged so
    // the flush republishes and the relay resolves last-writer-wins.
    let row = get_retained_event(&conn, 30175, "abc123", "test-persona")
        .unwrap()
        .unwrap();
    assert!(row.pending_sync);
    assert!(row.content.contains("Test"));
}

#[test]
fn inbound_strictly_newer_applies_and_clears_pending() {
    let conn = test_db();
    // Pending local edit at t=1000.
    let local = sample_event();
    retain_event(&conn, &local).unwrap();

    // Inbound strictly newer with different content.
    let inbound = RetainedEvent {
        content: r#"{"display_name":"Remote"}"#.to_string(),
        created_at: 2000,
        pending_sync: false,
        ..sample_event()
    };
    assert_eq!(
        retain_inbound_event(&conn, &inbound).unwrap(),
        InboundOutcome::Applied
    );

    // Inbound wins: content replaced and pending cleared, so the stale
    // local edit stops republishing instead of looping.
    let row = get_retained_event(&conn, 30175, "abc123", "test-persona")
        .unwrap()
        .unwrap();
    assert_eq!(row.created_at, 2000);
    assert!(!row.pending_sync);
    assert!(row.content.contains("Remote"));
}

#[test]
fn inbound_older_skips() {
    let conn = test_db();
    let mut local = sample_event();
    local.created_at = 2000;
    retain_event(&conn, &local).unwrap();

    let inbound = RetainedEvent {
        content: r#"{"display_name":"Stale"}"#.to_string(),
        created_at: 1000,
        pending_sync: false,
        ..sample_event()
    };
    assert_eq!(
        retain_inbound_event(&conn, &inbound).unwrap(),
        InboundOutcome::Skipped
    );

    let row = get_retained_event(&conn, 30175, "abc123", "test-persona")
        .unwrap()
        .unwrap();
    assert_eq!(row.created_at, 2000);
    assert!(!row.content.contains("Stale"));
}

#[test]
fn pending_sync_publishes_tombstones_before_replacements() {
    // B5 resurrection race: a kind:5 retained in session N and the same
    // coordinate's replacement 30175 retained on the next boot can sit
    // pending together. The relay's a-tag deletion ignores timestamps,
    // so the tombstone MUST publish first or it wipes the replacement.
    let conn = test_db();
    let replacement = RetainedEvent {
        kind: 30175,
        created_at: 2000,
        pending_sync: true,
        ..sample_event()
    };
    retain_event(&conn, &replacement).unwrap();
    let tombstone = RetainedEvent {
        kind: 5,
        d_tag: tombstone_retention_d_tag(30175, "test-persona"),
        content: String::new(),
        created_at: 1000,
        pending_sync: true,
        ..sample_event()
    };
    retain_event(&conn, &tombstone).unwrap();

    let pending = get_pending_sync(&conn).unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].kind, 5, "tombstone first");
    assert_eq!(pending[1].kind, 30175, "replacement second");
}

#[test]
fn deferral_predicate_is_kind_and_pubkey_qualified() {
    // Mid-sweep barrier semantics: a failed tombstone defers ONLY the
    // replacement at its exact coordinate — same target kind, same pubkey.
    use std::collections::HashSet;

    let failed: HashSet<(String, String)> = HashSet::from([(
        "abc123".to_string(),
        tombstone_retention_d_tag(30175, "test-persona"),
    )]);

    // The covered replacement defers.
    assert!(deferred_behind_failed_tombstone(
        30175,
        "abc123",
        "test-persona",
        &failed
    ));
    // Kind-qualified: a coinciding slug under a DIFFERENT kind is a
    // distinct coordinate (the cross-kind collision the retention d-tag
    // encoding exists to prevent) — never deferred.
    assert!(!deferred_behind_failed_tombstone(
        30177,
        "abc123",
        "test-persona",
        &failed
    ));
    // Never crosses pubkeys.
    assert!(!deferred_behind_failed_tombstone(
        30175,
        "other-key",
        "test-persona",
        &failed
    ));
    // Never defers kind:5 rows, even at a "matching" retention key.
    assert!(!deferred_behind_failed_tombstone(
        5,
        "abc123",
        "test-persona",
        &failed
    ));
    // Unrelated d-tags publish normally.
    assert!(!deferred_behind_failed_tombstone(
        30175,
        "abc123",
        "other-persona",
        &failed
    ));
}
