//! Serialized App callback admission: idempotency, locking, and atomic persist.

use std::sync::Arc;

use buzz_core::kind::{KIND_GIT_REPO_ANNOUNCEMENT, KIND_STREAM_MESSAGE};
use buzz_core::CommunityId;
use buzz_db::app::{create_app, CreateAppParams};
use buzz_db::app_admission::{
    begin_app_admission, AppDeliveryFailure, AppDeliveryRecord, AppDeliveryStatus,
    AppRouteSnapshot, BeginAppAdmission,
};
use buzz_db::channel::{ChannelType, ChannelVisibility};
use buzz_db::{Db, DbError};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::sync::Barrier;
use uuid::Uuid;

const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";
const EVENT_TYPE: &str = "workflow.run.completed";

fn database_url() -> String {
    std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| TEST_DB_URL.to_owned())
}

async fn setup() -> (Db, PgPool) {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url())
        .await
        .expect("connect to test DB");
    let db = Db::from_pool(pool.clone());
    db.migrate().await.expect("apply migrations");
    (db, pool)
}

async fn make_community(pool: &PgPool) -> CommunityId {
    let id = Uuid::new_v4();
    let host = format!("app-admission-{}.example", id.simple());
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(id)
        .bind(host)
        .execute(pool)
        .await
        .expect("insert community");
    CommunityId::from_uuid(id)
}

async fn make_app(db: &Db, community: CommunityId) -> Uuid {
    let app_id = Uuid::new_v4();
    let mut tx = db.begin_transaction().await.expect("begin create app");
    create_app(
        &mut tx,
        community,
        CreateAppParams {
            id: app_id,
            name: "Archon",
            description: None,
            icon_url: None,
            secret_hash: &[0x11; 32],
            created_by: &[0xab; 32],
        },
    )
    .await
    .expect("create app");
    tx.commit().await.expect("commit app");
    app_id
}

async fn make_channel(db: &Db, community: CommunityId, creator: &[u8]) -> Uuid {
    db.create_channel(
        community,
        &format!("dest-{}", Uuid::new_v4().simple()),
        ChannelType::Stream,
        ChannelVisibility::Open,
        None,
        creator,
        None,
    )
    .await
    .expect("create channel")
    .id
}

fn route(community: CommunityId, channel_id: Uuid) -> AppRouteSnapshot {
    AppRouteSnapshot {
        community_id: *community.as_uuid(),
        repository_coordinate: Some(format!("30617:{}:harness-service", "ab".repeat(32))),
        project_coordinate: Some(format!("30621:{}:gigo-harness", "cd".repeat(32))),
        channel_id,
    }
}

fn stream_event(keys: &Keys, channel_id: Uuid, content: &str, mention: Option<&str>) -> Event {
    let mut tags = vec![Tag::parse(["h", &channel_id.to_string()]).expect("h tag")];
    if let Some(pubkey) = mention {
        tags.push(Tag::parse(["p", pubkey]).expect("p tag"));
    }
    EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), content.to_owned())
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign stream event")
}

fn repo_event(keys: &Keys, d: &str) -> Event {
    EventBuilder::new(Kind::Custom(KIND_GIT_REPO_ANNOUNCEMENT as u16), "")
        .tags(vec![
            Tag::parse(["d", d]).expect("d tag"),
            Tag::parse(["name", d]).expect("name tag"),
        ])
        .sign_with_keys(keys)
        .expect("sign repo event")
}

async fn delivery_count(
    pool: &PgPool,
    community: CommunityId,
    app_id: Uuid,
    key: &[u8; 32],
) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM app_callback_deliveries \
         WHERE community_id = $1 AND app_id = $2 AND idempotency_key_hash = $3",
    )
    .bind(community.as_uuid())
    .bind(app_id)
    .bind(key.as_slice())
    .fetch_one(pool)
    .await
    .expect("count deliveries")
}

async fn event_count(pool: &PgPool, community: CommunityId, event_id: &[u8]) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id = $1 AND id = $2")
        .bind(community.as_uuid())
        .bind(event_id)
        .fetch_one(pool)
        .await
        .expect("count events")
}

async fn mention_pubkeys(pool: &PgPool, community: CommunityId, event_id: &[u8]) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT pubkey_hex FROM event_mentions \
         WHERE community_id = $1 AND event_id = $2 ORDER BY pubkey_hex",
    )
    .bind(community.as_uuid())
    .bind(event_id)
    .fetch_all(pool)
    .await
    .expect("list mentions")
}

