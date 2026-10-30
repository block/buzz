use super::*;

fn archive_request(key: &Keys, kind: u16) -> Event {
    EventBuilder::new(Kind::Custom(kind), Uuid::new_v4().to_string())
        .allow_self_tagging()
        .tags([
            Tag::parse(["-"]).unwrap(),
            Tag::parse(["p", &key.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(key)
        .unwrap()
}

async fn archive_snapshot(f: &Fixture) -> Vec<Value> {
    let mut snapshot = Vec::new();
    for table in ["archived_identities", "events"] {
        let rows: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text), '[]'::jsonb) FROM {table} t WHERE community_id=$1")))
            .bind(f.tenant.community().as_uuid()).fetch_one(&f.pool).await.unwrap();
        snapshot.push(rows);
    }
    snapshot
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn archive_and_unarchive_refusals_leave_no_state_or_notification_events() {
    for websocket in [false, true] {
        for kind in [9035, 9036] {
            for actor in [0, 1, 2] {
                let f = Fixture::new().await;
                let key = match actor {
                    0 => &f.user,
                    1 => &f.agent,
                    _ => &f.peer,
                };
                // Before fencing, both operations must work through the real path.
                let archive = archive_request(key, 9035);
                let unarchive = archive_request(key, 9036);
                for event in [&archive, &unarchive] {
                    let (accepted, reason) = f.send(key, event, websocket).await;
                    assert!(accepted, "baseline: {reason}");
                }
                if kind == 9036 {
                    assert!(f.send(key, &archive_request(key, 9035), websocket).await.0);
                }
                f.fence().await;
                let before = archive_snapshot(&f).await;
                let event = archive_request(key, kind);
                let (accepted, reason) = f.send(key, &event, websocket).await;
                if actor == 2 {
                    assert!(accepted, "unaffected user: {reason}");
                    assert_eq!(
                        f.state
                            .db
                            .is_archived(f.tenant.community(), &key.public_key().to_hex())
                            .await
                            .unwrap(),
                        kind == 9035
                    );
                } else {
                    assert!(!accepted);
                    assert_eq!(
                        reason,
                        "restricted: identity is restricted for user deletion"
                    );
                    assert_eq!(
                        archive_snapshot(&f).await,
                        before,
                        "refused kind {kind} changed archive state or persisted notifications"
                    );
                }
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn archive_delta_failure_rolls_back_request_and_mutation() {
    let f = Fixture::new().await;
    sqlx::raw_sql(
        "CREATE FUNCTION fail_archive_delta() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'injected archive delta failure'; END $$;
         CREATE TRIGGER fail_archive_delta BEFORE INSERT ON events
         FOR EACH ROW WHEN (NEW.kind IN (8002, 8003)) EXECUTE FUNCTION fail_archive_delta();",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    for websocket in [false, true] {
        for kind in [9035, 9036] {
            if kind == 9036 {
                f.state
                    .db
                    .archive(
                        f.tenant.community(),
                        &f.user.public_key().to_hex(),
                        "self",
                        &f.user.public_key().to_hex(),
                        None,
                        None,
                        &"a".repeat(64),
                    )
                    .await
                    .unwrap();
            }
            let before = archive_snapshot(&f).await;
            let request = archive_request(&f.user, kind);
            let (accepted, _, status) = f.submit(&f.user, &request, websocket).await;
            assert!(!accepted, "injected database failure must reach the caller");
            if let Some(status) = status {
                assert_eq!(status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
            }
            assert_eq!(
                archive_snapshot(&f).await,
                before,
                "delta failure must roll back the request and archive change"
            );
            f.state
                .db
                .unarchive(f.tenant.community(), &f.user.public_key().to_hex())
                .await
                .unwrap();
        }
    }
    sqlx::raw_sql("DROP TRIGGER fail_archive_delta ON events; DROP FUNCTION fail_archive_delta();")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        f.send(&f.user, &archive_request(&f.user, 9035), false)
            .await
            .0
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn archive_snapshot_failure_is_retryable_without_reapplying_mutation() {
    for websocket in [false, true] {
        let f = Fixture::new().await;
        let archive = archive_request(&f.user, 9035);
        sqlx::raw_sql(
            "CREATE FUNCTION fail_archive_snapshot() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN RAISE EXCEPTION 'injected archive snapshot failure'; END $$;
             CREATE TRIGGER fail_archive_snapshot BEFORE INSERT ON events
             FOR EACH ROW WHEN (NEW.kind = 13535) EXECUTE FUNCTION fail_archive_snapshot();",
        )
        .execute(&f.pool)
        .await
        .unwrap();
        let (accepted, _, status) = f.submit(&f.user, &archive, websocket).await;
        assert!(!accepted);
        if let Some(status) = status {
            assert_eq!(status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        }
        assert!(f
            .state
            .db
            .is_archived(f.tenant.community(), &f.user.public_key().to_hex())
            .await
            .unwrap());
        assert!(f
            .state
            .db
            .get_event_by_id(f.tenant.community(), archive.id.as_bytes())
            .await
            .unwrap()
            .is_some());
        sqlx::raw_sql(
            "DROP TRIGGER fail_archive_snapshot ON events; DROP FUNCTION fail_archive_snapshot();",
        )
        .execute(&f.pool)
        .await
        .unwrap();
        assert!(
            f.send(&f.user, &archive, websocket).await.0,
            "duplicate retry refreshes snapshot"
        );
        let unarchive = archive_request(&f.user, 9036);
        assert!(f.send(&f.user, &unarchive, websocket).await.0);
        assert!(f.send(&f.user, &archive, websocket).await.0);
        assert!(
            !f.state
                .db
                .is_archived(f.tenant.community(), &f.user.public_key().to_hex())
                .await
                .unwrap(),
            "replaying old archive must not undo later unarchive"
        );
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE community_id=$1 AND kind IN (8002,8003)",
        )
        .bind(f.tenant.community().as_uuid())
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(count, 2, "retries must not create new deltas");
        assert!(
            f.send(&f.user, &archive_request(&f.user, 9035), websocket)
                .await
                .0
        );
        assert!(f.send(&f.user, &unarchive, websocket).await.0);
        assert!(
            f.state
                .db
                .is_archived(f.tenant.community(), &f.user.public_key().to_hex())
                .await
                .unwrap(),
            "replaying old unarchive must not undo later archive"
        );
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn archive_requests_wait_for_fence_commit_or_rollback() {
    for websocket in [false, true] {
        for kind in [9035, 9036] {
            for commit in [false, true] {
                let f = Arc::new(Fixture::new().await);
                if kind == 9036 {
                    assert!(
                        f.send(&f.user, &archive_request(&f.user, 9035), websocket)
                            .await
                            .0
                    );
                }
                let before = archive_snapshot(&f).await;
                let mut gate = f.pending_fence().await;
                let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(&mut *gate)
                    .await
                    .unwrap();
                let task_f = f.clone();
                let task = tokio::spawn(async move {
                    task_f
                        .send(
                            &task_f.user,
                            &archive_request(&task_f.user, kind),
                            websocket,
                        )
                        .await
                });
                tokio::time::timeout(Duration::from_secs(4), async {
                    loop {
                        let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)))")
                            .bind(pid).fetch_one(&f.pool).await.unwrap();
                        if waiting { break; }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }).await.expect("archive writer must wait for the exclusive identity lock");
                assert_eq!(
                    archive_snapshot(&f).await,
                    before,
                    "no partial state before fencing decides"
                );
                if commit {
                    gate.commit().await.unwrap();
                } else {
                    gate.rollback().await.unwrap();
                }
                let (accepted, reason) = tokio::time::timeout(Duration::from_secs(8), task)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(accepted, !commit, "{reason}");
                if commit {
                    assert_eq!(archive_snapshot(&f).await, before);
                } else {
                    assert_eq!(
                        f.state
                            .db
                            .is_archived(f.tenant.community(), &f.user.public_key().to_hex())
                            .await
                            .unwrap(),
                        kind == 9035
                    );
                }
            }
        }
    }
}
