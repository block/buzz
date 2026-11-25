//! Message search through the OpenSearch backend.
//!
//! The shared [`search_suite`] cases run against OpenSearch, then sync tests
//! write through the database layer's normal paths and wait for the indexer.
//! Needs Postgres (run through `scripts/postgres-test-run.sh -p buzz-search
//! --tests`) and an OpenSearch cluster at `BUZZ_TEST_OPENSEARCH_URL`
//! (default `http://localhost:9200`; `docker compose up -d opensearch`).

mod search_suite;

use search_suite::{search_suite_tests, Backend};

// Not run here, with the reason:
// - kind0_* and artifact_*: kinds 0 and 45010 are not searchable kinds;
// - prefix_*: prefix mode is reworked with prefix analysis (issue 06).
search_suite_tests! {
    Backend::OpenSearch;
    #[ignore = "requires Postgres and OpenSearch"]
    fn search_finds_event_in_same_community,
    fn search_does_not_return_other_community_events,
    fn authors_filter_restricts_results,
    fn channel_scope_restricts_results,
    fn channel_scope_of_thousands_of_channels,
    fn empty_query_returns_empty_result_no_roundtrip,
    fn since_until_filters,
    fn pagination_works,
    fn newest_sort_pages_by_time_then_id_within_scope,
    fn channel_less_only_excludes_per_channel_events,
    fn nul_bytes_in_query_are_sanitized,
    fn enormous_page_number_is_clamped,
    fn very_long_query_is_bounded_before_pg_parse,
    fn excluded_kinds_are_storage_level_unsearchable,
    fn author_only_kinds_are_storage_level_unsearchable,
    fn p_gated_persistent_kinds_have_storage_null_tsvector,
    fn deleted_events_are_excluded,
}

use std::time::Duration;

use buzz_core::CommunityId;
use buzz_db::channel::{ChannelType, ChannelVisibility};
use buzz_db::event::ThreadMetadataParams;
use buzz_db::search_outbox::SearchDocument;
use buzz_db::user_deletion::content_batches::ContentBatchOptions;
use buzz_db::user_deletion::ExecutionMode;
use buzz_db::{search_outbox, Db, DbConfig, DeletionTarget};
use buzz_search::indexer::{Indexer, IndexerError};
use buzz_search::opensearch::{OpenSearchConfig, SearchIndex};
use buzz_search::{ChannelScope, SearchMode, SearchQuery, SearchService, SearchSort};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use opensearch::http::transport::Transport;
use opensearch::indices::{IndicesCreateParts, IndicesExistsParts};
use opensearch::{GetParts, OpenSearch};
use uuid::Uuid;

const TEST_OPENSEARCH_URL: &str = "http://localhost:9200";

fn opensearch_url() -> String {
    std::env::var("BUZZ_TEST_OPENSEARCH_URL").unwrap_or_else(|_| TEST_OPENSEARCH_URL.into())
}

/// A database layer with the search outbox on, as the relay runs it when
/// OpenSearch is configured.
async fn sync_db() -> Db {
    let db = unsynced_db().await;
    search_outbox::enable(db.pool())
        .await
        .expect("enable outbox");
    db
}

/// A database layer as a relay without OpenSearch runs it: admission queues
/// nothing.
async fn unsynced_db() -> Db {
    let (pool, _) = search_suite::setup().await;
    pool.close().await;
    Db::new(&DbConfig {
        database_url: std::env::var("BUZZ_TEST_DATABASE_URL")
            .unwrap_or_else(|_| search_suite::TEST_DB_URL.into()),
        ..DbConfig::default()
    })
    .await
    .expect("connect database layer")
}

/// A new index of the test's own, and its index-name prefix.
async fn sync_index() -> (SearchIndex, String) {
    let prefix = format!("test-{}-", Uuid::new_v4().simple());
    let index = SearchIndex::new(&OpenSearchConfig {
        url: opensearch_url(),
        basic_auth: None,
        aws_sigv4: None,
        index_prefix: prefix.clone(),
    })
    .expect("OpenSearch client");
    index.ensure().await.expect("create index");
    (index, prefix)
}

/// A message signed `offset` seconds from now.
fn message_at(keys: &Keys, content: &str, offset: i64) -> Event {
    EventBuilder::new(Kind::Custom(9), content)
        .custom_created_at(Timestamp::from(
            (Timestamp::now().as_secs() as i64 + offset) as u64,
        ))
        .sign_with_keys(keys)
        .expect("sign")
}

fn message(keys: &Keys, content: &str) -> Event {
    EventBuilder::new(Kind::Custom(9), content)
        .sign_with_keys(keys)
        .expect("sign")
}

/// Wait until the indexer has drained the outbox, then make its writes
/// visible to search.
async fn wait_until_indexed(db: &Db, index: &SearchIndex) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while search_outbox::depth(db.pool()).await.expect("outbox depth") > 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the indexer drains the outbox");
    index.refresh().await.expect("refresh");
}

async fn find(service: &SearchService, community: CommunityId, q: &str) -> Vec<[u8; 32]> {
    service
        .search(&SearchQuery {
            community,
            q: q.into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: SearchMode::FullText,
            sort: SearchSort::Newest,
        })
        .await
        .expect("search")
        .hits
        .into_iter()
        .map(|hit| hit.event_id)
        .collect()
}

/// An admitted message becomes searchable once the lease-holding indexer
/// catches up; a rolled-back one never does, and its text is not stored in
/// the document source.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn admitted_messages_become_searchable() {
    let db = sync_db().await;
    let (index, prefix) = sync_index().await;
    let community = search_suite::mk_community(db.pool(), "sync.example").await;
    let keys = Keys::generate();
    let indexer = tokio::spawn(
        Indexer::new(db.clone(), index.clone())
            .with_new_index_settle(Duration::ZERO)
            .run(),
    );

    let kept = message(&keys, "syncword kept");
    db.insert_event(community, &kept, None)
        .await
        .expect("insert");
    let mut tx = db
        .begin_event_write_transaction(community)
        .await
        .expect("begin");
    buzz_db::event::insert_event_in_transaction(
        &mut tx,
        &message(&keys, "syncword rolled back"),
        None,
    )
    .await
    .expect("insert");
    tx.rollback().await.expect("rollback");

    wait_until_indexed(&db, &index).await;
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    assert_eq!(
        find(&service, community, "syncword").await,
        vec![*kept.id.as_bytes()]
    );

    let client = OpenSearch::new(Transport::single_node(&opensearch_url()).expect("transport"));
    let document: serde_json::Value = client
        .get(GetParts::IndexId(
            &format!("{prefix}messages"),
            &format!("{}:{}", community.as_uuid(), kept.id.to_hex()),
        ))
        .send()
        .await
        .expect("get document")
        .json()
        .await
        .expect("document body");
    assert_eq!(document["found"], true);
    assert_eq!(document["_source"]["kind"], 9);
    assert!(
        document["_source"].get("content").is_none(),
        "message text must not be stored: {document}"
    );

    indexer.abort();
    index.delete().await.expect("delete index");
}