#[tokio::test]
async fn vacant_admission_allocates_delivery_uuid() {
    let (db, pool) = setup().await;
    let community = make_community(&pool).await;
    let app_id = make_app(&db, community).await;
    let key = Sha256::digest(b"vacant-key").into();
    let payload = Sha256::digest(b"vacant-payload").into();

    let begin = begin_app_admission(&pool, community, app_id, &key, &payload, EVENT_TYPE)
        .await
        .expect("begin vacant");
    let BeginAppAdmission::Vacant(guard) = begin else {
        panic!("first admission must be vacant");
    };
    assert!(
        !guard.delivery_id().is_nil(),
        "vacant guard allocates a UUID"
    );
    drop(guard);
}

#[tokio::test]
async fn same_payload_replay_returns_existing() {
    let (db, pool) = setup().await;
    let community = make_community(&pool).await;
    let app_id = make_app(&db, community).await;
    let key = Sha256::digest(b"replay-key").into();
    let payload = Sha256::digest(b"replay-payload").into();

    let begin = begin_app_admission(&pool, community, app_id, &key, &payload, EVENT_TYPE)
        .await
        .expect("begin first");
    let BeginAppAdmission::Vacant(guard) = begin else {
        panic!("first admission must be vacant");
    };
    let created = guard
        .reject(AppDeliveryFailure {
            code: "repository_missing",
        })
        .await
        .expect("reject first");

    let second = begin_app_admission(&pool, community, app_id, &key, &payload, EVENT_TYPE)
        .await
        .expect("begin replay");
    let BeginAppAdmission::Existing(existing) = second else {
        panic!("same payload must return existing");
    };
    assert_eq!(existing.id, created.id);
    assert_eq!(existing.status, AppDeliveryStatus::Rejected);
    assert_eq!(existing.completed_at, created.completed_at);
    assert_eq!(delivery_count(&pool, community, app_id, &key).await, 1);
}

#[tokio::test]
async fn different_payload_is_conflict() {
    let (db, pool) = setup().await;
    let community = make_community(&pool).await;
    let app_id = make_app(&db, community).await;
    let key = Sha256::digest(b"conflict-key").into();
    let original = Sha256::digest(b"payload-a").into();
    let changed = Sha256::digest(b"payload-b").into();

    let begin = begin_app_admission(&pool, community, app_id, &key, &original, EVENT_TYPE)
        .await
        .expect("begin first");
    let BeginAppAdmission::Vacant(guard) = begin else {
        panic!("first admission must be vacant");
    };
    let created = guard
        .reject(AppDeliveryFailure {
            code: "project_missing",
        })
        .await
        .expect("reject first");

    let second = begin_app_admission(&pool, community, app_id, &key, &changed, EVENT_TYPE)
        .await
        .expect("begin conflict");
    let BeginAppAdmission::PayloadConflict { existing } = second else {
        panic!("changed payload must conflict");
    };
    assert_eq!(existing.id, created.id);
    assert_eq!(existing.payload_hash, original);
    assert_eq!(delivery_count(&pool, community, app_id, &key).await, 1);
}

#[tokio::test]
async fn concurrent_identical_requests_create_one_row_and_one_event() {
    let (db, pool) = setup().await;
    let community = make_community(&pool).await;
    let app_id = make_app(&db, community).await;
    let relay = Keys::generate();
    let creator = relay.public_key().to_bytes();
    let channel_id = make_channel(&db, community, creator.as_slice()).await;
    let event = stream_event(&relay, channel_id, "shipped", None);
    let route = route(community, channel_id);
    let key: [u8; 32] = Sha256::digest(b"concurrent-key").into();
    let payload: [u8; 32] = Sha256::digest(b"concurrent-payload").into();

    let barrier = Arc::new(Barrier::new(2));
    let spawn_one = |pool: PgPool, event: Event, route: AppRouteSnapshot, barrier: Arc<Barrier>| {
        tokio::spawn(async move {
            barrier.wait().await;
            let begin =
                begin_app_admission(&pool, community, app_id, &key, &payload, EVENT_TYPE).await?;
            let record = match begin {
                BeginAppAdmission::Vacant(guard) => guard.deliver(&event, &route).await?.record,
                BeginAppAdmission::Existing(record) => record,
                BeginAppAdmission::PayloadConflict { .. } => {
                    panic!("identical payloads must not conflict")
                }
            };
            Ok::<AppDeliveryRecord, DbError>(record)
        })
    };

    let first = spawn_one(pool.clone(), event.clone(), route.clone(), barrier.clone());
    let second = spawn_one(pool.clone(), event.clone(), route, barrier);
    let first = first.await.expect("join first").expect("first admission");
    let second = second
        .await
        .expect("join second")
        .expect("second admission");

    assert_eq!(first.id, second.id);
    assert_eq!(first.status, AppDeliveryStatus::Delivered);
    assert_eq!(second.status, AppDeliveryStatus::Delivered);
    assert_eq!(delivery_count(&pool, community, app_id, &key).await, 1);
    assert_eq!(
        event_count(&pool, community, event.id.as_bytes().as_slice()).await,
        1
    );
}

