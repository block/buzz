use super::*;
use crate::CreateCommunityWithOwnerResult;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cluster_global_concurrent_dm_creation_reports_only_the_inserter() {
    // Exercise both interactive opens racing each other, and a workflow's
    // visibility-preserving create racing an interactive open.
    for second_is_interactive in [true, false] {
        let url = crate::test_support::database_url();
        let db = Db::from_pool(PgPool::connect(&url).await.expect("database"));
        let author = nostr::Keys::generate().public_key().to_bytes();
        let recipient = nostr::Keys::generate().public_key().to_bytes();
        let community = match db
            .create_community_with_owner(
                &format!("dm-race-{}.example.org", Uuid::new_v4().simple()),
                &hex::encode(author),
                &nostr::Keys::generate(),
            )
            .await
            .expect("community")
            .outcome
        {
            CreateCommunityWithOwnerResult::Created(row) => row.id,
            other => panic!("unexpected community: {other:?}"),
        };
        for key in [&author, &recipient] {
            db.ensure_user(community, key).await.expect("user");
        }
        // Dedicated one-connection pools let the barrier identify both writers
        // exactly, without relying on scheduling or sleeping before the race.
        let first = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("first pool");
        let second = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("second pool");
        let mut pids = Vec::new();
        for pool in [&first, &second] {
            pids.push(
                sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
                    .fetch_one(pool)
                    .await
                    .expect("writer PID"),
            );
        }
        let mut barrier = db.pool().begin().await.expect("barrier");
        sqlx::query("LOCK TABLE channels IN SHARE MODE")
            .execute(&mut *barrier)
            .await
            .expect("block inserts, allow lookups");
        let first_pool = first.clone();
        let first_task =
            tokio::spawn(
                async move { open_dm(&first_pool, community, &[&recipient], &author).await },
            );
        let second_pool = second.clone();
        let second_task = tokio::spawn(async move {
            if second_is_interactive {
                open_dm(&second_pool, community, &[&recipient], &author).await
            } else {
                create_dm_if_missing(&second_pool, community, &[&author, &recipient], &author).await
            }
        });
        let both_at_insert = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_locks WHERE pid = ANY($1) \
                     AND relation = 'channels'::regclass AND NOT granted",
                )
                .bind(&pids)
                .fetch_one(db.pool())
                .await
                .expect("blocked writers");
                if waiting == 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        barrier.rollback().await.expect("release inserts");
        both_at_insert.expect("both callers must reach INSERT before the barrier is released");
        let (first_result, second_result) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(first_task, second_task)
        })
        .await
        .expect("writers finish");
        let (first_channel, first_created) = first_result.expect("first task").expect("first open");
        let (second_channel, second_created) =
            second_result.expect("second task").expect("second open");
        assert_eq!(first_channel.id, second_channel.id);
        assert_ne!(
            first_created, second_created,
            "exactly one caller created the DM"
        );
        assert_eq!(
            db.get_members(community, first_channel.id)
                .await
                .expect("members")
                .len(),
            2
        );

        db.hide_dm(community, first_channel.id, &author)
            .await
            .expect("hide");
        let (reused, created) = db
            .create_dm_if_missing(community, &[&author, &recipient], &author)
            .await
            .expect("workflow reuse");
        assert_eq!(reused.id, first_channel.id);
        assert!(!created);
        assert!(db
            .list_hidden_dms(community, &author)
            .await
            .expect("hidden DMs")
            .contains(&reused.id));
        let (reopened, created) = db
            .open_dm(community, &[&recipient], &author)
            .await
            .expect("interactive reuse");
        assert_eq!(reopened.id, reused.id);
        assert!(!created);
        assert!(!db
            .list_hidden_dms(community, &author)
            .await
            .expect("hidden DMs")
            .contains(&reused.id));
        first.close().await;
        second.close().await;
        db.pool().close().await;
    }
}
