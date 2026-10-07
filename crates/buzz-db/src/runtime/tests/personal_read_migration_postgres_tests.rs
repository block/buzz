use super::*;

const SCOPE_INDEX: &str = "CREATE INDEX CONCURRENTLY idx_event_mentions_scope_received \
     ON event_mentions (community_id,pubkey_hex,channel_id,root_id,received_at)";
const SCOPE_COLUMNS: &str = "ALTER TABLE event_mentions \
     ADD COLUMN IF NOT EXISTS received_at TIMESTAMPTZ, ADD COLUMN IF NOT EXISTS root_id BYTEA";

async fn regclass_oid(pool: &PgPool, name: &str) -> Option<i64> {
    sqlx::query_scalar("SELECT to_regclass($1)::oid::bigint")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn migrated_version(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The documented brownfield path for 0057: prebuild the event_mentions
/// index concurrently and pre-drop the root index. The migration must keep
/// the prebuilt index, reject a wrong or invalid one without recording the
/// version, and bound its budgets like 0049.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_personal_read_prebuild_validation() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db_through(&admin, "pr_prebuild", Some(56)).await;
    sqlx::raw_sql(SCOPE_COLUMNS).execute(&pool).await.unwrap();
    for setup in [
        "CREATE INDEX idx_event_mentions_scope_received \
         ON event_mentions (community_id,pubkey_hex,channel_id,received_at)",
        // Simulate an invalid concurrent-build remnant only in this disposable superuser DB.
        "CREATE INDEX idx_event_mentions_scope_received \
         ON event_mentions (community_id,pubkey_hex,channel_id,root_id,received_at); \
         UPDATE pg_index SET indisvalid=false \
         WHERE indexrelid='idx_event_mentions_scope_received'::regclass",
    ] {
        sqlx::raw_sql(setup).execute(&pool).await.unwrap();
        let error = migration::run_migrations(&pool).await.unwrap_err();
        assert!(
            error.to_string().contains("invalid or wrong definition"),
            "must reject catalog shape, not merely time out: {error}"
        );
        assert_eq!(migrated_version(&pool).await, 56);
        sqlx::query("DROP INDEX CONCURRENTLY idx_event_mentions_scope_received")
            .execute(&pool)
            .await
            .unwrap();
    }

    sqlx::query(SCOPE_INDEX).execute(&pool).await.unwrap();
    sqlx::query("DROP INDEX CONCURRENTLY IF EXISTS public.idx_thread_metadata_root")
        .execute(&pool)
        .await
        .unwrap();
    let oid = regclass_oid(&pool, "idx_event_mentions_scope_received").await;
    assert!(oid.is_some());
    migration::run_migrations(&pool).await.unwrap();
    assert_eq!(
        oid,
        regclass_oid(&pool, "idx_event_mentions_scope_received").await
    );
    assert_eq!(regclass_oid(&pool, "idx_thread_metadata_root").await, None);
    assert!(migrated_version(&pool).await >= 57);
    drop_scratch_db(&admin, pool, &name).await;
}

/// Without a prebuild, 0057 builds the index and drops the root index itself.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_personal_read_fresh_build_and_budgets() {
    let migration = include_str!("../../../../../migrations/0057_personal_read_scopes.sql");
    assert!(migration.contains("SET LOCAL lock_timeout = '1s'"));
    assert!(migration.contains("SET LOCAL statement_timeout = '5s'"));

    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db_through(&admin, "pr_fresh", Some(56)).await;
    assert!(regclass_oid(&pool, "idx_thread_metadata_root")
        .await
        .is_some());
    migration::run_migrations(&pool).await.unwrap();
    assert!(regclass_oid(&pool, "idx_event_mentions_scope_received")
        .await
        .is_some());
    assert_eq!(regclass_oid(&pool, "idx_thread_metadata_root").await, None);
    drop_scratch_db(&admin, pool, &name).await;
}

/// Under 0056 only a read intent creates an account, so 0057 starts every
/// existing account at the migration. A message that arrives after upgrade
/// counts without a further intent. Accounts in a community being deleted
/// are skipped, so the write fence cannot fail the migration.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_personal_read_starts_existing_accounts() {
    use crate::{
        channel::{ChannelType, ChannelVisibility},
        Db,
    };
    use nostr::{EventBuilder, Keys, Kind, Tag};

    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db_through(&admin, "pr_upgrade", Some(56)).await;
    let actor = Keys::generate();
    let actor_bytes = actor.public_key().to_bytes();
    let active: Uuid =
        sqlx::query_scalar("INSERT INTO communities (host) VALUES ($1) RETURNING id")
            .bind(format!("upgrade-{}.local", Uuid::new_v4()))
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO personal_read_accounts (community_id,actor) VALUES ($1,$2)")
        .bind(active)
        .bind(actor_bytes.as_slice())
        .execute(&pool)
        .await
        .unwrap();
    // A fenced community's account, set up past its fence only in this
    // disposable superuser DB.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await
        .unwrap();
    let fenced: Uuid = sqlx::query_scalar(
        "INSERT INTO communities (host,deletion_state) VALUES ($1,'fenced') RETURNING id",
    )
    .bind(format!("fenced-{}.local", Uuid::new_v4()))
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO personal_read_accounts (community_id,actor) VALUES ($1,$2)")
        .bind(fenced)
        .bind(actor_bytes.as_slice())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    migration::run_migrations(&pool).await.unwrap();
    let started = |community: Uuid| {
        sqlx::query_scalar::<_, bool>(
            "SELECT started_at IS NOT NULL FROM personal_read_accounts
             WHERE community_id=$1 AND actor=$2",
        )
        .bind(community)
        .bind(actor_bytes.as_slice())
        .fetch_one(&pool)
    };
    assert!(started(active).await.unwrap(), "existing account starts");
    assert!(!started(fenced).await.unwrap(), "fenced account is skipped");

    let db = Db::from_pool(pool.clone());
    let community = CommunityId::from_uuid(active);
    let channel = db
        .create_channel(
            community,
            "upgraded",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &actor_bytes,
            None,
        )
        .await
        .unwrap()
        .id;
    let event = EventBuilder::new(Kind::Custom(9), "after upgrade")
        .tags(vec![Tag::public_key(actor.public_key())])
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &event, Some(channel))
        .await
        .unwrap();
    let row = db
        .personal_read_sidebar(community, &actor.public_key(), 20, None)
        .await
        .unwrap()
        .channels
        .remove(0);
    assert_eq!(
        (row.unread, row.mentions),
        (true, 1),
        "an arrival after upgrade counts before the next intent"
    );
    drop(db);
    drop_scratch_db(&admin, pool, &name).await;
}