#[tokio::test]
async fn deterministic_rejection_replay() {
    let (db, pool) = setup().await;
    let community = make_community(&pool).await;
    let app_id = make_app(&db, community).await;
    let key = Sha256::digest(b"reject-key").into();
    let payload = Sha256::digest(b"reject-payload").into();

    let begin = begin_app_admission(&pool, community, app_id, &key, &payload, EVENT_TYPE)
        .await
        .expect("begin first");
    let BeginAppAdmission::Vacant(guard) = begin else {
        panic!("first admission must be vacant");
    };
    let created = guard
        .reject(AppDeliveryFailure {
            code: "channel_missing",
        })
        .await
        .expect("reject");
    assert_eq!(created.status, AppDeliveryStatus::Rejected);
    assert_eq!(created.failure_code.as_deref(), Some("channel_missing"));
    assert!(created.event_id.is_none());
    assert!(created.route_snapshot.is_none());

    let replay = begin_app_admission(&pool, community, app_id, &key, &payload, EVENT_TYPE)
        .await
        .expect("replay");
    let BeginAppAdmission::Existing(existing) = replay else {
        panic!("rejection replay must return existing");
    };
    assert_eq!(existing.id, created.id);
    assert_eq!(existing.failure_code.as_deref(), Some("channel_missing"));
    assert_eq!(existing.status, AppDeliveryStatus::Rejected);
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id = $1 AND kind = $2")
            .bind(community.as_uuid())
            .bind(KIND_STREAM_MESSAGE as i32)
            .fetch_one(&pool)
            .await
            .expect("count kind 9");
    assert_eq!(events, 0);
}

#[tokio::test]
async fn dropped_guard_creates_neither_event_nor_delivery() {
    let (db, pool) = setup().await;
    let community = make_community(&pool).await;
    let app_id = make_app(&db, community).await;
    let key = Sha256::digest(b"drop-key").into();
    let payload = Sha256::digest(b"drop-payload").into();

    let begin = begin_app_admission(&pool, community, app_id, &key, &payload, EVENT_TYPE)
        .await
        .expect("begin");
    let BeginAppAdmission::Vacant(guard) = begin else {
        panic!("first admission must be vacant");
    };
    drop(guard);

    assert_eq!(delivery_count(&pool, community, app_id, &key).await, 0);
    let retry = begin_app_admission(&pool, community, app_id, &key, &payload, EVENT_TYPE)
        .await
        .expect("retry after drop");
    let BeginAppAdmission::Vacant(retry_guard) = retry else {
        panic!("dropped guard must leave the key vacant");
    };
    drop(retry_guard);
    assert_eq!(delivery_count(&pool, community, app_id, &key).await, 0);
}

#[tokio::test]
async fn cross_community_isolation() {
    let (db, pool) = setup().await;
    let community_a = make_community(&pool).await;
    let community_b = make_community(&pool).await;
    let app_a = make_app(&db, community_a).await;
    let app_b = make_app(&db, community_b).await;
    let key = Sha256::digest(b"shared-key").into();
    let payload = Sha256::digest(b"shared-payload").into();

    let first = begin_app_admission(&pool, community_a, app_a, &key, &payload, EVENT_TYPE)
        .await
        .expect("begin a");
    let BeginAppAdmission::Vacant(guard_a) = first else {
        panic!("community A must be vacant");
    };
    let record_a = guard_a
        .reject(AppDeliveryFailure {
            code: "repository_missing",
        })
        .await
        .expect("reject a");

    let second = begin_app_admission(&pool, community_b, app_b, &key, &payload, EVENT_TYPE)
        .await
        .expect("begin b");
    let BeginAppAdmission::Vacant(guard_b) = second else {
        panic!("same key in another community must be vacant");
    };
    let record_b = guard_b
        .reject(AppDeliveryFailure {
            code: "project_missing",
        })
        .await
        .expect("reject b");

    assert_ne!(record_a.id, record_b.id);
    assert_eq!(record_a.community_id, community_a);
    assert_eq!(record_b.community_id, community_b);
    assert_eq!(delivery_count(&pool, community_a, app_a, &key).await, 1);
    assert_eq!(delivery_count(&pool, community_b, app_b, &key).await, 1);
}

