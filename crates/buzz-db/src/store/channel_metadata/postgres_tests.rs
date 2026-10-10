//! Exercise the actual admitted boundary against isolated PostgreSQL databases.

mod activation_postgres_tests;
mod lifecycle_postgres_tests;

use buzz_core::{
    channel::{ChannelType, ChannelVisibility},
    CommunityId,
};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use super::{ChannelCreate, ChannelMetadataWrite, CommandApplication};
use crate::{Db, DbConfig};

async fn database(migration: bool) -> (Db, CommunityId) {
    let db = Db::new(&DbConfig {
        database_url: crate::test_support::database_url(),
        ..DbConfig::default()
    })
    .await
    .unwrap();
    if migration {
        crate::migration::run_migrations(db.pool()).await.unwrap();
    }
    let community = CommunityId::from_uuid(Uuid::new_v4());
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community.as_uuid())
        .bind(format!("labels-{}.example", community.as_uuid()))
        .execute(db.pool())
        .await
        .unwrap();
    (db, community)
}

fn command(keys: &Keys, channel: Uuid, kind: u16, operations: &[(&str, &str)]) -> Event {
    let mut tags = vec![Tag::parse(["h", &channel.to_string()]).unwrap()];
    tags.extend(
        operations
            .iter()
            .map(|(key, value)| Tag::parse([*key, *value]).unwrap()),
    );
    EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

fn fields() -> ChannelCreate {
    ChannelCreate {
        name: "labels".into(),
        channel_type: ChannelType::Stream,
        visibility: ChannelVisibility::Private,
        description: Some("canonical description".into()),
        ttl_seconds: None,
    }
}

async fn snapshot(write: &mut ChannelMetadataWrite, relay: &Keys) -> Event {
    let tags = write.snapshot_tags().await.unwrap();
    let timestamp = write.snapshot_timestamp().unwrap();
    let relay = relay.clone();
    let event = tokio::task::spawn_blocking(move || {
        EventBuilder::new(Kind::Custom(39000), "")
            .tags(tags)
            .allow_self_tagging()
            .custom_created_at(timestamp)
            .sign_with_keys(&relay)
            .unwrap()
    })
    .await
    .unwrap();
    write.store_snapshot(&event, 512 * 1024).await.unwrap();
    event
}

async fn apply(db: &Db, community: CommunityId, relay: &Keys, event: &Event) -> Option<Event> {
    let channel = buzz_core::channel_labels::LabelCommand::parse(event)
        .unwrap()
        .unwrap()
        .channel();
    let mut write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    assert_eq!(
        write.apply_command(event, Some(&fields())).await.unwrap(),
        CommandApplication::Applied
    );
    if write.needs_snapshot().await.unwrap() {
        snapshot(&mut write, relay).await;
    }
    write.commit().await.unwrap().map(|stored| stored.event)
}

async fn evidence(db: &Db, community: CommunityId, event: &Event) -> Option<bool> {
    sqlx::query_scalar("SELECT nip_cl_applied FROM events WHERE community_id = $1 AND id = $2")
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_optional(db.pool())
        .await
        .unwrap()
}

async fn atomic_create_contract(migration: bool) {
    let (db, community) = database(migration).await;
    let owner = Keys::generate();
    let relay = Keys::generate();
    let channel = Uuid::new_v4();
    let create = command(
        &owner,
        channel,
        9007,
        &[("name", "labels"), ("label", "team:infra")],
    );
    let mut write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    assert_eq!(
        write.apply_command(&create, Some(&fields())).await.unwrap(),
        CommandApplication::Applied
    );
    // Missing snapshot must make commit impossible, with no torn channel/owner/receipt.
    assert!(write.commit().await.is_err());
    assert!(db.get_channel(community, channel).await.is_err());
    assert_eq!(evidence(&db, community, &create).await, None);
    let members: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM channel_members WHERE community_id=$1 AND channel_id=$2",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(members, 0);

    let published = apply(&db, community, &relay, &create).await.unwrap();
    assert_eq!(evidence(&db, community, &create).await, Some(true));
    assert!(db
        .is_member(community, channel, owner.public_key().as_bytes())
        .await
        .unwrap());
    assert_eq!(
        buzz_core::channel_labels::verify_snapshot(&published, relay.public_key(), channel)
            .unwrap()
            .values(),
        &["team:infra"]
    );
    // A separate create must conflict, never recover somebody else's UUID as success.
    let conflict = command(&owner, channel, 9007, &[("name", "other")]);
    let mut write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    assert_eq!(
        write
            .apply_command(&conflict, Some(&fields()))
            .await
            .unwrap(),
        CommandApplication::Conflict
    );
    write.rollback().await.unwrap();
    assert_eq!(evidence(&db, community, &conflict).await, None);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn create_is_atomic_on_desired_schema() {
    atomic_create_contract(false).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_create_is_atomic() {
    atomic_create_contract(true).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn noop_and_exact_retry_preserve_head_and_soft_deleted_evidence() {
    let (db, community) = database(false).await;
    let owner = Keys::generate();
    let relay = Keys::generate();
    let channel = Uuid::new_v4();
    let create = command(&owner, channel, 9007, &[("label", "a")]);
    let original = apply(&db, community, &relay, &create).await.unwrap();
    let noop = command(&owner, channel, 9002, &[("add-label", "a")]);
    assert!(apply(&db, community, &relay, &noop).await.is_none());
    assert_eq!(evidence(&db, community, &noop).await, Some(true));
    let write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    assert_eq!(write.previous.as_ref().unwrap().event.id, original.id);
    write.rollback().await.unwrap();
    let remove = command(&owner, channel, 9002, &[("remove-label", "a")]);
    let removed = apply(&db, community, &relay, &remove).await.unwrap();
    assert!(removed.created_at > original.created_at);
    db.soft_delete_event_and_update_thread(community, noop.id.as_bytes(), None, None)
        .await
        .unwrap();
    for event in [&noop, &create] {
        let mut write = db
            .begin_channel_metadata_write(community, channel, relay.public_key())
            .await
            .unwrap();
        assert_eq!(
            write.apply_command(event, Some(&fields())).await.unwrap(),
            CommandApplication::Committed
        );
        assert!(write.labels().is_empty());
        assert_eq!(write.previous.as_ref().unwrap().event.id, removed.id);
        assert!(write.commit().await.unwrap().is_none());
    }
    // Delayed ordinary publication captures the current empty set, not old tags.
    db.set_topic(
        community,
        channel,
        "fresh topic",
        owner.public_key().as_bytes(),
    )
    .await
    .unwrap();
    let mut write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    assert!(write.needs_snapshot().await.unwrap());
    let repaired = snapshot(&mut write, &relay).await;
    assert!(repaired.created_at > removed.created_at);
    assert!(
        buzz_core::channel_labels::verify_snapshot(&repaired, relay.public_key(), channel)
            .unwrap()
            .is_empty()
    );
    write.commit().await.unwrap();
    db.soft_delete_channel(community, channel).await.unwrap();
    let mut write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    assert!(write.needs_snapshot().await.is_err());
    assert_eq!(
        write.apply_command(&noop, None).await.unwrap(),
        CommandApplication::Restricted
    );
    write.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_adds_preserve_values_and_retry_authority_is_fresh() {
    let (db, community) = database(false).await;
    let owner = Keys::generate();
    let relay = Keys::generate();
    let channel = Uuid::new_v4();
    let create = command(&owner, channel, 9007, &[]);
    apply(&db, community, &relay, &create).await;
    let first = command(&owner, channel, 9002, &[("add-label", "a")]);
    let second = command(&owner, channel, 9002, &[("add-label", "b")]);
    let (a, b) = tokio::join!(
        apply(&db, community, &relay, &first),
        apply(&db, community, &relay, &second)
    );
    assert!(a.is_some() && b.is_some());
    let write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    assert_eq!(write.labels().values(), &["a", "b"]);
    write.rollback().await.unwrap();
    sqlx::query("UPDATE channel_members SET role='member' WHERE community_id=$1 AND channel_id=$2")
        .bind(community.as_uuid())
        .bind(channel)
        .execute(db.pool())
        .await
        .unwrap();
    let mut write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    assert_eq!(
        write.apply_command(&first, None).await.unwrap(),
        CommandApplication::Restricted
    );
    write.rollback().await.unwrap();
    assert_eq!(evidence(&db, community, &first).await, Some(true));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn legacy_evidence_is_unknown_and_scoped_to_community() {
    let (db, a) = database(false).await;
    let owner = Keys::generate();
    let relay = Keys::generate();
    let channel = Uuid::new_v4();
    let create = command(&owner, channel, 9007, &[]);
    apply(&db, a, &relay, &create).await;
    let legacy = command(&owner, channel, 9002, &[("add-label", "legacy")]);
    db.insert_event(a, &legacy, Some(channel)).await.unwrap();
    let mut write = db
        .begin_channel_metadata_write(a, channel, relay.public_key())
        .await
        .unwrap();
    assert_eq!(
        write.apply_command(&legacy, None).await.unwrap(),
        CommandApplication::Unknown
    );
    assert!(write.labels().is_empty());
    write.rollback().await.unwrap();
    let b = CommunityId::from_uuid(Uuid::new_v4());
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(b.as_uuid())
        .bind(format!("labels-b-{}.example", b.as_uuid()))
        .execute(db.pool())
        .await
        .unwrap();
    // Identical UUID and signed event in B creates independent state, not an A receipt.
    assert!(apply(&db, b, &relay, &create).await.is_some());
    assert!(apply(&db, b, &relay, &legacy).await.is_some());
    assert_eq!(evidence(&db, a, &legacy).await, Some(false));
    assert_eq!(evidence(&db, b, &legacy).await, Some(true));
}