/// Stopping the instance holding the indexer lease lets another take over,
/// and rows queued meanwhile are indexed.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn the_indexer_lease_hands_over() {
    let db = sync_db().await;
    let (index, _) = sync_index().await;
    let community = search_suite::mk_community(db.pool(), "lease.example").await;
    let keys = Keys::generate();
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());

    // Both are signed now, a second apart, so the newest-first order the
    // assertion expects does not depend on how long the handover takes.
    let before = message_at(&keys, "leaseword before", -1);
    let after = message_at(&keys, "leaseword after", 0);

    let first = tokio::spawn(
        Indexer::new(db.clone(), index.clone())
            .with_new_index_settle(Duration::ZERO)
            .run(),
    );
    db.insert_event(community, &before, None)
        .await
        .expect("insert");
    wait_until_indexed(&db, &index).await;

    // The first instance holds the lease, so the second only waits.
    let second = tokio::spawn(
        Indexer::new(db.clone(), index.clone())
            .with_new_index_settle(Duration::ZERO)
            .run(),
    );
    first.abort();
    let _ = first.await;
    db.insert_event(community, &after, None)
        .await
        .expect("insert");
    wait_until_indexed(&db, &index).await;
    assert_eq!(
        find(&service, community, "leaseword").await,
        vec![*after.id.as_bytes(), *before.id.as_bytes()]
    );

    second.abort();
    index.delete().await.expect("delete index");
}

/// Admission never calls OpenSearch: with it unreachable, messages are
/// stored and their rows stay queued until indexing succeeds.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn admission_queues_while_opensearch_is_unreachable() {
    let db = sync_db().await;
    let community = search_suite::mk_community(db.pool(), "down.example").await;
    let unreachable = SearchIndex::new(&OpenSearchConfig {
        url: "http://127.0.0.1:9".into(),
        basic_auth: None,
        aws_sigv4: None,
        index_prefix: "unreachable-".into(),
    })
    .expect("client");
    db.insert_event(community, &message(&Keys::generate(), "downword"), None)
        .await
        .expect("a message is stored while OpenSearch is down");

    Indexer::new(db.clone(), unreachable)
        .drain()
        .await
        .expect_err("indexing fails while OpenSearch is down");
    assert_eq!(search_outbox::depth(db.pool()).await.expect("depth"), 1);

    let (index, _) = sync_index().await;
    Indexer::new(db.clone(), index.clone())
        .drain()
        .await
        .expect("indexing succeeds once OpenSearch is back");
    index.refresh().await.expect("refresh");
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    assert_eq!(find(&service, community, "downword").await.len(), 1);
    index.delete().await.expect("delete index");
}

fn raw_client() -> OpenSearch {
    OpenSearch::new(Transport::single_node(&opensearch_url()).expect("transport"))
}

/// A bulk item the cluster rejects keeps only its own row queued: the other
/// items of the same request are indexed and their rows removed, other
/// communities are indexed in the same pass, and the pass does not fail, so
/// nothing backs off.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn a_failed_bulk_item_keeps_only_its_own_row() {
    let db = sync_db().await;
    // An index whose `content` is a keyword: a term longer than 32766 bytes
    // is rejected per document with HTTP 400.
    let prefix = format!("test-{}-", Uuid::new_v4().simple());
    let response = raw_client()
        .indices()
        .create(IndicesCreateParts::Index(&format!("{prefix}messages-v1")))
        .body(serde_json::json!({
            "mappings": { "properties": { "content": { "type": "keyword" } } },
            "aliases": { format!("{prefix}messages"): {} }
        }))
        .send()
        .await
        .expect("create index");
    assert!(response.status_code().is_success());
    let index = SearchIndex::new(&OpenSearchConfig {
        url: opensearch_url(),
        basic_auth: None,
        aws_sigv4: None,
        index_prefix: prefix.clone(),
    })
    .expect("client");

    let poisoned = search_suite::mk_community(db.pool(), "poisoned.example").await;
    let healthy = search_suite::mk_community(db.pool(), "healthy.example").await;
    let keys = Keys::generate();
    let oversized = message(&keys, &"x".repeat(40_000));
    let neighbour = message(&keys, "fine neighbour");
    let elsewhere = message(&keys, "fine elsewhere");
    for (community, event) in [
        (poisoned, &oversized),
        (poisoned, &neighbour),
        (healthy, &elsewhere),
    ] {
        db.insert_event(community, event, None)
            .await
            .expect("insert");
    }

    let report = Indexer::new(db.clone(), index.clone())
        .drain()
        .await
        .expect("a rejected item does not fail the pass");
    // The refused item is tried again with its community's next batch,
    // which holds nothing else, so that community is skipped.
    assert_eq!((report.written, report.failed), (2, 2));
    assert_eq!(queued_events(&db).await, vec![*oversized.id.as_bytes()]);
    for (community, event, found) in [
        (poisoned, &oversized, false),
        (poisoned, &neighbour, true),
        (healthy, &elsewhere, true),
    ] {
        assert_eq!(
            indexed(&prefix, community, event).await,
            found,
            "{}",
            event.content
        );
    }
    index.delete().await.expect("delete index");
}

/// Writes go through the alias and never create an index themselves, so a
/// deleted index cannot come back with a dynamic mapping; `ensure` restores
/// it with its alias.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn writes_never_create_an_index() {
    let db = sync_db().await;
    let (index, prefix) = sync_index().await;
    index.delete().await.expect("delete index");
    let community = search_suite::mk_community(db.pool(), "gone.example").await;
    db.insert_event(community, &message(&Keys::generate(), "orphan"), None)
        .await
        .expect("insert");

    let indexer = Indexer::new(db.clone(), index.clone());
    indexer
        .drain()
        .await
        .expect_err("a write without the index fails");
    let exists = raw_client()
        .indices()
        .exists(IndicesExistsParts::Index(&[&format!(
            "{prefix}messages-v1"
        )]))
        .send()
        .await
        .expect("exists");
    assert_eq!(exists.status_code().as_u16(), 404, "no index was created");
    assert_eq!(search_outbox::depth(db.pool()).await.expect("depth"), 1);

    index.ensure().await.expect("recreate index");
    indexer.drain().await.expect("indexing resumes");
    index.refresh().await.expect("refresh");
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    assert_eq!(find(&service, community, "orphan").await.len(), 1);
    index.delete().await.expect("delete index");
}

fn signed_at(event: &Event) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).expect("timestamp")
}

