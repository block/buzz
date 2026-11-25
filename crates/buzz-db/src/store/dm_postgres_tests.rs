use super::*;
use crate::CreateCommunityWithOwnerResult;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

const ALLOW: AgentAddPolicy<'static> = &|_: &str, _: &[u8]| Ok(());

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
        let first_task = tokio::spawn(async move {
            open_dm(&first_pool, community, &[&recipient], &author, false, ALLOW).await
        });
        let second_pool = second.clone();
        let second_task = tokio::spawn(async move {
            if second_is_interactive {
                open_dm(
                    &second_pool,
                    community,
                    &[&recipient],
                    &author,
                    false,
                    ALLOW,
                )
                .await
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
            .open_dm(community, &[&recipient], &author, false, ALLOW)
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

/// A new community with `owner`, and every key in `users` known to it.
async fn community_with_users(db: &Db, owner: &[u8; 32], users: &[&[u8; 32]]) -> CommunityId {
    let community = match db
        .create_community_with_owner(
            &format!("dm-self-{}.example.org", Uuid::new_v4().simple()),
            &hex::encode(owner),
            &nostr::Keys::generate(),
        )
        .await
        .expect("community")
        .outcome
    {
        CreateCommunityWithOwnerResult::Created(row) => row.id,
        other => panic!("unexpected community: {other:?}"),
    };
    for key in users {
        db.ensure_user(community, *key).await.expect("user");
    }
    community
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn self_dm_is_created_once_and_reopened() {
    let url = crate::test_support::database_url();
    let db = Db::from_pool(PgPool::connect(&url).await.expect("database"));
    let me = nostr::Keys::generate().public_key().to_bytes();
    let other = nostr::Keys::generate().public_key().to_bytes();
    let community = community_with_users(&db, &me, &[&me, &other]).await;

    // Naming only oneself opens a DM with one member: oneself.
    let (created_dm, created) = db
        .open_dm(community, &[&me], &me, false, ALLOW)
        .await
        .expect("open self DM");
    assert!(created);
    assert_eq!(created_dm.channel_type, "dm");
    assert_eq!(created_dm.name, "DM");
    let members = db
        .get_members(community, created_dm.id)
        .await
        .expect("members");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].pubkey, me);

    // Opening it again, either way, finds the same DM and creates nothing.
    for by_people in [false, true] {
        let (again, created) = db
            .open_dm(community, &[&me], &me, by_people, ALLOW)
            .await
            .expect("reopen self DM");
        assert_eq!(again.id, created_dm.id);
        assert!(!created);
    }

    // It is the DM a workflow finds, and it is not a DM with anyone else.
    let (found, created) = db
        .create_dm_if_missing(community, &[&me], &me)
        .await
        .expect("workflow reuse");
    assert_eq!(found.id, created_dm.id);
    assert!(!created);
    let (with_other, created) = db
        .open_dm(community, &[&other], &me, false, ALLOW)
        .await
        .expect("open DM with someone else");
    assert!(created);
    assert_ne!(with_other.id, created_dm.id);

    // Hiding it, then opening it again, shows it again.
    db.hide_dm(community, created_dm.id, &me)
        .await
        .expect("hide");
    assert!(db
        .list_hidden_dms(community, &me)
        .await
        .expect("hidden DMs")
        .contains(&created_dm.id));
    let (reopened, created) = db
        .open_dm(community, &[&me], &me, false, ALLOW)
        .await
        .expect("reopen hidden self DM");
    assert_eq!(reopened.id, created_dm.id);
    assert!(!created);
    assert!(!db
        .list_hidden_dms(community, &me)
        .await
        .expect("hidden DMs")
        .contains(&created_dm.id));

    // Each person has their own, and it lists with them as its only
    // participant.
    let (others_own, created) = db
        .open_dm(community, &[&other], &other, false, ALLOW)
        .await
        .expect("open the other person's self DM");
    assert!(created);
    assert_ne!(others_own.id, created_dm.id);
    let listed = list_dms_for_user(db.pool(), community, &me, 50, None)
        .await
        .expect("list DMs");
    let mine = listed
        .iter()
        .find(|dm| dm.channel_id == created_dm.id)
        .expect("self DM is listed");
    assert_eq!(mine.participants.len(), 1);
    assert_eq!(mine.participants[0].pubkey, me);
    db.pool().close().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn dm_still_needs_someone_to_message() {
    let url = crate::test_support::database_url();
    let db = Db::from_pool(PgPool::connect(&url).await.expect("database"));
    let me = nostr::Keys::generate().public_key().to_bytes();
    let other = nostr::Keys::generate().public_key().to_bytes();
    let community = community_with_users(&db, &me, &[&me, &other]).await;

    // No one named: not a DM with oneself.
    assert!(matches!(
        db.open_dm(community, &[], &me, false, ALLOW).await,
        Err(DbError::InvalidData(_))
    ));
    // A lone participant must be the creator.
    assert!(matches!(
        db.create_dm_if_missing(community, &[&other], &me).await,
        Err(DbError::InvalidData(_))
    ));
    assert!(matches!(
        db.create_dm_if_missing(community, &[], &me).await,
        Err(DbError::InvalidData(_))
    ));
    // A malformed pubkey is still refused.
    assert!(matches!(
        db.open_dm(community, &[&me[..31]], &me, false, ALLOW).await,
        Err(DbError::InvalidData(_))
    ));
    db.pool().close().await;
}
