use super::*;

async fn counts(f: &Fixture, event: &Event) -> (i32, i32, Option<DateTime<Utc>>) {
    sqlx::query_as("SELECT reply_count,descendant_count,last_reply_at FROM thread_metadata WHERE community_id=$1 AND event_id=$2")
        .bind(f.community.as_uuid()).bind(event.id.as_bytes().as_slice())
        .fetch_one(f.db.pool()).await.unwrap()
}

async fn metadata_exists(f: &Fixture, event: &Event) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM thread_metadata WHERE community_id=$1 AND event_id=$2)",
    )
    .bind(f.community.as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .fetch_one(f.db.pool())
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn dependent_vote_repairs_its_stored_thread_not_only_its_target() {
    let f = fixture().await;
    let target = f
        .event(&f.alice, 45001, "delete this post", vec![], None)
        .await;
    let other = f.event(&f.bob, 45001, "keep this post", vec![], None).await;
    let vote = f
        .event(
            &f.bob,
            45002,
            "+",
            vec![tag(&["e", &target.id.to_hex()])],
            Some((&other, &other, 1)),
        )
        .await;
    assert_eq!(counts(&f, &other).await.0, 1);
    assert_eq!(f.batch(1).await.unwrap().events_deleted, 2);
    assert!(!f.exists(&target).await && !f.exists(&vote).await);
    assert!(f.exists(&other).await);
    assert!(!metadata_exists(&f, &vote).await);
    assert_eq!(counts(&f, &other).await, (0, 0, None));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn authored_edit_and_vote_repair_their_stored_thread() {
    for kind in [40003, 45002] {
        let f = fixture().await;
        let target = f.event(&f.bob, 45001, "target", vec![], None).await;
        let other = f.event(&f.bob, 45001, "stored thread", vec![], None).await;
        let event = f
            .event(
                &f.alice,
                kind,
                "change",
                vec![tag(&["e", &target.id.to_hex()])],
                Some((&other, &other, 1)),
            )
            .await;
        assert_eq!(counts(&f, &other).await.0, 1);
        assert_eq!(f.batch(1).await.unwrap().events_deleted, 1);
        assert!(!f.exists(&event).await && !metadata_exists(&f, &event).await);
        assert!(f.exists(&target).await && f.exists(&other).await);
        assert_eq!(counts(&f, &other).await, (0, 0, None));
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn last_reply_time_uses_direct_replies_not_newer_nested_replies() {
    let f = fixture().await;
    let root = f.event(&f.bob, 9, "root", vec![], None).await;
    let direct = f
        .event(&f.bob, 9, "direct", vec![], Some((&root, &root, 1)))
        .await;
    let nested = f
        .event(&f.bob, 9, "nested", vec![], Some((&root, &direct, 2)))
        .await;
    let removed = f
        .event(&f.alice, 9, "remove", vec![], Some((&root, &root, 1)))
        .await;
    let earlier = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let later = DateTime::from_timestamp(1_700_000_100, 0).unwrap();
    for (event, received) in [(&direct, earlier), (&nested, later)] {
        sqlx::query("UPDATE events SET received_at=$3 WHERE community_id=$1 AND id=$2")
            .bind(f.community.as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .bind(received)
            .execute(f.db.pool())
            .await
            .unwrap();
    }
    f.batch(1).await.unwrap();
    assert!(!f.exists(&removed).await);
    assert_eq!(counts(&f, &root).await, (1, 2, Some(earlier)));
    assert_eq!(counts(&f, &direct).await, (1, 0, Some(later)));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn valueless_actor_tags_are_skipped_before_using_p_fallback() {
    let f = fixture().await;
    let removed = f
        .event(
            &f.relay,
            9,
            "belongs to subject",
            vec![
                tag(&["actor"]),
                tag(&["actor", &f.alice.public_key().to_hex()]),
                tag(&["p", &f.bob.public_key().to_hex()]),
            ],
            None,
        )
        .await;
    let retained = f
        .event(
            &f.relay,
            9,
            "belongs to other",
            vec![
                tag(&["actor"]),
                tag(&["actor", &f.bob.public_key().to_hex()]),
                tag(&["p", &f.alice.public_key().to_hex()]),
            ],
            None,
        )
        .await;
    f.finish(1).await;
    assert!(!f.exists(&removed).await);
    assert!(f.exists(&retained).await);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn affected_thread_limit_preserves_every_source_and_saves_retry() {
    let f = fixture().await;
    let target = f.event(&f.alice, 45001, "target", vec![], None).await;
    let mut dependents = Vec::new();
    // The seed plus 32 different stored roots exceeds the 32-thread limit.
    for i in 0..32 {
        let root = f
            .event(&f.bob, 45001, &format!("root {i}"), vec![], None)
            .await;
        let vote = f
            .event(
                &f.bob,
                45002,
                "+",
                vec![tag(&["e", &target.id.to_hex()])],
                Some((&root, &root, 1)),
            )
            .await;
        dependents.push((root, vote));
    }
    let error = f.batch(1).await.unwrap_err();
    assert!(
        error.to_string().contains("too many affected threads"),
        "{error}"
    );
    assert!(f.exists(&target).await);
    for (root, vote) in dependents {
        assert!(f.exists(&root).await && f.exists(&vote).await);
        assert!(metadata_exists(&f, &vote).await);
        assert_eq!(counts(&f, &root).await.0, 1);
    }
    let status: (String, String) = sqlx::query_as("SELECT r.status,s.status FROM entity_deletion_requests r JOIN entity_deletion_stage_history s ON s.request_id=r.id WHERE s.id=$1")
        .bind(f.stage.id).fetch_one(f.db.pool()).await.unwrap();
    assert_eq!(status, ("retry_wait".into(), "failed".into()));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn failure_repairing_another_thread_rolls_back_the_entire_batch() {
    let f = fixture().await;
    let target = f.event(&f.alice, 45001, "target", vec![], None).await;
    let other = f.event(&f.bob, 45001, "other", vec![], None).await;
    let vote = f
        .event(
            &f.bob,
            45002,
            "+",
            vec![tag(&["e", &target.id.to_hex()])],
            Some((&other, &other, 1)),
        )
        .await;
    let trigger = format!("fail_other_thread_{}", Uuid::new_v4().simple());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION {trigger}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected other thread repair failure'; END $$;
         CREATE TRIGGER {trigger} AFTER UPDATE ON thread_metadata FOR EACH ROW
         WHEN (NEW.community_id='{}' AND NEW.event_id=decode('{}','hex')) EXECUTE FUNCTION {trigger}();",
        f.community.as_uuid(), other.id.to_hex(),
    ))).execute(f.db.pool()).await.unwrap();
    let outcome = f.batch(1).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER {trigger} ON thread_metadata; DROP FUNCTION {trigger}();"
    )))
    .execute(f.db.pool())
    .await
    .unwrap();
    assert!(outcome
        .unwrap_err()
        .to_string()
        .contains("injected other thread"));
    assert!(f.exists(&target).await && f.exists(&vote).await && f.exists(&other).await);
    assert!(metadata_exists(&f, &target).await && metadata_exists(&f, &vote).await);
    assert_eq!(counts(&f, &other).await.0, 1);
}
