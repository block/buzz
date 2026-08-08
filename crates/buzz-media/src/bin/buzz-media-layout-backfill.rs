//! Idempotently copy legacy media payloads into the sharded layout.

use anyhow::{Context, Result};
use buzz_media::migration::{objects_for_sidecar, parse_sidecar_key, RequestPacer};
use clap::Parser;

mod media_layout_common;
use media_layout_common::{verify_destination, CommonArgs};

#[derive(Debug, Parser)]
#[command(name = "buzz-media-layout-backfill")]
struct Args {
    #[command(flatten)]
    common: CommonArgs,
    /// Report actions without copying objects.
    #[arg(long, env = "BUZZ_MEDIA_MIGRATION_DRY_RUN", default_value_t = false)]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_target(false).init();
    let args = Args::parse();
    let storage = args.common.storage()?;
    let mut pacer = RequestPacer::new(args.common.requests_per_second);
    let mut continuation = None;
    let mut start_after = args.common.start_after.clone();
    let mut processed = 0_u64;
    let mut copied = 0_u64;
    let mut skipped = 0_u64;
    let mut checkpoint = start_after.clone();

    loop {
        pacer.wait().await;
        let page = storage
            .list_prefix_page(
                "_meta/",
                continuation.take(),
                start_after.take(),
                args.common.page_size,
            )
            .await
            .context("list media sidecars")?;
        for (sidecar_key, _) in page.objects {
            let Some((community, sha)) = parse_sidecar_key(&sidecar_key) else {
                tracing::warn!(key = %sidecar_key, "skipping malformed sidecar key");
                continue;
            };
            pacer.wait().await;
            let bytes = storage
                .get(&sidecar_key)
                .await
                .context("read media sidecar")?;
            let meta = serde_json::from_slice(&bytes).context("parse media sidecar")?;
            for object in objects_for_sidecar(community, sha, &meta)? {
                processed += 1;
                pacer.wait().await;
                if verify_destination(&storage, &mut pacer, &object).await? {
                    skipped += 1;
                    continue;
                }
                pacer.wait().await;
                let source_exists = storage.head(&object.legacy).await?;
                if !source_exists {
                    anyhow::bail!(
                        "legacy source missing: {} (checkpoint: {sidecar_key})",
                        object.legacy
                    );
                }
                if args.dry_run {
                    tracing::info!(source = %object.legacy, destination = %object.sharded, "would copy");
                } else {
                    pacer.wait().await;
                    storage.copy(&object.legacy, &object.sharded).await?;
                    if !verify_destination(&storage, &mut pacer, &object).await? {
                        anyhow::bail!("destination verification failed: {}", object.sharded);
                    }
                    copied += 1;
                }
            }
            checkpoint = Some(sidecar_key);
        }
        if !page.is_truncated {
            break;
        }
        continuation = page.next_continuation_token;
        if continuation.is_none() {
            anyhow::bail!("truncated S3 listing returned no continuation token");
        }
    }
    tracing::info!(processed, copied, skipped, dry_run = args.dry_run, checkpoint = ?checkpoint, "backfill complete");
    Ok(())
}