#[tokio::test]
async fn live_channel_lookup_heads_and_named_members() {
    let (db, pool) = setup().await;
    let community = make_community(&pool).await;
    let other = make_community(&pool).await;
    let app_id = make_app(&db, community).await;
    let owner = Keys::generate();
    let member = Keys::generate();
    let owner_bytes = owner.public_key().to_bytes();
    let member_bytes = member.public_key().to_bytes();
    db.ensure_user(community, owner_bytes.as_slice())
        .await
        .expect("ensure owner");
    db.ensure_user(community, member_bytes.as_slice())
        .await
        .expect("ensure member");
    db.update_user_profile(
        community,
        member_bytes.as_slice(),
        Some("Archon"),
        None,
        None,
        None,
    )
    .await
    .expect("name member");

    let live_id = make_channel(&db, community, owner_bytes.as_slice()).await;
    db.add_member(
        community,
        live_id,
        member_bytes.as_slice(),
        buzz_db::channel::MemberRole::Member,
        Some(owner_bytes.as_slice()),
    )
    .await
    .expect("add named member");

    let archived_id = make_channel(&db, community, owner_bytes.as_slice()).await;
    sqlx::query("UPDATE channels SET archived_at = NOW() WHERE community_id = $1 AND id = $2")
        .bind(community.as_uuid())
        .bind(archived_id)
        .execute(&pool)
        .await
        .expect("archive channel");

    let deleted_id = make_channel(&db, community, owner_bytes.as_slice()).await;
    sqlx::query("UPDATE channels SET deleted_at = NOW() WHERE community_id = $1 AND id = $2")
        .bind(community.as_uuid())
        .bind(deleted_id)
        .execute(&pool)
        .await
        .expect("delete channel");

    let foreign_id = make_channel(&db, other, owner_bytes.as_slice()).await;
    let repo = repo_event(&owner, "harness-service");
    db.insert_event(community, &repo, None)
        .await
        .expect("insert repo head");
    let foreign_repo = repo_event(&owner, "other-community");
    db.insert_event(other, &foreign_repo, None)
        .await
        .expect("insert foreign head");

    let key = Sha256::digest(b"lookup-key").into();
    let payload = Sha256::digest(b"lookup-payload").into();
    let begin = begin_app_admission(&pool, community, app_id, &key, &payload, EVENT_TYPE)
        .await
        .expect("begin");
    let BeginAppAdmission::Vacant(mut guard) = begin else {
        panic!("lookup admission must be vacant");
    };

    let live = guard
        .load_live_destination_channel(live_id)
        .await
        .expect("load live");
    assert_eq!(live.expect("live channel").id, live_id);
    assert!(guard
        .load_live_destination_channel(archived_id)
        .await
        .expect("archived")
        .is_none());
    assert!(guard
        .load_live_destination_channel(deleted_id)
        .await
        .expect("deleted")
        .is_none());
    assert!(guard
        .load_live_destination_channel(foreign_id)
        .await
        .expect("foreign")
        .is_none());

    let members = guard
        .list_named_destination_members(live_id)
        .await
        .expect("named members");
    assert!(
        members
            .iter()
            .any(|m| m.pubkey == member_bytes && m.display_name.as_deref() == Some("Archon")),
        "current named member must be visible in the admission transaction"
    );
    let foreign_members = guard
        .list_named_destination_members(foreign_id)
        .await
        .expect("foreign members");
    assert!(foreign_members.is_empty());

    let heads = guard
        .list_latest_parameterized_heads(KIND_GIT_REPO_ANNOUNCEMENT as i32)
        .await
        .expect("heads");
    assert!(
        heads.iter().any(|h| h.event.id == repo.id),
        "guard must see live heads in its community"
    );
    assert!(
        heads.iter().all(|h| h.event.id != foreign_repo.id),
        "guard must not see another community's heads"
    );
    drop(guard);
}