/// A channel of `community` created by `owner`.
async fn channel(db: &Db, community: CommunityId, owner: &Keys) -> Uuid {
    db.ensure_user(community, owner.public_key().as_bytes())
        .await
        .expect("ensure user");
    db.create_channel(
        community,
        &format!("sync-{}", Uuid::new_v4().simple()),
        ChannelType::Stream,
        ChannelVisibility::Open,
        None,
        owner.public_key().as_bytes(),
        None,
    )
    .await
    .expect("create channel")
    .id
}

/// A message in `channel`, signed `offset` seconds from now.
fn channel_message(
    keys: &Keys,
    channel: Uuid,
    kind: u16,
    content: &str,
    tags: &[&[&str]],
    offset: i64,
) -> Event {
    let mut all = vec![Tag::parse(["h", &channel.to_string()]).expect("h tag")];
    all.extend(
        tags.iter()
            .map(|tag| Tag::parse(tag.iter().copied()).expect("tag")),
    );
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(all)
        .custom_created_at(Timestamp::from(
            (Timestamp::now().as_secs() as i64 + offset) as u64,
        ))
        .sign_with_keys(keys)
        .expect("sign")
}

/// Delete `target` with a NIP-09 deletion, as the relay stores one.
async fn delete(db: &Db, community: CommunityId, keys: &Keys, channel: Uuid, target: &Event) {
    let deletion = channel_message(keys, channel, 5, "", &[&["e", &target.id.to_hex()]], 2);
    let (_, _, deleted, _) = db
        .insert_deletion_event(
            community,
            &deletion,
            Some(channel),
            None,
            &[DeletionTarget {
                event_id: target.id.as_bytes(),
                created_at: signed_at(target),
                parent_event_id: None,
                root_event_id: None,
            }],
            &Keys::generate(),
        )
        .await
        .expect("insert deletion");
    assert_eq!(deleted, vec![true]);
}

/// A deleted message no longer matches once the indexer catches up.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn a_deleted_message_no_longer_matches() {
    let db = sync_db().await;
    let (index, _) = sync_index().await;
    let community = search_suite::mk_community(db.pool(), "delete.example").await;
    let keys = Keys::generate();
    let channel = channel(&db, community, &keys).await;
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    let indexer = tokio::spawn(
        Indexer::new(db.clone(), index.clone())
            .with_new_index_settle(Duration::ZERO)
            .run(),
    );

    let doomed = channel_message(&keys, channel, 9, "deleteword doomed", &[], 0);
    let kept = channel_message(&keys, channel, 9, "deleteword kept", &[], 0);
    for event in [&doomed, &kept] {
        db.insert_event(community, event, Some(channel))
            .await
            .expect("insert");
    }
    wait_until_indexed(&db, &index).await;
    assert_eq!(find(&service, community, "deleteword").await.len(), 2);

    delete(&db, community, &keys, channel, &doomed).await;
    wait_until_indexed(&db, &index).await;
    assert_eq!(
        find(&service, community, "deleteword").await,
        vec![*kept.id.as_bytes()]
    );

    indexer.abort();
    index.delete().await.expect("delete index");
}

/// An edited message matches its new text and not its old text; deleting
/// the edit brings the old text back.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn an_edited_message_matches_its_current_text() {
    let db = sync_db().await;
    let (index, _) = sync_index().await;
    let community = search_suite::mk_community(db.pool(), "edit.example").await;
    let keys = Keys::generate();
    let channel = channel(&db, community, &keys).await;
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    let indexer = tokio::spawn(
        Indexer::new(db.clone(), index.clone())
            .with_new_index_settle(Duration::ZERO)
            .run(),
    );

    let original = channel_message(&keys, channel, 9, "oldword typo", &[], 0);
    db.insert_event(community, &original, Some(channel))
        .await
        .expect("insert");
    wait_until_indexed(&db, &index).await;
    assert_eq!(
        find(&service, community, "oldword").await,
        vec![*original.id.as_bytes()]
    );

    let target = original.id.to_hex();
    let first = channel_message(&keys, channel, 40003, "midword", &[&["e", &target]], 1);
    let latest = channel_message(
        &keys,
        channel,
        40003,
        "newword fixed",
        &[&["e", &target]],
        2,
    );
    for edit in [&latest, &first] {
        db.insert_event(community, edit, Some(channel))
            .await
            .expect("insert edit");
    }
    wait_until_indexed(&db, &index).await;
    assert!(find(&service, community, "oldword").await.is_empty());
    assert!(find(&service, community, "midword").await.is_empty());
    assert_eq!(
        find(&service, community, "newword").await,
        vec![*original.id.as_bytes()]
    );

    delete(&db, community, &keys, channel, &latest).await;
    delete(&db, community, &keys, channel, &first).await;
    wait_until_indexed(&db, &index).await;
    assert!(find(&service, community, "newword").await.is_empty());
    assert_eq!(
        find(&service, community, "oldword").await,
        vec![*original.id.as_bytes()]
    );

    indexer.abort();
    index.delete().await.expect("delete index");
}

/// Account deletion scrubs a message another person replied to and erases
/// the rest; neither matches its text afterwards, and the reply still does.
/// It runs as `buzz-admin user-deletions` does, without the outbox enabled:
/// the messages reach the index through the backfill.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn scrubbed_and_erased_messages_no_longer_match() {
    let db = unsynced_db().await;
    let (index, _) = sync_index().await;
    let community = search_suite::mk_community(db.pool(), "scrub.example").await;
    let leaving = Keys::generate();
    let staying = Keys::generate();
    let channel = channel(&db, community, &leaving).await;
    db.ensure_user(community, staying.public_key().as_bytes())
        .await
        .expect("ensure user");
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    let indexer = Indexer::new(db.clone(), index.clone());

    let root = channel_message(&leaving, channel, 9, "scrubword referenced", &[], -2);
    let erased = channel_message(&leaving, channel, 9, "scrubword alone", &[], -1);
    for event in [&root, &erased] {
        db.insert_event(community, event, Some(channel))
            .await
            .expect("insert");
    }
    let root_hex = root.id.to_hex();
    let reply = channel_message(
        &staying,
        channel,
        9,
        "replyword",
        &[
            &["e", &root_hex, "", "root"],
            &["e", &root_hex, "", "reply"],
        ],
        0,
    );
    db.insert_event_with_thread_metadata(
        community,
        &reply,
        Some(channel),
        Some(ThreadMetadataParams {
            event_id: reply.id.as_bytes(),
            event_created_at: signed_at(&reply),
            channel_id: channel,
            parent_event_id: Some(root.id.as_bytes()),
            parent_event_created_at: Some(signed_at(&root)),
            root_event_id: Some(root.id.as_bytes()),
            root_event_created_at: Some(signed_at(&root)),
            depth: 1,
            broadcast: false,
        }),
    )
    .await
    .expect("insert reply");
    assert_eq!(search_outbox::depth(db.pool()).await.expect("depth"), 0);
    indexer.backfill().await.expect("backfill");
    drain(&indexer, &index).await;
    assert_eq!(find(&service, community, "scrubword").await.len(), 2);

    let summary = db
        .user_deletion_store()
        .run_message_batches(
            &ContentBatchOptions {
                community: *community.as_uuid(),
                pubkey: leaving.public_key().to_hex(),
                relay_pubkey: None,
                batch_size: 100,
                max_items: 1000,
            },
            ExecutionMode::Delete,
            None,
            &mut Vec::new(),
        )
        .await
        .expect("delete the account's messages");
    assert_eq!(
        summary.scrubbed_events, 1,
        "the replied-to root is scrubbed"
    );
    drain(&indexer, &index).await;
    assert!(find(&service, community, "scrubword").await.is_empty());
    assert_eq!(
        find(&service, community, "replyword").await,
        vec![*reply.id.as_bytes()]
    );

    index.delete().await.expect("delete index");
}

