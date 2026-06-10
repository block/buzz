//! One-shot admin tool: re-index all kind:0 (user metadata) events in Typesense.
//!
//! Necessary after the indexer change that appends `display_name`/`name`/`nip05`
//! values to the indexed content for kind:0 docs (see `sprout-search`'s
//! `flatten_kind0_for_indexing`). Existing docs need to be rewritten with the
//! appended tokens before they become searchable by display name.
//!
//! New / updated kind:0 events index correctly automatically — this tool only
//! exists to backfill the existing population.
//!
//! Usage (from the repo root, with .env sourced):
//!
//! ```
//! cargo run --release -p buzz-relay --bin sprout-reindex-kind0
//! ```
//!
//! Idempotent — Typesense uses upsert semantics, so running twice is safe.
//! Streams in batches so memory stays bounded regardless of relay size.
//!
//! ## Paging
//!
//! Walks `query_events` with a snapshot ceiling (`until = now()` at start) plus
//! a keyset cursor over `(created_at, id)` matching the underlying
//! `ORDER BY created_at DESC, id ASC` index. This guarantees:
//!
//! - No rows are skipped if new kind:0 events arrive during the run
//!   (they're newer than the snapshot, so they fall outside the predicate).
//! - No rows are double-counted at page boundaries (the cursor advances
//!   strictly past the last row of each batch).
//! - Bounded total work — won't chase its own tail under live write traffic.
//!
//! Newly-arrived kind:0 events that fall outside the snapshot are indexed by
//! the relay's live write path anyway, so this backfill plus the live path
//! together cover the full population.

use anyhow::Context;
use chrono::{DateTime, Utc};
use tracing::{info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use buzz_db::{Db, DbConfig, EventQuery};
use buzz_relay::config::Config;
use buzz_search::{SearchConfig, SearchService};

/// Page size for the SQL → Typesense pipeline. Small enough to keep DB and
/// Typesense memory comfortable, large enough to amortise per-batch overhead.
const BATCH: i64 = 500;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(
            EnvFilter::from_default_env()
                .add_directive("sprout_reindex_kind0=info".parse()?)
                .add_directive("buzz_relay=info".parse()?),
        )
        .init();

    let config = Config::from_env().context("loading relay config from environment")?;

    let db_config = DbConfig {
        database_url: config.database_url.clone(),
        ..DbConfig::default()
    };
    let db = Db::new(&db_config)
        .await
        .context("connecting to postgres")?;

    // SearchConfig::default() reads TYPESENSE_URL / TYPESENSE_API_KEY /
    // TYPESENSE_COLLECTION from the environment, same as the relay does.
    let search = SearchService::new(SearchConfig::default());
    search
        .ensure_collection()
        .await
        .context("ensuring Typesense collection")?;

    // Snapshot ceiling: we only reindex events that already exist at start.
    // Anything newer is handled by the relay's live indexing path.
    let snapshot: DateTime<Utc> = Utc::now();

    // Keyset cursor over (created_at, id) — matches the underlying
    // `ORDER BY created_at DESC, id ASC` index. On the first iteration both
    // cursor fields are None and the predicate reduces to `created_at <= snapshot`.
    // Subsequent iterations advance to strictly past the last row of the prior batch.
    let mut cursor_until: DateTime<Utc> = snapshot;
    let mut cursor_before_id: Option<Vec<u8>> = None;

    let mut total_indexed: usize = 0;
    let mut total_failed: usize = 0;
    let mut batches: usize = 0;

    info!(?snapshot, "starting kind:0 reindex");

    loop {
        let q = EventQuery {
            kinds: Some(vec![0]),
            limit: Some(BATCH),
            max_limit: Some(BATCH),
            until: Some(cursor_until),
            before_id: cursor_before_id.clone(),
            ..EventQuery::default()
        };

        let batch = db
            .query_events(&q)
            .await
            .context("querying kind:0 events")?;

        if batch.is_empty() {
            break;
        }

        let batch_len = batch.len();

        // Capture the tail of the batch for cursor advance *before* the index
        // call, so we still advance even if indexing fails for this batch.
        // (We'd otherwise loop forever on a poisoned batch.)
        let tail = batch
            .last()
            .map(|ev| {
                let ts = ev.event.created_at.as_secs() as i64;
                let dt = DateTime::<Utc>::from_timestamp(ts, 0).unwrap_or(cursor_until);
                let id_bytes = ev.event.id.to_bytes().to_vec();
                (dt, id_bytes)
            })
            .expect("batch is non-empty (checked above)");

        match search.index_batch(&batch).await {
            Ok(indexed) => {
                total_indexed += indexed;
                if indexed < batch_len {
                    let failed = batch_len - indexed;
                    total_failed += failed;
                    warn!(failed, batch_len, "some events failed to index in batch");
                }
                info!(indexed, batch_len, batches, total_indexed, "indexed batch");
            }
            Err(e) => {
                warn!(error = %e, batch_len, batches, "batch index failed entirely");
                total_failed += batch_len;
            }
        }

        batches += 1;

        // Tail of the prior batch becomes the cursor for the next page.
        // `query_events` will use the composite predicate
        //   created_at < cursor_until OR (created_at = cursor_until AND id > cursor_before_id)
        // which exactly skips past the last row we just processed.
        cursor_until = tail.0;
        cursor_before_id = Some(tail.1);

        // If we got fewer than BATCH back, we're at the tail of the table.
        if (batch_len as i64) < BATCH {
            break;
        }
    }

    info!(
        total_indexed,
        total_failed, batches, "kind:0 reindex complete"
    );
    if total_failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