#[tokio::test]
async fn deliver_commits_event_and_delivery_atomically() {
    let (db, pool) = setup().await;
    let community = make_community(&pool).await;
    let app_id = make_app(&db, community).await;
    let relay = Keys::generate();
    let creator = relay.public_key().to_bytes();
    let channel_id = make_channel(&db, community, creator.as_slice()).await;
    let event = stream_event(&relay, channel_id, "delivered", None);
    let route = route(community, channel_id);
    let key = Sha256::digest(b"atomic-key").into();
    let payload = Sha256::digest(b"atomic-payload").into();

    let begin = begin_app_admission(&pool, community, app_id, &key, &payload, EVENT_TYPE)
        .await
        .expect("begin");
    let BeginAppAdmission::Vacant(guard) = begin else {
        panic!("first admission must be vacant");
    };
    let delivery_id = guard.delivery_id();
    let commit = guard.deliver(&event, &route).await.expect("deliver");

    assert_eq!(commit.record.id, delivery_id);
    assert_eq!(commit.record.status, AppDeliveryStatus::Delivered);
    assert_eq!(
        commit.record.event_id.as_ref().map(|id| id.as_slice()),
        Some(event.id.as_bytes().as_slice())
    );
    assert_eq!(commit.record.route_snapshot.as_ref(), Some(&route));
    assert!(commit.record.failure_code.is_none());
    assert_eq!(commit.stored_event.event.id, event.id);
    assert_eq!(delivery_count(&pool, community, app_id, &key).await, 1);
    assert_eq!(
        event_count(&pool, community, event.id.as_bytes().as_slice()).await,
        1
    );
}

#[tokio::test]
async fn deliver_inserts_event_mentions() {
    let (db, pool) = setup().await;
    let community = make_community(&pool).await;
    let app_id = make_app(&db, community).await;
    let relay = Keys::generate();
    let mentioned = Keys::generate();
    let creator = relay.public_key().to_bytes();
    let channel_id = make_channel(&db, community, creator.as_slice()).await;
    let mention_hex = mentioned.public_key().to_hex();
    let event = stream_event(&relay, channel_id, "@Archon shipped", Some(&mention_hex));
    let route = route(community, channel_id);
    let key = Sha256::digest(b"mention-key").into();
    let payload = Sha256::digest(b"mention-payload").into();

    let begin = begin_app_admission(&pool, community, app_id, &key, &payload, EVENT_TYPE)
        .await
        .expect("begin");
    let BeginAppAdmission::Vacant(guard) = begin else {
        panic!("first admission must be vacant");
    };
    let commit = guard
        .deliver(&event, &route)
        .await
        .expect("deliver with mention");
    let stored = mention_pubkeys(
        &pool,
        community,
        commit.stored_event.event.id.as_bytes().as_slice(),
    )
    .await;
    assert_eq!(stored, vec![mention_hex.to_ascii_lowercase()]);
}

#[tokio::test]
async fn failed_delivery_insert_rolls_back_event() {
    let (db, pool) = setup().await;
    let community = make_community(&pool).await;
    let app_id = make_app(&db, community).await;
    let other_app = make_app(&db, community).await;
    let relay = Keys::generate();
    let creator = relay.public_key().to_bytes();
    let channel_id = make_channel(&db, community, creator.as_slice()).await;
    let event = stream_event(&relay, channel_id, "should roll back", None);
    let route = route(community, channel_id);
    let key = Sha256::digest(b"rollback-key").into();
    let payload = Sha256::digest(b"rollback-payload").into();

    let begin = begin_app_admission(&pool, community, app_id, &key, &payload, EVENT_TYPE)
        .await
        .expect("begin");
    let BeginAppAdmission::Vacant(guard) = begin else {
        panic!("first admission must be vacant");
    };
    let colliding_id = guard.delivery_id();
    sqlx::query(
        "INSERT INTO app_callback_deliveries \
         (community_id, id, app_id, idempotency_key_hash, payload_hash, event_type, \
          route_snapshot, status, event_id, failure_code, created_at, completed_at) \
         VALUES ($1, $2, $3, $4, $5, 'workflow.run.completed', $6, 'delivered', $7, NULL, NOW(), NOW())",
    )
    .bind(community.as_uuid())
    .bind(colliding_id)
    .bind(other_app)
    .bind(Sha256::digest(b"other-key").as_slice())
    .bind(Sha256::digest(b"other-payload").as_slice())
    .bind(serde_json::json!({"channel_id": channel_id}))
    .bind([0x55u8; 32].as_slice())
    .execute(&pool)
    .await
    .expect("insert colliding delivery id");

    let err = guard
        .deliver(&event, &route)
        .await
        .expect_err("duplicate delivery id must fail the final insert");
    assert!(
        matches!(err, DbError::Sqlx(_)) || err.to_string().contains("duplicate"),
        "unexpected delivery-insert error: {err:?}"
    );
    assert_eq!(delivery_count(&pool, community, app_id, &key).await, 0);
    assert_eq!(
        event_count(&pool, community, event.id.as_bytes().as_slice()).await,
        0
    );
}