/// Keys queued in the outbox, newest event first.
async fn queued_events(db: &Db) -> Vec<[u8; 32]> {
    sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT event_id FROM search_outbox ORDER BY event_created_at DESC, event_id DESC",
    )
    .fetch_all(db.pool())
    .await
    .expect("queued rows")
    .into_iter()
    .map(|id| id.try_into().expect("32-byte id"))
    .collect()
}

/// Index what is queued and make it visible to search.
async fn drain(indexer: &Indexer, index: &SearchIndex) {
    indexer.drain().await.expect("drain");
    index.refresh().await.expect("refresh");
}

/// Messages stored before OpenSearch was configured are backfilled newest
/// first, one bounded batch at a time. A restart resumes below the last
/// batch instead of starting over, live messages are indexed while the
/// backfill runs, and backfilling again creates no duplicate documents.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn stored_messages_are_backfilled_newest_first_and_resume() {
    let db = unsynced_db().await;
    let (index, prefix) = sync_index().await;
    let community = search_suite::mk_community(db.pool(), "backfill.example").await;
    // Another community, with nothing to backfill, takes its turns too.
    search_suite::mk_community(db.pool(), "bystander.example").await;
    let keys = Keys::generate();
    // Oldest first: stored[4] is the newest.
    let stored: Vec<Event> = (0..5)
        .map(|n| message_at(&keys, &format!("oldword {n}"), n - 10))
        .collect();
    for event in &stored {
        db.insert_event(community, event, None)
            .await
            .expect("insert");
    }
    let id = |event: &Event| *event.id.as_bytes();
    assert_eq!(search_outbox::depth(db.pool()).await.expect("depth"), 0);

    search_outbox::enable(db.pool())
        .await
        .expect("enable outbox");
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    // Two keys per batch, and no more queued than one batch at a time.
    let indexer = || {
        Indexer::new(db.clone(), index.clone())
            .with_batch_size(2)
            .with_backfill_bound(2)
    };

    // The first batch is the two newest messages.
    let first = indexer();
    assert_eq!(first.backfill().await.expect("backfill"), 2);
    assert_eq!(
        queued_events(&db).await,
        vec![id(&stored[4]), id(&stored[3])]
    );
    assert_eq!(
        first.backfill().await.expect("backfill"),
        0,
        "nothing more is queued while the outbox is at its bound"
    );
    drain(&first, &index).await;
    assert_eq!(
        find(&service, community, "oldword").await,
        vec![id(&stored[4]), id(&stored[3])]
    );

    // Stop here. A new indexer, as after a restart, continues below the
    // first batch.
    drop(first);
    let restarted = indexer();
    assert_eq!(restarted.backfill().await.expect("backfill"), 2);
    assert_eq!(
        queued_events(&db).await,
        vec![id(&stored[2]), id(&stored[1])]
    );

    // A live message admitted mid-backfill is indexed by the next drain.
    let live = message(&keys, "oldword live");
    db.insert_event(community, &live, None)
        .await
        .expect("insert live");
    drain(&restarted, &index).await;
    let completed: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT completed_at FROM search_backfill WHERE community_id = $1")
            .bind(community.as_uuid())
            .fetch_one(db.pool())
            .await
            .expect("marker");
    assert!(completed.is_none(), "the backfill is still running");
    let progress = buzz_db::search_backfill::progress(db.pool())
        .await
        .expect("progress");
    assert!(progress.contains(&(community, false)), "{progress:?}");
    assert_eq!(
        find(&service, community, "oldword").await,
        vec![
            id(&live),
            id(&stored[4]),
            id(&stored[3]),
            id(&stored[2]),
            id(&stored[1])
        ]
    );

    // The last batch is short, which completes the community.
    assert_eq!(restarted.backfill().await.expect("backfill"), 1);
    drain(&restarted, &index).await;
    assert_eq!(restarted.backfill().await.expect("backfill"), 0);
    let progress = buzz_db::search_backfill::progress(db.pool())
        .await
        .expect("progress");
    assert!(progress.contains(&(community, true)), "{progress:?}");
    let everything = vec![
        id(&live),
        id(&stored[4]),
        id(&stored[3]),
        id(&stored[2]),
        id(&stored[1]),
        id(&stored[0]),
    ];
    assert_eq!(find(&service, community, "oldword").await, everything);

    // Backfill everything again: the documents are rewritten, not doubled.
    sqlx::query("DELETE FROM search_backfill")
        .execute(db.pool())
        .await
        .expect("restart backfill");
    while restarted.backfill().await.expect("backfill") > 0 {
        drain(&restarted, &index).await;
    }
    drain(&restarted, &index).await;
    assert_eq!(find(&service, community, "oldword").await, everything);
    let count: serde_json::Value = raw_client()
        .count(opensearch::CountParts::Index(&[&format!(
            "{prefix}messages"
        )]))
        .send()
        .await
        .expect("count")
        .json()
        .await
        .expect("count body");
    assert_eq!(count["count"], 6, "{count}");
    index.delete().await.expect("delete index");
}

/// Wait until `count` messages of `community` match `q`, while the index
/// may still be missing.
async fn wait_until_found(
    index: &SearchIndex,
    service: &SearchService,
    community: CommunityId,
    q: &str,
    count: usize,
) {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            // Fails until the indexer has created the index.
            if index.refresh().await.is_ok() && find(service, community, q).await.len() == count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("every stored message becomes searchable");
}

