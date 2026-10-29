use super::*;

/// Exercise the actual TTL-before-author settings writer against a message
/// holding the author's shared lock and a queued exclusive fence. PostgreSQL
/// may reorder soft wait edges; neither an error nor a swallowed TTL deadlock
/// is acceptable evidence of safe completion.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn message_then_fence_then_ttl_settings_complete_without_deadlock() {
    let f = Fixture::new().await;
    let user = Keys::generate();
    let channel = Uuid::new_v4();
    f.db.create_channel_with_id(
        f.community,
        channel,
        "lock-order",
        crate::channel::ChannelType::Stream,
        crate::channel::ChannelVisibility::Open,
        None,
        user.public_key().as_bytes(),
        Some(60),
    )
    .await
    .unwrap();
    let mut message_tx = crate::begin_community_event_write_transaction(
        &f.pool,
        f.community,
        crate::observability::WriterOperation::EventWrite,
    )
    .await
    .unwrap();
    let first = message(&user, 9);
    event::insert_event_in_transaction(&mut message_tx, &first, Some(channel))
        .await
        .unwrap();

    let mut gate = f.pool.begin().await.unwrap();
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    let identity = FenceIdentity::user(user.public_key());
    let lease = f.lease.clone();
    let fence_task = tokio::spawn(async move {
        fences::insert(&mut gate, identity, &lease, None).await?;
        gate.commit().await?;
        crate::Result::Ok(())
    });
    wait_for_waiter(&f.pool, gate_pid).await;

    // A one-connection pool lets us observe the settings writer's actual PID.
    let settings_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let settings_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&settings_pool)
        .await
        .unwrap();
    let settings_db = Db::from_pool(settings_pool);
    let settings_event = message(&user, 9002);
    let settings_id = settings_event.id;
    let community = f.community;
    let settings_task = tokio::spawn(async move {
        let mut edit = crate::channel::ChannelMetadataEdit::default();
        edit.columns.ttl_seconds = Some(Some(3600));
        settings_db
            .apply_channel_metadata_event(community, channel, &settings_event, edit, &[])
            .await
    });
    wait_for_waiter(&f.pool, settings_pid).await;
    // The settings transaction really holds the exclusive TTL lock, rather
    // than merely waiting on some earlier lock in its path.
    let holds_ttl: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE pid=$1 AND locktype='advisory' \
         AND mode='ExclusiveLock' AND granted \
         AND classid=((hashtextextended($2,0) >> 32) & 4294967295)::oid \
         AND objid=(hashtextextended($2,0) & 4294967295)::oid)",
    )
    .bind(settings_pid)
    .bind(crate::channel::channel_ttl_lock_key(f.community, channel))
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert!(holds_ttl);

    tokio::time::timeout(Duration::from_secs(8), async {
        message_tx.commit().await.unwrap();
        fence_task.await.unwrap().unwrap();
        let settings = settings_task.await.unwrap();
        let accepted = match settings {
            Ok(write) => {
                assert!(write.inserted);
                true
            }
            Err(DbError::UserDeletionRestricted) => false,
            Err(other) => panic!("settings failed for a non-fence reason: {other:?}"),
        };
        assert_eq!(
            f.db.get_event_by_id(f.community, settings_id.as_bytes())
                .await
                .unwrap()
                .is_some(),
            accepted
        );
    })
    .await
    .expect("all three production transactions must finish");
    assert!(f
        .db
        .get_event_by_id(f.community, first.id.as_bytes())
        .await
        .unwrap()
        .is_some());
    restricted(
        f.db.insert_event(f.community, &message(&user, 9), Some(channel))
            .await,
    );
    let deadlocks: i64 = sqlx::query_scalar(
        "SELECT deadlocks FROM pg_stat_database WHERE datname=current_database()",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(deadlocks, 0, "include deadlocks swallowed by TTL refresh");
}
