//! A read that does not name a channel returns only the legacy channel types
//! (NIP-CL). The relay must leave out a channel type that it adds later, not
//! only `system`: a client sees a channel only if it knows the channel's type.

use crate::Db;
use buzz_core::CommunityId;
use nostr::Keys;
use sqlx::PgPool;
use uuid::Uuid;

/// A channel type that no client knows. The test adds it to the enum of a
/// scratch database, as a later migration would.
pub(super) const FUTURE_TYPE: &str = "future_type";

/// A migrated scratch database with [`FUTURE_TYPE`] in `channel_type`, so the
/// shared test database keeps its schema. Returns `(db, admin, name)`.
pub(super) async fn scratch_db_with_future_type() -> (Db, PgPool, String) {
    let url = crate::test_support::database_url();
    let admin = PgPool::connect(&url).await.expect("connect admin");
    let name = format!("legacy_types_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&admin)
        .await
        .expect("create scratch db");
    let base = &url[..url.rfind('/').expect("db url has a path")];
    let pool = PgPool::connect(&format!("{base}/{name}"))
        .await
        .expect("connect scratch db");
    crate::migration::run_migrations(&pool)
        .await
        .expect("migrate scratch db");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER TYPE channel_type ADD VALUE '{FUTURE_TYPE}'"
    )))
    .execute(&pool)
    .await
    .expect("add future channel type");
    (Db::from_pool(pool), admin, name)
}

async fn insert_channel(
    pool: &PgPool,
    community: CommunityId,
    channel_type: &str,
    visibility: &str,
    member: Option<&Keys>,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO channels(community_id,id,name,visibility,channel_type,created_by)
         VALUES($1,$2,$3,$4::channel_visibility,$5::channel_type,$6)",
    )
    .bind(community.as_uuid())
    .bind(id)
    .bind(format!("{channel_type}-{visibility}"))
    .bind(visibility)
    .bind(channel_type)
    .bind(Keys::generate().public_key().to_bytes().as_slice())
    .execute(pool)
    .await
    .expect("insert channel");
    if let Some(member) = member {
        sqlx::query(
            "INSERT INTO channel_members(community_id,channel_id,pubkey,role)
             VALUES($1,$2,$3,'member')",
        )
        .bind(community.as_uuid())
        .bind(id)
        .bind(member.public_key().to_bytes().as_slice())
        .execute(pool)
        .await
        .expect("insert member");
    }
    id
}

/// Mutation: put back `channel_type <> 'system'` in
/// `get_accessible_channel_ids`, `get_accessible_channels` or the sidebar
/// roster → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn reads_that_name_no_channel_leave_out_a_type_added_later() {
    let (db, admin, name) = scratch_db_with_future_type().await;
    let pool = db.pool.clone();
    let community = CommunityId::from_uuid(Uuid::new_v4());
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(community.as_uuid())
        .bind(format!("legacy-types-{}.test", community.as_uuid()))
        .execute(&pool)
        .await
        .expect("insert community");
    let reader = Keys::generate();
    sqlx::query(
        "INSERT INTO personal_read_accounts(community_id,actor,started_at)
         VALUES($1,$2,'1900-01-01T00:00:00Z')",
    )
    .bind(community.as_uuid())
    .bind(reader.public_key().to_bytes().as_slice())
    .execute(&pool)
    .await
    .expect("start read account");

    let mut legacy = Vec::new();
    for channel_type in buzz_core::channel::LEGACY_CHANNEL_TYPES {
        legacy.push(insert_channel(&pool, community, channel_type, "private", Some(&reader)).await);
    }
    let open_legacy = insert_channel(&pool, community, "stream", "open", None).await;
    for (visibility, member) in [("open", None), ("private", Some(&reader))] {
        insert_channel(&pool, community, FUTURE_TYPE, visibility, member).await;
        insert_channel(&pool, community, "system", visibility, member).await;
    }
    let mut expected = legacy.clone();
    expected.push(open_legacy);
    expected.sort();

    let reader_bytes = reader.public_key().to_bytes();
    let mut ids = db
        .get_accessible_channel_ids(community, &reader_bytes)
        .await
        .expect("accessible channel ids");
    ids.sort();
    assert_eq!(ids, expected, "get_accessible_channel_ids");

    let mut listed: Vec<Uuid> = db
        .get_accessible_channels(community, &reader_bytes, None, None)
        .await
        .expect("accessible channels")
        .into_iter()
        .map(|channel| channel.channel.id)
        .collect();
    listed.sort();
    assert_eq!(listed, expected, "get_accessible_channels");

    let mut joined: Vec<Uuid> = db
        .personal_read_sidebar(
            community,
            &reader.public_key(),
            crate::store::personal_read::DEFAULT_RETENTION_SECONDS,
            20,
            None,
        )
        .await
        .expect("sidebar")
        .channels
        .into_iter()
        .map(|channel| channel.channel_id)
        .collect();
    joined.sort();
    legacy.sort();
    assert_eq!(joined, legacy, "sidebar roster");

    db.pool.close().await;
    let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
    )))
    .execute(&admin)
    .await;
}