/// The lease holder starts backfilling on its own once OpenSearch is
/// configured, and again into an index it recreates after the old one went
/// away, also when the relay stopped just before or just after creating it.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn the_indexer_backfills_on_its_own() {
    let db = unsynced_db().await;
    let (index, prefix) = sync_index().await;
    let proxy = Proxy::start().await;
    let community = search_suite::mk_community(db.pool(), "auto.example").await;
    let keys = Keys::generate();
    for n in 0..5 {
        db.insert_event(
            community,
            &message_at(&keys, &format!("autoword {n}"), -n),
            None,
        )
        .await
        .expect("insert");
    }
    search_outbox::enable(db.pool())
        .await
        .expect("enable outbox");
    let start = || {
        tokio::spawn(
            Indexer::new(db.clone(), proxy.index(&prefix))
                .with_batch_size(2)
                .with_backfill_bound(2)
                .with_new_index_settle(Duration::ZERO)
                .run(),
        )
    };
    let mut indexer = start();
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    wait_until_found(&index, &service, community, "autoword", 5).await;
    let progress = buzz_db::search_backfill::progress(db.pool())
        .await
        .expect("progress");
    assert!(progress.contains(&(community, true)), "{progress:?}");

    // The index goes away while the backfill is complete.
    index.delete().await.expect("delete index");
    wait_until_found(&index, &service, community, "autoword", 5).await;

    // The relay stops once before its create reaches the cluster, and once
    // right after the cluster created the index. Each time the restarted
    // relay backfills every message into the new index.
    for create in [Create::Refuse, Create::Lose] {
        let (done, stopped) = tokio::sync::oneshot::channel();
        proxy.set_create(create(done)).await;
        index.delete().await.expect("delete index");
        tokio::time::timeout(Duration::from_secs(30), stopped)
            .await
            .expect("the indexer creates the index")
            .expect("create signal");
        indexer.abort();
        let _ = indexer.await;
        indexer = start();
        wait_until_found(&index, &service, community, "autoword", 5).await;
    }
    indexer.abort();
    index.delete().await.expect("delete index");
}

/// While a new index is backfilled, no community reports a complete
/// backfill, including one whose progress was already cleared, as by an
/// operator's `DELETE FROM search_backfill` before deleting the index.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn no_community_reports_a_complete_backfill_while_it_runs_again() {
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let scrape = recorder.handle();
    // The test runtime has one thread, so the spawned indexer records here.
    let _recorder = metrics::set_default_local_recorder(&recorder);
    // What a scrape reports for the community, if anything.
    let complete = |community: CommunityId| {
        let series = format!("buzz_search_backfill_complete{{community=\"{community}\"}} ");
        scrape
            .render()
            .lines()
            .find_map(|line| line.strip_prefix(series.as_str()).map(str::to_owned))
    };

    let db = unsynced_db().await;
    let (index, prefix) = sync_index().await;
    let proxy = Proxy::start().await;
    let keys = Keys::generate();
    let mut communities = Vec::new();
    for host in ["first.example", "second.example"] {
        let community = search_suite::mk_community(db.pool(), host).await;
        for n in 0..3 {
            db.insert_event(
                community,
                &message_at(&keys, &format!("gaugeword {n}"), -n),
                None,
            )
            .await
            .expect("insert");
        }
        communities.push(community);
    }
    search_outbox::enable(db.pool())
        .await
        .expect("enable outbox");
    let indexer = tokio::spawn(
        Indexer::new(db.clone(), proxy.index(&prefix))
            .with_batch_size(2)
            .with_backfill_bound(2)
            .with_new_index_settle(Duration::ZERO)
            .run(),
    );
    tokio::time::timeout(Duration::from_secs(30), async {
        while !(communities
            .iter()
            .all(|community| complete(*community).as_deref() == Some("1"))
            && search_outbox::depth(db.pool()).await.expect("depth") == 0)
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("both backfills complete and are indexed");

    // Rebuild, holding the first write into the new index: the first
    // community's first batch is queued, the second's not yet. The hold
    // starts with the create, so a pass before the index delete passes.
    let (arrived, held) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    proxy
        .set_create(Create::Then(Bulk::Hold {
            arrived,
            release: released,
        }))
        .await;
    sqlx::query("DELETE FROM search_backfill")
        .execute(db.pool())
        .await
        .expect("clear backfill progress");
    index.delete().await.expect("delete index");
    tokio::time::timeout(Duration::from_secs(30), held)
        .await
        .expect("the indexer writes into the new index")
        .expect("hold signal");
    for community in &communities {
        assert_eq!(complete(*community).as_deref(), Some("0"), "{community}");
    }
    let _ = release.send(());
    indexer.abort();
    index.delete().await.expect("delete index");
}

/// Whether the index holds `event`'s document.
async fn indexed(prefix: &str, community: CommunityId, event: &Event) -> bool {
    let response = raw_client()
        .get(GetParts::IndexId(
            &format!("{prefix}messages"),
            &format!("{}:{}", community.as_uuid(), event.id.to_hex()),
        ))
        .send()
        .await
        .expect("get document");
    let document: serde_json::Value = response.json().await.expect("document body");
    document["found"] == true
}

/// What the proxy does with the next bulk request.
enum Bulk {
    /// Forward it.
    Pass,
    /// Answer HTTP 429, as Serverless does while it scales.
    Throttle,
    /// Signal `arrived`, wait for `release`, then forward it.
    Hold {
        arrived: tokio::sync::oneshot::Sender<()>,
        release: tokio::sync::oneshot::Receiver<()>,
    },
}

/// What the proxy does with the next index creation, signalling the
/// sender once it is done.
enum Create {
    /// Answer HTTP 503 without creating the index, as when the relay stops
    /// before its create reaches the cluster.
    Refuse(tokio::sync::oneshot::Sender<()>),
    /// Create the index, then answer HTTP 503, as when the relay stops
    /// right after the cluster created it.
    Lose(tokio::sync::oneshot::Sender<()>),
    /// Create the index, and apply this to the bulk requests that follow.
    Then(Bulk),
}

/// An HTTP proxy in front of the test cluster that can throttle or hold
/// bulk requests, and refuse or lose an index creation.
struct Proxy {
    url: String,
    bulk: std::sync::Arc<tokio::sync::Mutex<Bulk>>,
    create: std::sync::Arc<tokio::sync::Mutex<Option<Create>>>,
    server: tokio::task::JoinHandle<()>,
}

impl Proxy {
    async fn start() -> Self {
        let bulk = std::sync::Arc::new(tokio::sync::Mutex::new(Bulk::Pass));
        let create = std::sync::Arc::new(tokio::sync::Mutex::new(None));
        let state = (
            bulk.clone(),
            create.clone(),
            reqwest::Client::new(),
            opensearch_url(),
        );
        let app = axum::Router::new()
            .fallback(proxy_request)
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy");
        let url = format!("http://{}", listener.local_addr().expect("proxy address"));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve proxy");
        });
        Self {
            url,
            bulk,
            create,
            server,
        }
    }

    /// The index behind this proxy, using `prefix`.
    fn index(&self, prefix: &str) -> SearchIndex {
        SearchIndex::new(&OpenSearchConfig {
            url: self.url.clone(),
            basic_auth: None,
            aws_sigv4: None,
            index_prefix: prefix.into(),
        })
        .expect("proxied client")
    }

    async fn set(&self, bulk: Bulk) {
        *self.bulk.lock().await = bulk;
    }

    async fn set_create(&self, create: Create) {
        *self.create.lock().await = Some(create);
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.server.abort();
    }
}

