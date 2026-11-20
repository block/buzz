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
// - prefix_*: prefix mode is reworked with prefix analysis (issue 06);
// - deleted_events_are_excluded: deletes reach the index in issue 04.
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
}

use std::time::Duration;

use buzz_core::CommunityId;
use buzz_db::{search_outbox, Db, DbConfig};
use buzz_search::indexer::Indexer;
use buzz_search::opensearch::{OpenSearchConfig, SearchIndex};
use buzz_search::{ChannelScope, SearchMode, SearchQuery, SearchService, SearchSort};
use nostr::{Event, EventBuilder, Keys, Kind};
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
    let (pool, _) = search_suite::setup().await;
    search_outbox::enable(&pool).await.expect("enable outbox");
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
        index_prefix: prefix.clone(),
    })
    .expect("OpenSearch client");
    index.ensure().await.expect("create index");
    (index, prefix)
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
    let indexer = tokio::spawn(Indexer::new(db.clone(), index.clone()).run());

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

    let first = tokio::spawn(Indexer::new(db.clone(), index.clone()).run());
    let before = message(&keys, "leaseword before");
    db.insert_event(community, &before, None)
        .await
        .expect("insert");
    wait_until_indexed(&db, &index).await;

    // The first instance holds the lease, so the second only waits.
    let second = tokio::spawn(Indexer::new(db.clone(), index.clone()).run());
    first.abort();
    let _ = first.await;
    let after = message(&keys, "leaseword after");
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

/// A batch the cluster rejects leaves its rows queued and its community
/// skipped, while other communities are still indexed in the same pass.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn a_rejected_community_does_not_hold_back_others() {
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
        index_prefix: prefix,
    })
    .expect("client");

    let poisoned = search_suite::mk_community(db.pool(), "poisoned.example").await;
    let healthy = search_suite::mk_community(db.pool(), "healthy.example").await;
    let keys = Keys::generate();
    db.insert_event(poisoned, &message(&keys, &"x".repeat(40_000)), None)
        .await
        .expect("insert oversized term");
    db.insert_event(healthy, &message(&keys, "fine"), None)
        .await
        .expect("insert");

    let error = Indexer::new(db.clone(), index.clone())
        .drain()
        .await
        .expect_err("the pass reports the rejected batch");
    assert!(error.to_string().contains("rejected"), "{error}");
    let queued: Vec<Uuid> = sqlx::query_scalar("SELECT community_id FROM search_outbox")
        .fetch_all(db.pool())
        .await
        .expect("queued rows");
    assert_eq!(queued, vec![*poisoned.as_uuid()]);
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
