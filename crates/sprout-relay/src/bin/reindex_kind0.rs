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
//! cargo run --release -p sprout-relay --bin sprout-reindex-kind0
//! ```
//!
//! Idempotent — Typesense uses upsert semantics, so running twice is safe.
//! Streams in batches so memory stays bounded regardless of relay size.

use anyhow::Context;
use tracing::{info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use sprout_db::{Db, DbConfig, EventQuery};
use sprout_relay::config::Config;
use sprout_search::{SearchConfig, SearchService};

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
                .add_directive("sprout_relay=info".parse()?),
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

    let mut offset: i64 = 0;
    let mut total_indexed: usize = 0;
    let mut total_failed: usize = 0;

    info!("starting kind:0 reindex");

    loop {
        let q = EventQuery {
            kinds: Some(vec![0]),
            limit: Some(BATCH),
            offset: Some(offset),
            max_limit: Some(BATCH),
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
        match search.index_batch(&batch).await {
            Ok(indexed) => {
                total_indexed += indexed;
                if indexed < batch_len {
                    let failed = batch_len - indexed;
                    total_failed += failed;
                    warn!(failed, batch_len, "some events failed to index in batch");
                }
                info!(indexed, batch_len, offset, total_indexed, "indexed batch");
            }
            Err(e) => {
                warn!(error = %e, batch_len, offset, "batch index failed entirely");
                total_failed += batch_len;
            }
        }

        // If we got fewer than BATCH back, we're at the tail of the table.
        if (batch_len as i64) < BATCH {
            break;
        }
        offset += BATCH;
    }

    info!(total_indexed, total_failed, "kind:0 reindex complete");
    if total_failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