type ProxyState = (
    std::sync::Arc<tokio::sync::Mutex<Bulk>>,
    std::sync::Arc<tokio::sync::Mutex<Option<Create>>>,
    reqwest::Client,
    String,
);

fn unavailable() -> axum::response::Response {
    axum::response::Response::builder()
        .status(503)
        .body(axum::body::Body::empty())
        .expect("unavailable response")
}

async fn proxy_request(
    axum::extract::State((bulk, create, client, upstream)): axum::extract::State<ProxyState>,
    request: axum::extract::Request,
) -> axum::response::Response {
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("request body");
    // An index creation is a PUT of the index name alone.
    let creates =
        parts.method == axum::http::Method::PUT && parts.uri.path().matches('/').count() == 1;
    let mut lost = None;
    if creates {
        match create.lock().await.take() {
            None => {}
            Some(Create::Refuse(done)) => {
                let _ = done.send(());
                return unavailable();
            }
            Some(Create::Lose(done)) => lost = Some(done),
            Some(Create::Then(next)) => *bulk.lock().await = next,
        }
    }
    if parts.uri.path().ends_with("/_bulk") {
        let action = std::mem::replace(&mut *bulk.lock().await, Bulk::Pass);
        match action {
            Bulk::Pass => {}
            Bulk::Throttle => {
                *bulk.lock().await = Bulk::Throttle;
                return axum::response::Response::builder()
                    .status(429)
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        r#"{"error":{"type":"too_many_requests"},"status":429}"#,
                    ))
                    .expect("throttled response");
            }
            Bulk::Hold { arrived, release } => {
                let _ = arrived.send(());
                let _ = release.await;
            }
        }
    }
    let path = parts.uri.path_and_query().map_or("/", |path| path.as_str());
    let mut forward = client
        .request(parts.method, format!("{upstream}{path}"))
        .body(body);
    if let Some(content_type) = parts.headers.get("content-type") {
        forward = forward.header("content-type", content_type);
    }
    let response = forward.send().await.expect("forward to OpenSearch");
    if let Some(done) = lost {
        assert!(response.status().is_success(), "the index is created");
        let _ = done.send(());
        return unavailable();
    }
    let mut builder = axum::response::Response::builder().status(response.status().as_u16());
    if let Some(content_type) = response.headers().get("content-type") {
        builder = builder.header("content-type", content_type);
    }
    builder
        .body(axum::body::Body::from(
            response.bytes().await.expect("OpenSearch response"),
        ))
        .expect("proxied response")
}

/// Throttling ends the pass as throttled, not failed, and keeps every row
/// queued; indexing completes once the cluster accepts writes again.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn throttling_keeps_rows_queued() {
    let db = sync_db().await;
    let (index, prefix) = sync_index().await;
    let proxy = Proxy::start().await;
    let community = search_suite::mk_community(db.pool(), "throttle.example").await;
    let keys = Keys::generate();
    let queued = [
        message(&keys, "slowword one"),
        message(&keys, "slowword two"),
    ];
    for event in &queued {
        db.insert_event(community, event, None)
            .await
            .expect("insert");
    }
    let indexer = Indexer::new(db.clone(), proxy.index(&prefix)).with_workers(2);

    proxy.set(Bulk::Throttle).await;
    let error = indexer.drain().await.expect_err("throttled");
    assert!(matches!(error, IndexerError::Throttled), "{error}");
    assert_eq!(search_outbox::depth(db.pool()).await.expect("depth"), 2);

    proxy.set(Bulk::Pass).await;
    let report = indexer.drain().await.expect("drain");
    assert_eq!(report.written, 2);
    index.refresh().await.expect("refresh");
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    assert_eq!(find(&service, community, "slowword").await.len(), 2);
    index.delete().await.expect("delete index");
}

/// A change committed while an earlier row for the same message is being
/// indexed keeps its own row and is indexed afterwards. The proxy holds the
/// worker's bulk request, after its re-read and before its delete.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn a_change_during_indexing_is_indexed_afterwards() {
    let db = sync_db().await;
    let (index, prefix) = sync_index().await;
    let proxy = Proxy::start().await;
    let community = search_suite::mk_community(db.pool(), "gate.example").await;
    let keys = Keys::generate();
    let channel = channel(&db, community, &keys).await;
    let original = channel_message(&keys, channel, 9, "firstword", &[], 0);
    db.insert_event(community, &original, Some(channel))
        .await
        .expect("insert");

    let (arrived_tx, arrived) = tokio::sync::oneshot::channel();
    let (release, release_rx) = tokio::sync::oneshot::channel();
    proxy
        .set(Bulk::Hold {
            arrived: arrived_tx,
            release: release_rx,
        })
        .await;
    let indexer = Indexer::new(db.clone(), proxy.index(&prefix));
    let drain = tokio::spawn(async move { indexer.drain().await });
    arrived.await.expect("the worker sent its batch");

    // The worker has read "firstword" and not yet deleted its row.
    let edit = channel_message(
        &keys,
        channel,
        40003,
        "secondword",
        &[&["e", &original.id.to_hex()]],
        1,
    );
    db.insert_event(community, &edit, Some(channel))
        .await
        .expect("insert edit");
    assert_eq!(search_outbox::depth(db.pool()).await.expect("depth"), 2);
    release.send(()).expect("release the batch");

    let report = drain.await.expect("drain task").expect("drain");
    assert_eq!(report.batches, 2, "the edit's row is its own batch");
    assert_eq!(search_outbox::depth(db.pool()).await.expect("depth"), 0);
    index.refresh().await.expect("refresh");
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    assert!(find(&service, community, "firstword").await.is_empty());
    assert_eq!(
        find(&service, community, "secondword").await,
        vec![*original.id.as_bytes()]
    );
    index.delete().await.expect("delete index");
}

/// A stored channel message's document with `content` as its text.
fn document_of(event: &Event, channel: Uuid, content: &str) -> SearchDocument {
    SearchDocument {
        event_id: event.id.to_bytes(),
        kind: 9,
        pubkey: event.pubkey.to_bytes(),
        channel_id: Some(channel),
        created_at: signed_at(event),
        content: content.into(),
        channel_type: Some("stream".into()),
        author_is_agent: false,
        thread_root: None,
        mentions: Vec::new(),
    }
}

