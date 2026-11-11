//! The recent-pass snapshot fixture. It writes `events` partitions directly,
//! so it lives in a standalone test module.

/// Commit a row replacement while a read split at the recent-pass floor (see
/// `recent_pass`) is between its passes.
///
/// Seeds `channel` with two kind:9 rows above the floor, `old` below it,
/// `ancient` in the partition before every dated monthly, and `new`: dated
/// now, soft-deleted until the replacement. Another session locks the
/// ancient partition, which only the older pass plans, so `read` stops
/// between its passes. The replacement then commits in one transaction, as
/// every replacement does: it deletes `old` and makes `new` live. The lock is
/// released and `read` finishes.
///
/// Returns `read`'s result and the page its first pass's snapshot holds:
/// `[recent, recent, old, ancient]`.
pub(crate) async fn replace_between_passes<F, Fut, T>(
    pool: &sqlx::PgPool,
    community: buzz_core::CommunityId,
    channel: uuid::Uuid,
    read: F,
) -> (T, Vec<nostr::EventId>)
where
    F: FnOnce(sqlx::pool::PoolConnection<sqlx::Postgres>) -> Fut,
    Fut: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    use nostr::{EventBuilder, Keys, Kind, Timestamp};

    let db = crate::Db::from_pool(pool.clone());
    let author = Keys::generate();
    let now = chrono::Utc::now();
    let floor = crate::store::recent_pass::floor_at(now).timestamp() as u64;
    let now = now.timestamp() as u64;
    let insert = |at: u64, label: &str| {
        let event = EventBuilder::new(Kind::Custom(9), format!("{label} at {at}"))
            .custom_created_at(Timestamp::from(at))
            .sign_with_keys(&author)
            .expect("sign");
        let db = db.clone();
        async move {
            db.insert_event(community, &event, Some(channel))
                .await
                .expect("insert");
            event.id
        }
    };
    let snapshot_page = vec![
        insert(now - 10, "recent").await,
        insert(now - 20, "recent").await,
        insert(floor - 86_400, "old").await,
        insert(1_748_736_000, "ancient").await, // 2025-06-01
    ];
    let new = insert(now - 5, "new").await;
    let partition_of = |id: nostr::EventId| async move {
        sqlx::query_scalar::<_, String>("SELECT tableoid::regclass::text FROM events WHERE id = $1")
            .bind(id.as_bytes().to_vec())
            .fetch_one(pool)
            .await
            .expect("partition")
    };
    let old = snapshot_page[2];
    let (old_partition, ancient_partition, new_partition) = (
        partition_of(old).await,
        partition_of(snapshot_page[3]).await,
        partition_of(new).await,
    );
    assert_ne!(
        old_partition, ancient_partition,
        "the replacement must not wait on the lock"
    );
    assert_ne!(
        new_partition, ancient_partition,
        "the replacement must not wait on the lock"
    );
    sqlx::query("UPDATE events SET deleted_at = now() WHERE id = $1")
        .bind(new.as_bytes().to_vec())
        .execute(pool)
        .await
        .expect("hide the replacement");

    let mut locker = pool.begin().await.expect("begin locker");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "LOCK TABLE {ancient_partition} IN ACCESS EXCLUSIVE MODE"
    )))
    .execute(&mut *locker)
    .await
    .expect("lock the ancient partition");
    let conn = pool.acquire().await.expect("acquire reader");
    let mut task = tokio::spawn(read(conn));
    let waiting = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                // `pg_locks` spans every database, and the per-test databases
                // are clones that share relation OIDs: match this one only.
                "SELECT EXISTS(SELECT 1 FROM pg_locks WHERE relation = $1::text::regclass \
                 AND NOT granted \
                 AND database = (SELECT oid FROM pg_database WHERE datname = current_database()))",
            )
            .bind(&ancient_partition)
            .fetch_one(&mut *locker)
            .await
            .expect("inspect the older pass");
            if blocked {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    if waiting.is_err() {
        task.abort();
        panic!("the read never reached its older pass");
    }

    let mut replace = pool.begin().await.expect("begin replacement");
    for (partition, deleted_at, id) in [
        (&old_partition, "now()", old),
        (&new_partition, "NULL", new),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {partition} SET deleted_at = {deleted_at} WHERE id = $1"
        )))
        .bind(id.as_bytes().to_vec())
        .execute(&mut *replace)
        .await
        .expect("replace");
    }
    replace.commit().await.expect("commit replacement");
    locker.rollback().await.expect("release the lock");

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), &mut task).await;
    if result.is_err() {
        task.abort();
        panic!("the read did not finish after the lock was released");
    }
    (result.unwrap().expect("read task"), snapshot_page)
}
