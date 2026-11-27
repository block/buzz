//! Account deletion queues search index changes in its own process.
//!
//! `buzz-admin user-deletions` never calls `search_outbox::enable`, so these
//! tests leave it off, as that process does. nextest runs each test in its
//! own process, so the flag one test turns on does not leak into the other.
use super::scrub_postgres_tests::{message, options, setup};
use crate::user_deletion::ExecutionMode;

/// When an indexing relay first runs against the database.
#[derive(PartialEq)]
enum Indexer {
    Never,
    BeforeTheJob,
    /// After the job has planned its batches, before it runs them.
    DuringTheJob,
}

/// Job output that, once the job starts deleting, takes a value from the
/// version sequence as an indexing relay does when it starts.
struct IndexerStartsMidJob {
    pool: sqlx::PgPool,
    output: Vec<u8>,
    started: bool,
}

impl std::io::Write for IndexerStartsMidJob {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.output.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.started && self.output.windows(12).any(|w| w == b"delete_start") {
            self.started = true;
            let pool = self.pool.clone();
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(sqlx::query("SELECT nextval('search_index_version')").execute(&pool))
            })
            .map_err(std::io::Error::other)?;
        }
        Ok(())
    }
}

/// Scrub one message (it has a reply from someone else) and erase another,
/// and return the queued event ids and the two messages' ids.
async fn delete_account(indexer: Indexer) -> (Vec<Vec<u8>>, [Vec<u8>; 2]) {
    let (db, community, channel, alice, bob) = setup().await;
    let root = message(&db, community, channel, &alice, "referenced", None).await;
    message(
        &db,
        community,
        channel,
        &bob,
        "reply",
        Some((&root, &root, 1)),
    )
    .await;
    let alone = message(&db, community, channel, &alice, "alone", None).await;
    if indexer == Indexer::BeforeTheJob {
        // What an indexing relay's backfill leaves behind.
        sqlx::query("INSERT INTO search_backfill (community_id, completed_at) VALUES ($1, now())")
            .bind(community.as_uuid())
            .execute(db.pool())
            .await
            .unwrap();
    }
    let mut output = IndexerStartsMidJob {
        pool: db.pool().clone(),
        output: Vec::new(),
        started: indexer != Indexer::DuringTheJob,
    };
    let summary = db
        .user_deletion_store()
        .run_message_batches(
            &options(community, &alice),
            ExecutionMode::Delete,
            None,
            &mut output,
        )
        .await
        .unwrap();
    assert_eq!(
        summary.scrubbed_events, 1,
        "the replied-to root is scrubbed"
    );
    let queued: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT event_id FROM search_outbox WHERE community_id = $1 ORDER BY event_id",
    )
    .bind(community.as_uuid())
    .fetch_all(db.pool())
    .await
    .unwrap();
    (
        queued,
        [root.id.as_bytes().to_vec(), alone.id.as_bytes().to_vec()],
    )
}

/// Once an indexer maintains this database, the scrubbed and the erased
/// message are queued, so their documents leave the index.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn account_deletion_queues_its_changes_when_an_index_is_maintained() {
    let (queued, mut deleted) = delete_account(Indexer::BeforeTheJob).await;
    deleted.sort();
    assert_eq!(queued, deleted.to_vec());
}

/// An indexer that starts while the job runs may already have indexed the
/// messages the job removes, so their changes are queued too.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires Postgres"]
async fn account_deletion_queues_its_changes_when_indexing_starts_mid_job() {
    let (queued, mut deleted) = delete_account(Indexer::DuringTheJob).await;
    deleted.sort();
    assert_eq!(queued, deleted.to_vec());
}

/// Without an indexer nothing would drain the outbox, so nothing is queued.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn account_deletion_queues_nothing_without_an_index() {
    let (queued, _) = delete_account(Indexer::Never).await;
    assert!(queued.is_empty());
}