/// A community of its own with one channel, and the channel owner's keys.
async fn late_community(db: &Db) -> (CommunityId, Keys, Uuid) {
    let community = search_suite::mk_community(
        db.pool(),
        &format!("late-{}.example", Uuid::new_v4().simple()),
    )
    .await;
    let keys = Keys::generate();
    let channel = channel(db, community, &keys).await;
    (community, keys, channel)
}

/// The next value of the version sequence batches read.
async fn next_version(db: &Db) -> i64 {
    sqlx::query_scalar("SELECT nextval('search_index_version')")
        .fetch_one(db.pool())
        .await
        .expect("next version")
}

/// A database layer whose connections read `clock_timestamp()` ahead by
/// the microseconds [`set_clock_skew`] last set, through a function on
/// their own search path. Nothing else on these connections changes.
async fn skewed_clock_db() -> Db {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .unwrap_or_else(|_| search_suite::TEST_DB_URL.into());
    let setup = Db::new(&DbConfig {
        database_url: url.clone(),
        ..DbConfig::default()
    })
    .await
    .expect("connect database layer");
    for statement in [
        "CREATE SCHEMA IF NOT EXISTS buzz_test_clock",
        "CREATE TABLE IF NOT EXISTS buzz_test_clock.skew (micros BIGINT NOT NULL)",
        "CREATE OR REPLACE FUNCTION buzz_test_clock.clock_timestamp() RETURNS timestamptz \
         LANGUAGE sql AS $$ SELECT pg_catalog.clock_timestamp() + make_interval(secs => \
         (SELECT coalesce(max(micros), 0) FROM buzz_test_clock.skew) / 1000000.0) $$",
    ] {
        sqlx::query(statement)
            .execute(setup.pool())
            .await
            .expect("install the skewed clock");
    }
    let separator = if url.contains('?') { '&' } else { '?' };
    Db::new(&DbConfig {
        database_url: format!(
            "{url}{separator}options=-c%20search_path%3Dbuzz_test_clock%2Cpg_catalog%2Cpublic"
        ),
        ..DbConfig::default()
    })
    .await
    .expect("connect skewed database layer")
}

/// Run every `clock_timestamp()` of a [`skewed_clock_db`] `micros` ahead.
async fn set_clock_skew(db: &Db, micros: i64) {
    sqlx::query(
        "WITH cleared AS (DELETE FROM buzz_test_clock.skew) \
         INSERT INTO buzz_test_clock.skew (micros) VALUES ($1)",
    )
    .bind(micros)
    .execute(db.pool())
    .await
    .expect("set the clock skew");
}

/// The index keeps each document's newest version: a bulk request the
/// cluster applies after a later one, as when its indexer timed out or
/// crashed with the request in flight, changes nothing. Its items come back
/// as conflicts, which nobody reads: its batch has ended.
/// Mutation: write without the external version → RED.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn a_late_write_cannot_undo_a_newer_one() {
    let db = sync_db().await;
    let (index, prefix) = sync_index().await;
    let (community, keys, channel) = late_community(&db).await;
    let event = channel_message(&keys, channel, 9, "firstword", &[], 0);
    db.insert_event(community, &event, Some(channel))
        .await
        .expect("insert");
    let document = |content: &str| document_of(&event, channel, content);
    let write = |documents: Vec<SearchDocument>, removed: Vec<[u8; 32]>, version: i64| {
        let index = index.clone();
        async move {
            index
                .write(community, &documents, &removed, version)
                .await
                .expect("write")
                .conflicts
        }
    };
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());

    // The edit's write lands first, then the original's late one.
    assert_eq!(write(vec![document("secondword")], vec![], 2).await, 0);
    assert_eq!(write(vec![document("firstword")], vec![], 1).await, 1);
    index.refresh().await.expect("refresh");
    assert!(find(&service, community, "firstword").await.is_empty());
    assert_eq!(
        find(&service, community, "secondword").await,
        vec![event.id.to_bytes()]
    );

    // The removal lands first, then the edit's late write.
    assert_eq!(write(vec![], vec![event.id.to_bytes()], 3).await, 0);
    assert_eq!(write(vec![document("secondword")], vec![], 2).await, 1);
    index.refresh().await.expect("refresh");
    assert!(!indexed(&prefix, community, &event).await);
    index.delete().await.expect("delete index");
}

/// A live batch is its events' newest writer, so a conflict it meets means
/// the version sequence handed out a value again, as after a Postgres crash
/// or promoting an asynchronous replica: here the document was
/// written at a version the sequence has not reached. The newer state stays
/// queued and the pass fails, and once the sequence passes the version the
/// index holds, it is written.
/// Mutation: count a version conflict as written → RED.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn a_newer_state_below_the_held_version_stays_queued() {
    let db = sync_db().await;
    let (index, _prefix) = sync_index().await;
    let (community, keys, channel) = late_community(&db).await;
    let original = channel_message(&keys, channel, 9, "firstword", &[], 0);
    db.insert_event(community, &original, Some(channel))
        .await
        .expect("insert");
    let ahead = next_version(&db).await + 1_000;
    let outcome = index
        .write(
            community,
            &[document_of(&original, channel, "firstword")],
            &[],
            ahead,
        )
        .await
        .expect("write ahead");
    assert!(outcome.failed.is_empty(), "{outcome:?}");
    let edit = channel_message(
        &keys,
        channel,
        40003,
        "secondword",
        &[&["e", &original.id.to_hex()]],
        1,
    );
    db.insert_event(community, &edit, Some(channel))
        .await
        .expect("insert edit");
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    let indexer = Indexer::new(db.clone(), index.clone());

    assert!(indexer.drain().await.is_err(), "a conflict fails the pass");
    assert!(search_outbox::depth(db.pool()).await.expect("depth") > 0);
    index.refresh().await.expect("refresh");
    assert!(find(&service, community, "secondword").await.is_empty());

    sqlx::query("SELECT setval('search_index_version', $1)")
        .bind(ahead)
        .execute(db.pool())
        .await
        .expect("move the sequence to the held version");
    drain(&indexer, &index).await;
    assert_eq!(search_outbox::depth(db.pool()).await.expect("depth"), 0);
    assert!(find(&service, community, "firstword").await.is_empty());
    assert_eq!(
        find(&service, community, "secondword").await,
        vec![original.id.to_bytes()]
    );
    index.delete().await.expect("delete index");
}

