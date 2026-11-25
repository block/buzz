//! PostgreSQL contract for the search outbox: what admission queues and what
//! a claim reads back. The community-deletion fence and purge are covered in
//! `deletion::postgres_tests::search_outbox_is_fenced_and_purged`.
//!
//! nextest runs each test in its own process, so [`search_outbox::enable`]
//! does not leak between tests.
use crate::{event, search_outbox, Db, DbConfig};
use buzz_core::CommunityId;
use chrono::DateTime;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use sqlx::PgPool;
use uuid::Uuid;

async fn connect() -> Db {
    let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .expect("Postgres wrapper must provide an isolated database URL");
    Db::new(&DbConfig {
        database_url,
        ..DbConfig::default()
    })
    .await
    .expect("connect to isolated database")
}

async fn create_community(pool: &PgPool) -> CommunityId {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("search-outbox-{}.example", id.simple()))
        .execute(pool)
        .await
        .expect("create community");
    CommunityId::from_uuid(id)
}

async fn create_dm(pool: &PgPool, community: CommunityId, author: &Keys) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO channels (id, community_id, name, channel_type, created_by) \
         VALUES ($1, $2, $3, 'dm', $4)",
    )
    .bind(id)
    .bind(community.as_uuid())
    .bind(format!("dm-{}", id.simple()))
    .bind(author.public_key().to_bytes().as_slice())
    .execute(pool)
    .await
    .expect("create channel");
    id
}

fn signed(keys: &Keys, kind: u16, content: &str, tags: Vec<Tag>) -> Event {
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign event")
}

async fn queued(pool: &PgPool, community: CommunityId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM search_outbox WHERE community_id = $1")
        .bind(community.as_uuid())
        .fetch_one(pool)
        .await
        .expect("count outbox rows")
}

/// An admitted message queues one row; a claim reads back its current state,
/// including fields from other tables, and completing it empties the queue.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn claim_reads_the_current_state_of_queued_events() {
    let db = connect().await;
    let pool = db.pool();
    search_outbox::enable(pool).await.expect("enable outbox");
    let community = create_community(pool).await;
    let agent = Keys::generate();
    let owner = Keys::generate();
    let mentioned = Keys::generate();
    let channel = create_dm(pool, community, &agent).await;
    sqlx::query("INSERT INTO users (community_id, pubkey, agent_owner_pubkey) VALUES ($1, $2, $3)")
        .bind(community.as_uuid())
        .bind(agent.public_key().to_bytes().as_slice())
        .bind(owner.public_key().to_bytes().as_slice())
        .execute(pool)
        .await
        .expect("register agent");

    let root = signed(&agent, 9, "root message", vec![]);
    db.insert_event(community, &root, Some(channel))
        .await
        .expect("insert root");
    let reply = signed(
        &agent,
        9,
        "reply mentioning someone",
        vec![Tag::public_key(mentioned.public_key())],
    );
    let root_created = DateTime::from_timestamp(root.created_at.as_secs() as i64, 0).unwrap();
    let reply_created = DateTime::from_timestamp(reply.created_at.as_secs() as i64, 0).unwrap();
    db.insert_event_with_thread_metadata(
        community,
        &reply,
        Some(channel),
        Some(event::ThreadMetadataParams {
            event_id: reply.id.as_bytes(),
            event_created_at: reply_created,
            channel_id: channel,
            parent_event_id: Some(root.id.as_bytes()),
            parent_event_created_at: Some(root_created),
            root_event_id: Some(root.id.as_bytes()),
            root_event_created_at: Some(root_created),
            depth: 1,
            broadcast: false,
        }),
    )
    .await
    .expect("insert reply");
    assert_eq!(queued(pool, community).await, 2);

    let batch = search_outbox::claim_batch(pool, 10, &[])
        .await
        .expect("claim")
        .expect("a batch");
    assert_eq!(batch.community(), community);
    assert!(batch.removed.is_empty());
    let doc = batch
        .documents
        .iter()
        .find(|doc| doc.event_id == *reply.id.as_bytes())
        .expect("reply document");
    assert_eq!(doc.kind, 9);
    assert_eq!(doc.pubkey, agent.public_key().to_bytes());
    assert_eq!(doc.channel_id, Some(channel));
    assert_eq!(doc.created_at, reply_created);
    assert_eq!(doc.content, "reply mentioning someone");
    assert_eq!(doc.channel_type.as_deref(), Some("dm"));
    assert!(doc.author_is_agent);
    assert_eq!(doc.thread_root, Some(*root.id.as_bytes()));
    assert_eq!(doc.mentions, vec![mentioned.public_key().to_hex()]);
    batch.complete().await.expect("complete");
    assert_eq!(queued(pool, community).await, 0);
    assert!(search_outbox::claim_batch(pool, 10, &[])
        .await
        .expect("claim")
        .is_none());
}

