//! PostgreSQL tests for relay-provisioned system channels.
use crate::Db;
use buzz_core::CommunityId;
use nostr::Keys;
use sqlx::PgPool;
use uuid::Uuid;

async fn db() -> Db {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
        crate::migration::run_migrations(&pool).await.unwrap();
    }
    Db::from_pool(pool)
}

async fn community(db: &Db) -> CommunityId {
    db.ensure_configured_community(&format!("provisioned-{}.test", Uuid::new_v4().simple()))
        .await
        .unwrap()
        .id
}

async fn provisioned_count(db: &Db, community: CommunityId, label: &str, relay: &[u8]) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM channels WHERE community_id=$1 AND created_by=$2 AND $3 = ANY(labels) AND channel_type='system' AND archived_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(relay)
    .bind(label)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// Repeated and concurrent ensures leave one channel. Mutation: drop the
/// advisory lock → the concurrent calls each insert one → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn ensure_is_idempotent_under_concurrency() {
    let db = db().await;
    let community = community(&db).await;
    let relay = Keys::generate().public_key().to_bytes();
    let label = "example-records";
    let calls = (0..8).map(|_| db.ensure_provisioned_system_channel(community, label, &relay));
    let results = futures_util::future::join_all(calls).await;
    let ids: std::collections::HashSet<Uuid> =
        results.into_iter().map(|r| r.unwrap().channel.id).collect();
    assert_eq!(ids.len(), 1);
    let again = db
        .ensure_provisioned_system_channel(community, label, &relay)
        .await
        .unwrap();
    assert!(!again.created);
    assert!(ids.contains(&again.channel.id));
    assert_eq!(provisioned_count(&db, community, label, &relay).await, 1);
    let channel = again.channel;
    assert_eq!(channel.channel_type, "system");
    assert_eq!(channel.visibility, "open");
    assert_eq!(channel.labels, vec![label.to_string()]);
    assert_eq!(channel.created_by, relay.to_vec());
}

/// Another key's channel with the label doesn't count, and an archived
/// provisioned channel is replaced.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn ensure_ignores_other_creators_and_replaces_archived() {
    let db = db().await;
    let community = community(&db).await;
    let relay = Keys::generate().public_key().to_bytes();
    let member = Keys::generate().public_key().to_bytes();
    let label = "example-records";
    db.create_labeled_channel(
        community,
        "decoy",
        crate::channel::ChannelType::System,
        crate::channel::ChannelVisibility::Open,
        None,
        &member,
        None,
        &[label.to_string()],
        false,
    )
    .await
    .unwrap();
    let first = db
        .ensure_provisioned_system_channel(community, label, &relay)
        .await
        .unwrap();
    assert!(first.created, "a member's channel is not the relay's");
    assert!(!first.has_metadata);
    db.archive_channel(community, first.channel.id)
        .await
        .unwrap();
    let missing = db
        .communities_missing_provisioned_channel(label, &relay, false, None, 10_000)
        .await
        .unwrap();
    assert!(missing.iter().any(|(id, _)| *id == community));
    let second = db
        .ensure_provisioned_system_channel(community, label, &relay)
        .await
        .unwrap();
    assert!(second.created);
    assert_ne!(second.channel.id, first.channel.id);
    let after = db
        .communities_missing_provisioned_channel(label, &relay, false, None, 10_000)
        .await
        .unwrap();
    assert!(!after.iter().any(|(id, _)| *id == community));
    let without_metadata = db
        .communities_missing_provisioned_channel(label, &relay, true, None, 10_000)
        .await
        .unwrap();
    assert!(
        without_metadata.iter().any(|(id, _)| *id == community),
        "no relay-signed kind:39000 yet"
    );
}