/// A batch's request can land after the batch has ended and a later batch
/// has written a newer state, as when its indexer timed out with the
/// request in flight. The later batch's version must be the larger one even
/// when the database clock stepped back in between; otherwise the late
/// request wins, the index keeps the older text, and no live batch ever
/// meets a conflict. Here the clock runs 10 s ahead for the first batch and
/// is put back before the second.
/// Mutation: read the version from `clock_timestamp()` → RED.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn a_late_request_from_before_a_clock_step_changes_nothing() {
    let db = sync_db().await;
    let (index, _prefix) = sync_index().await;
    let skewed = skewed_clock_db().await;
    let (community, keys, channel) = late_community(&db).await;
    let original = channel_message(&keys, channel, 9, "firstword", &[], 0);
    db.insert_event(community, &original, Some(channel))
        .await
        .expect("insert");

    // The first batch reads the original while the clock runs ahead; it
    // sends its request and ends without completing, so the request is in
    // flight and the row stays queued.
    set_clock_skew(&db, 10_000_000).await;
    let mut skip = Vec::new();
    let (documents, removed, version) = loop {
        let batch = search_outbox::claim_batch(skewed.pool(), 100, &skip)
            .await
            .expect("claim")
            .expect("a batch of the community");
        if batch.community() == community {
            break (
                batch.documents.clone(),
                batch.removed.clone(),
                batch.version,
            );
        }
        skip.push(batch.community());
    };
    assert_eq!(
        documents,
        vec![document_of(&original, channel, "firstword")]
    );

    // The clock steps back, the message is edited, and the next batch
    // writes the edit.
    set_clock_skew(&db, 0).await;
    let edit = channel_message(
        &keys,
        channel,
        40003,
        "secondword",
        &[&["e", &original.id.to_hex()]],
        1,
    );
    db.insert_event(community, &edit, Some(channel))
        .await
        .expect("insert edit");
    let indexer = Indexer::new(skewed.clone(), index.clone());
    drain(&indexer, &index).await;
    assert_eq!(search_outbox::depth(db.pool()).await.expect("depth"), 0);

    // The first batch's request lands now.
    let outcome = index
        .write(community, &documents, &removed, version)
        .await
        .expect("late write");
    index.refresh().await.expect("refresh");
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    assert!(
        find(&service, community, "firstword").await.is_empty(),
        "the late request put the original text back: {outcome:?}"
    );
    assert_eq!(
        find(&service, community, "secondword").await,
        vec![original.id.to_bytes()]
    );
    assert_eq!(outcome.conflicts, 1, "{outcome:?}");
    index.delete().await.expect("delete index");
}

/// Two workers drain the outbox together without losing a row or indexing
/// one twice, and afterwards the index holds exactly what Postgres does.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn two_workers_drain_without_loss() {
    let db = sync_db().await;
    let (index, prefix) = sync_index().await;
    let keys = Keys::generate();
    let mut stored = Vec::new();
    for n in 0..3 {
        let community = search_suite::mk_community(db.pool(), &format!("workers{n}.example")).await;
        let channel = channel(&db, community, &keys).await;
        for m in 0..30 {
            let event = channel_message(&keys, channel, 9, &format!("workword {n} {m}"), &[], -m);
            db.insert_event(community, &event, Some(channel))
                .await
                .expect("insert");
            stored.push((community, channel, event));
        }
    }
    let indexer = Indexer::new(db.clone(), index.clone())
        .with_batch_size(7)
        .with_workers(2);
    let report = indexer.drain().await.expect("drain");
    assert_eq!(
        (report.written, report.removed, report.failed),
        (90, 0, 0),
        "every row indexed exactly once"
    );
    assert_eq!(search_outbox::depth(db.pool()).await.expect("depth"), 0);

    // Delete every third message and edit every fifth live one.
    let mut deleted = Vec::new();
    for (i, (community, channel, event)) in stored.iter().enumerate() {
        if i % 3 == 0 {
            delete(&db, *community, &keys, *channel, event).await;
            deleted.push(event.id);
        } else if i % 5 == 0 {
            let edit = channel_message(
                &keys,
                *channel,
                40003,
                "rewritten",
                &[&["e", &event.id.to_hex()]],
                1,
            );
            db.insert_event(*community, &edit, Some(*channel))
                .await
                .expect("insert edit");
        }
    }
    indexer.drain().await.expect("drain");
    assert_eq!(search_outbox::depth(db.pool()).await.expect("depth"), 0);
    index.refresh().await.expect("refresh");
    for (community, _, event) in &stored {
        assert_eq!(
            indexed(&prefix, *community, event).await,
            !deleted.contains(&event.id),
            "{}",
            event.content
        );
    }
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    let mut rewritten = 0;
    for community in stored
        .iter()
        .map(|(community, _, _)| *community)
        .collect::<std::collections::BTreeSet<_>>()
    {
        rewritten += find(&service, community, "rewritten").await.len();
    }
    let expected = (0..stored.len())
        .filter(|i| i % 3 != 0 && i % 5 == 0)
        .count();
    assert_eq!(rewritten, expected);
    let count: serde_json::Value = raw_client()
        .count(opensearch::CountParts::Index(&[&format!(
            "{prefix}messages"
        )]))
        .send()
        .await
        .expect("count")
        .json()
        .await
        .expect("count body");
    assert_eq!(count["count"], 60, "{count}");
    index.delete().await.expect("delete index");
}

/// Several instances started at once end with exactly one lease holder, which
/// indexes what is queued.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn instances_started_together_elect_one_leader() {
    let db = sync_db().await;
    let (index, _) = sync_index().await;
    let community = search_suite::mk_community(db.pool(), "fleet.example").await;
    let event = message(&Keys::generate(), "fleetword");
    db.insert_event(community, &event, None)
        .await
        .expect("insert");
    let instances: Vec<_> = (0..4)
        .map(|_| {
            tokio::spawn(
                Indexer::new(db.clone(), index.clone())
                    .with_new_index_settle(Duration::ZERO)
                    .run(),
            )
        })
        .collect();
    let service = SearchService::with_opensearch(db.pool().clone(), index.clone());
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            index.refresh().await.expect("refresh");
            if !find(&service, community, "fleetword").await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the leader indexes the queued message");
    // The indexer lease: a session advisory lock on one bigint key, which
    // pg_locks shows as its high and low halves.
    let holders: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_locks \
         WHERE locktype = 'advisory' AND granted AND objsubid = 1 \
           AND database = (SELECT oid FROM pg_database WHERE datname = current_database()) \
           AND classid = $1::bigint::oid AND objid = $2::bigint::oid",
    )
    .bind(0x4255_5A5A_i64)
    .bind(0x5345_4152_i64)
    .fetch_one(db.pool())
    .await
    .expect("lease holders");
    assert_eq!(holders, 1);
    for instance in instances {
        instance.abort();
    }
    index.delete().await.expect("delete index");
}