/// A claimed event that was deleted after it was queued comes back as
/// removed, not as a document.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn claim_reports_deleted_events_as_removed() {
    let db = connect().await;
    let pool = db.pool();
    search_outbox::enable(pool).await.expect("enable outbox");
    let community = create_community(pool).await;
    let message = signed(&Keys::generate(), 9, "soon deleted", vec![]);
    db.insert_event(community, &message, None)
        .await
        .expect("insert");
    sqlx::query("UPDATE events SET deleted_at = now() WHERE community_id = $1 AND id = $2")
        .bind(community.as_uuid())
        .bind(message.id.as_bytes().as_slice())
        .execute(pool)
        .await
        .expect("delete");

    let batch = search_outbox::claim_batch(pool, 10, &[])
        .await
        .expect("claim")
        .expect("a batch");
    assert!(batch.documents.is_empty());
    assert_eq!(batch.removed, vec![*message.id.as_bytes()]);
    batch.complete().await.expect("complete");
}

/// Only searchable kinds queue, a rolled-back insert queues nothing, and a
/// batch dropped without completing leaves its rows queued.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn only_committed_searchable_events_queue() {
    let db = connect().await;
    let pool = db.pool();
    search_outbox::enable(pool).await.expect("enable outbox");
    let community = create_community(pool).await;
    let keys = Keys::generate();
    db.insert_event(community, &signed(&keys, 7, "+", vec![]), None)
        .await
        .expect("insert reaction");
    assert_eq!(queued(pool, community).await, 0, "kind 7 is not searchable");

    let mut tx = db
        .begin_event_write_transaction(community)
        .await
        .expect("begin");
    event::insert_event_in_transaction(&mut tx, &signed(&keys, 9, "rolled back", vec![]), None)
        .await
        .expect("insert");
    tx.rollback().await.expect("rollback");
    assert_eq!(
        queued(pool, community).await,
        0,
        "a rollback queues nothing"
    );

    db.insert_event(community, &signed(&keys, 9, "kept", vec![]), None)
        .await
        .expect("insert message");
    let batch = search_outbox::claim_batch(pool, 10, &[])
        .await
        .expect("claim")
        .expect("a batch");
    drop(batch);
    assert_eq!(
        queued(pool, community).await,
        1,
        "an abandoned batch keeps its row"
    );
}

/// Without [`search_outbox::enable`], admission writes no outbox rows.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn disabled_outbox_queues_nothing() {
    let db = connect().await;
    let pool = db.pool();
    let community = create_community(pool).await;
    db.insert_event(
        community,
        &signed(&Keys::generate(), 9, "hello", vec![]),
        None,
    )
    .await
    .expect("insert");
    assert_eq!(queued(pool, community).await, 0);
}

/// Account deletion queues index changes once an indexing relay has started,
/// even while the outbox and the backfill are both empty.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn the_index_counts_as_maintained_once_a_relay_enabled_it() {
    let db = connect().await;
    let pool = db.pool();
    let mut conn = pool.acquire().await.expect("connect");
    assert!(!search_outbox::index_maintained(&mut conn)
        .await
        .expect("read before enabling"));
    // Mutation: drop the sequence check from index_maintained → RED.
    search_outbox::enable(pool).await.expect("enable outbox");
    assert!(search_outbox::index_maintained(&mut conn)
        .await
        .expect("read after enabling"));
}

/// Enabling fails, and leaves admission unchanged, while the version
/// sequence or the table is missing.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn enable_requires_the_table() {
    let db = connect().await;
    let pool = db.pool();
    sqlx::query("DROP SEQUENCE search_index_version")
        .execute(pool)
        .await
        .expect("drop sequence");
    search_outbox::enable(pool)
        .await
        .expect_err("enable refuses a database without the version sequence");
    sqlx::query("DROP TABLE search_outbox")
        .execute(pool)
        .await
        .expect("drop table");
    search_outbox::enable(pool)
        .await
        .expect_err("enable refuses a database without the table");
    let community = create_community(pool).await;
    db.insert_event(
        community,
        &signed(&Keys::generate(), 9, "still stored", vec![]),
        None,
    )
    .await
    .expect("admission works with the outbox off");
}

/// A claim passes over communities it is told to skip and communities whose
/// rows another worker holds.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn claim_passes_over_skipped_and_held_communities() {
    let db = connect().await;
    let pool = db.pool();
    search_outbox::enable(pool).await.expect("enable outbox");
    let first = create_community(pool).await;
    let second = create_community(pool).await;
    let keys = Keys::generate();
    for community in [first, second] {
        db.insert_event(community, &signed(&keys, 9, "queued", vec![]), None)
            .await
            .expect("insert");
    }

    let held = search_outbox::claim_batch(pool, 10, &[])
        .await
        .expect("claim")
        .expect("a batch");
    assert_eq!(held.community(), first, "the oldest row's community");
    let other = search_outbox::claim_batch(pool, 10, &[])
        .await
        .expect("claim")
        .expect("another worker's batch");
    assert_eq!(other.community(), second);
    drop(other);
    drop(held);

    let skipping = search_outbox::claim_batch(pool, 10, &[first])
        .await
        .expect("claim")
        .expect("a batch");
    assert_eq!(skipping.community(), second);
    drop(skipping);
    assert!(search_outbox::claim_batch(pool, 10, &[first, second])
        .await
        .expect("claim")
        .is_none());
}

