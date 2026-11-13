use super::*;

async fn conversation(f: &Fixture) -> (Event, Event, Event) {
    let root = f.event(&f.bob, 9, "root", vec![], None).await;
    let source = f
        .event(
            &f.alice,
            9,
            "reply",
            vec![tag(&["p", &f.bob.public_key().to_hex()])],
            Some((&root, &root, 1)),
        )
        .await;
    let reaction = f.reaction(&f.bob, &source).await;
    (root, source, reaction)
}

async fn assert_preserved(f: &Fixture, root: &Event, source: &Event, reaction: &Event) {
    assert!(f.exists(root).await && f.exists(source).await && f.exists(reaction).await);
    assert_eq!(
        f.count("SELECT count(*) FROM reactions WHERE community_id=$1")
            .await,
        1
    );
    assert_eq!(
        f.count("SELECT count(*) FROM event_mentions WHERE community_id=$1")
            .await,
        1
    );
    assert_eq!(
        f.db.get_thread_summary(f.community, root.id.as_bytes())
            .await
            .unwrap()
            .unwrap()
            .reply_count,
        1
    );
}

async fn assert_retry(f: &Fixture, expected: &str) {
    let (status, current, finished, error): (String, bool, Option<DateTime<Utc>>, String) =
        sqlx::query_as("SELECT status,is_current,finished_at,last_error FROM entity_deletion_stage_history WHERE id=$1")
            .bind(f.stage.id).fetch_one(f.db.pool()).await.unwrap();
    assert_eq!(status, "failed");
    assert!(current && finished.is_none());
    assert!(error.contains(expected), "{error}");
    let (status, retries, scheduled): (String, i32, bool) = sqlx::query_as(
        "SELECT status,retry_count,next_attempt_at IS NOT NULL FROM entity_deletion_requests WHERE id=$1",
    ).bind(f.lease.request_id).fetch_one(f.db.pool()).await.unwrap();
    assert_eq!(status, "retry_wait");
    assert_eq!(retries, 1);
    assert!(scheduled);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn authored_nip30_emoji_preserves_image_reference_and_saves_retry() {
    let f = fixture().await;
    let source = f
        .event(
            &f.alice,
            9,
            ":party:",
            vec![tag(&["emoji", "party", "https://files.example/party.png"])],
            None,
        )
        .await;
    let reaction = f.reaction(&f.bob, &source).await;
    let error = f.batch(10).await.unwrap_err();
    assert!(error.to_string().contains("attachment"), "{error}");
    assert!(f.exists(&source).await && f.exists(&reaction).await);
    assert_eq!(
        f.count("SELECT count(*) FROM reactions WHERE community_id=$1")
            .await,
        1
    );
    assert_retry(&f, "attachment").await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn dependent_nip30_emoji_preserves_source_and_saves_retry() {
    let f = fixture().await;
    let (root, source, reaction) = conversation(&f).await;
    let emoji = f
        .event(
            &f.bob,
            7,
            ":party:",
            vec![
                tag(&["e", &source.id.to_hex()]),
                tag(&["emoji", "party", "https://files.example/party.png"]),
            ],
            None,
        )
        .await;
    let error = f.batch(10).await.unwrap_err();
    assert!(error.to_string().contains("file references"), "{error}");
    assert!(f.exists(&emoji).await);
    assert_preserved(&f, &root, &source, &reaction).await;
    assert_retry(&f, "file references").await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mismatched_saved_inputs_refuse_the_correct_approved_community() {
    let f = fixture().await;
    let (root, source, reaction) = conversation(&f).await;
    // Keep the target community, request, digest, approval and fence valid.
    // Only the saved input snapshot differs from the approved inventory.
    sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json=jsonb_set(metadata_json,'{inputs,target}','\"different-user\"') WHERE id=$1")
        .bind(f.stage.id).execute(f.db.pool()).await.unwrap();
    let error = f.batch(10).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("current approved purge_messages stage"),
        "{error}"
    );
    assert_preserved(&f, &root, &source, &reaction).await;
    assert_retry(&f, "current approved purge_messages stage").await;
}

async fn external_lease(f: &Fixture, community: CommunityId) {
    sqlx::query("INSERT INTO community_serving_write_leases (community_id,operation,owner,fence_generation,lease_until) VALUES ($1,'test-upload','external-worker',0,clock_timestamp()+interval '10 minutes')")
        .bind(community.as_uuid()).execute(f.db.pool()).await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn active_external_write_lease_refuses_only_its_community() {
    let f = fixture().await;
    let (root, source, reaction) = conversation(&f).await;
    external_lease(&f, f.community).await;
    let error = f.batch(10).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("external community writes must drain"),
        "{error}"
    );
    assert_preserved(&f, &root, &source, &reaction).await;
    assert_retry(&f, "external community writes must drain").await;

    // A live lease on another tenant must not stop this tenant's cleanup.
    let other = fixture().await;
    external_lease(&other, other.community).await;
    sqlx::query("DELETE FROM community_serving_write_leases WHERE community_id=$1")
        .bind(f.community.as_uuid())
        .execute(f.db.pool())
        .await
        .unwrap();
    sqlx::query(
        "UPDATE entity_deletion_requests SET next_attempt_at=clock_timestamp() WHERE id=$1",
    )
    .bind(f.lease.request_id)
    .execute(f.db.pool())
    .await
    .unwrap();
    f.finish(10).await;
    assert!(!f.exists(&source).await && !f.exists(&reaction).await);
    assert!(f.exists(&root).await);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn expiry_after_admission_rolls_back_deletions_and_count_repairs() {
    let f = fixture().await;
    let (root, source, reaction) = conversation(&f).await;
    let trigger = format!("expire_message_lease_{}", Uuid::new_v4().simple());
    // Inject expiry at count repair, after all admission checks and deletes.
    // No wall-clock sleep or race with the initial lease check is needed.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION {trigger}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
         UPDATE entity_deletion_requests SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id='{}';
         RETURN NEW; END $$;
         CREATE TRIGGER {trigger} AFTER UPDATE ON thread_metadata FOR EACH ROW
         WHEN (NEW.community_id='{}') EXECUTE FUNCTION {trigger}();",
        f.lease.request_id, f.community.as_uuid(),
    ))).execute(f.db.pool()).await.unwrap();
    let outcome = f.batch(10).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER {trigger} ON thread_metadata; DROP FUNCTION {trigger}();",
    )))
    .execute(f.db.pool())
    .await
    .unwrap();
    let error = outcome.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("lease expired during cleanup batch"),
        "{error}"
    );
    assert_preserved(&f, &root, &source, &reaction).await;
    // The injected deadline is transactional too. Rollback restores the live
    // lease, so this case can assert durable failure recording independently.
    assert_retry(&f, "lease expired during cleanup batch").await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn replacement_worker_reclaims_and_resumes_without_deleting_twice() {
    let f = fixture().await;
    let (root, first, reaction) = conversation(&f).await;
    let second = f
        .event(&f.alice, 9, "second reply", vec![], Some((&root, &root, 1)))
        .await;
    let batch = f.batch(1).await.unwrap();
    assert!(!batch.exhausted);
    assert_ne!(f.exists(&first).await, f.exists(&second).await);
    let remaining = if f.exists(&first).await {
        &first
    } else {
        &second
    };
    sqlx::query("UPDATE entity_deletion_requests SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1")
        .bind(f.lease.request_id).execute(f.db.pool()).await.unwrap();
    let restarted = Db::from_pool(PgPool::connect(&database_url()).await.unwrap());
    let successor = restarted
        .entity_deletion_store()
        .claim(
            f.lease.request_id,
            "replacement-worker",
            Duration::from_secs(600),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(successor.generation, f.lease.generation + 1);
    assert!(f
        .batch(1)
        .await
        .unwrap_err()
        .to_string()
        .contains("stale deletion lease"));
    assert!(f.exists(remaining).await);
    assert!(restarted
        .entity_deletion_store()
        .heartbeat(&f.lease, Duration::from_secs(600))
        .await
        .is_err());
    let mut exhausted = false;
    for _ in 0..3 {
        if handle(
            &restarted,
            &successor,
            &f.stage,
            &f.relay.public_key().to_bytes(),
            1,
        )
        .await
        .unwrap()
        .exhausted
        {
            exhausted = true;
            break;
        }
    }
    assert!(exhausted);
    assert!(!f.exists(&first).await && !f.exists(&second).await && !f.exists(&reaction).await);
    assert_eq!(
        f.db.get_thread_summary(f.community, root.id.as_bytes())
            .await
            .unwrap()
            .unwrap()
            .reply_count,
        0
    );
    let (status, current): (String, bool) =
        sqlx::query_as("SELECT status,is_current FROM entity_deletion_stage_history WHERE id=$1")
            .bind(f.stage.id)
            .fetch_one(f.db.pool())
            .await
            .unwrap();
    assert_eq!(status, "completed");
    assert!(current);
}