/// Two workers can claim one community at once, but never rows of the same
/// event. A message changed or deleted while a batch holds its earlier row
/// gets a new row that no concurrent batch can claim; it is claimed, with the
/// newer state, only once the holding batch completes. So a stale state
/// never overwrites a newer one, and a delete is never followed by a stale
/// write.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_claims_of_one_community_never_share_an_event() {
    let db = connect().await;
    let pool = db.pool();
    search_outbox::enable(pool).await.expect("enable outbox");
    let community = create_community(pool).await;
    let keys = Keys::generate();
    for n in 0..200 {
        db.insert_event(
            community,
            &signed(&keys, 9, &format!("message {n}"), vec![]),
            None,
        )
        .await
        .expect("insert");
    }

    let first = search_outbox::claim_batch_share(pool, 1000, &[], 2)
        .await
        .expect("claim")
        .expect("a batch");
    let changed = first.documents[0].event_id;
    let deleted = first.documents[1].event_id;
    // What an edit and a deletion queue while the first batch is in flight.
    for event in [changed, deleted] {
        sqlx::query(
            "INSERT INTO search_outbox (community_id, event_id, event_created_at) \
             SELECT community_id, id, created_at FROM events \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(event.as_slice())
        .execute(pool)
        .await
        .expect("queue a change");
    }
    sqlx::query("UPDATE events SET deleted_at = now() WHERE community_id = $1 AND id = $2")
        .bind(community.as_uuid())
        .bind(deleted.as_slice())
        .execute(pool)
        .await
        .expect("delete");

    let second = search_outbox::claim_batch_share(pool, 1000, &[], 2)
        .await
        .expect("claim")
        .expect("a concurrent batch of the same community");
    assert_eq!(second.community(), community);
    let events =
        |batch: &search_outbox::SearchOutboxBatch| -> std::collections::BTreeSet<[u8; 32]> {
            batch
                .documents
                .iter()
                .map(|document| document.event_id)
                .chain(batch.removed.iter().copied())
                .collect()
        };
    let (held, concurrent) = (events(&first), events(&second));
    assert!(!concurrent.is_empty());
    assert!(
        held.is_disjoint(&concurrent),
        "two batches in flight hold the same event"
    );
    drop(second);
    first.complete().await.expect("complete");

    // Only now are the changes claimed, with their current state.
    let after = search_outbox::claim_batch(pool, 1000, &[])
        .await
        .expect("claim")
        .expect("a batch");
    assert!(after
        .documents
        .iter()
        .any(|document| document.event_id == changed));
    assert!(after.removed.contains(&deleted));
    assert!(after
        .documents
        .iter()
        .all(|document| document.event_id != deleted));
    after.complete().await.expect("complete");
    assert_eq!(queued(pool, community).await, 0);
}

/// A worker whose share of the queue head is held by another batch still
/// claims rows from the other partitions, instead of coming back empty and
/// leaving the drain while rows remain.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_claim_looks_past_a_held_queue_head() {
    let db = connect().await;
    let pool = db.pool();
    search_outbox::enable(pool).await.expect("enable outbox");
    let community = create_community(pool).await;
    let keys = Keys::generate();
    let partition =
        |event: &Event| i32::from(event.id.as_bytes()[0]) % search_outbox::CLAIM_PARTITIONS;
    // With batches of 1 among 2 workers, the head is the oldest 4 rows.
    let mut head = std::collections::BTreeSet::new();
    for n in 0..4 {
        let event = signed(&keys, 9, &format!("head {n}"), vec![]);
        head.insert(partition(&event));
        db.insert_event(community, &event, None)
            .await
            .expect("insert");
    }
    let behind = (0..)
        .map(|n| signed(&keys, 9, &format!("behind {n}"), vec![]))
        .find(|event| !head.contains(&partition(event)))
        .expect("an event outside the head's partitions");
    db.insert_event(community, &behind, None)
        .await
        .expect("insert");

    let first = search_outbox::claim_batch_share(pool, 1, &[], 2)
        .await
        .expect("claim")
        .expect("a batch");
    let second = search_outbox::claim_batch_share(pool, 1, &[], 2)
        .await
        .expect("claim")
        .expect("a batch from past the held head");
    assert_eq!(second.documents.len(), 1);
    assert_eq!(second.documents[0].event_id, *behind.id.as_bytes());
    drop(second);
    drop(first);
}
