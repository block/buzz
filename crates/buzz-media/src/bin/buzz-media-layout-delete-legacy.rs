//! Delete verified legacy media payloads after migration reconciliation.

use anyhow::{Context, Result};
use buzz_media::migration::{objects_for_sidecar, parse_sidecar_key, RequestPacer};
use buzz_media::MediaStorage;
use clap::Parser;

mod media_layout_common;
use media_layout_common::{verify_destination, CommonArgs};

const CONFIRMATION: &str = "delete-verified-legacy-media";

#[derive(Debug, Parser)]
#[command(name = "buzz-media-layout-delete-legacy")]
struct Args {
    #[command(flatten)]
    common: CommonArgs,
    /// Preview is the safe default. Set false only after reconciliation.
    #[arg(long, env = "BUZZ_MEDIA_MIGRATION_DRY_RUN", default_value_t = true, action = clap::ArgAction::Set)]
    dry_run: bool,
    /// Required with --dry-run=false to guard accidental destructive Jobs.
    #[arg(long, env = "BUZZ_MEDIA_DELETE_CONFIRM")]
    confirm: Option<String>,
}

/// Scan all selected sidecars before deleting anything. This two-pass design is
/// important because one flat legacy CAS key can serve multiple communities:
/// every community destination must exist before the shared source is removed.
async fn scan(
    storage: &MediaStorage,
    common: &CommonArgs,
    pacer: &mut RequestPacer,
    delete: bool,
    dry_run: bool,
) -> Result<(u64, u64, Option<String>)> {
    let mut continuation = None;
    let mut start_after = common.start_after.clone();
    let mut changed = 0_u64;
    let mut skipped = 0_u64;
    let mut checkpoint = start_after.clone();
    loop {
        pacer.wait().await;
        let page = storage
            .list_prefix_page(
                "_meta/",
                continuation.take(),
                start_after.take(),
                common.page_size,
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
                if !delete {
                    pacer.wait().await;
                    if !storage.head(&object.sharded).await? {
                        anyhow::bail!(
                            "refusing deletion: sharded destination missing: {} (checkpoint: {sidecar_key})",
                            object.sharded
                        );
                    }
                    continue;
                }
                pacer.wait().await;
                if !storage.head(&object.legacy).await? {
                    skipped += 1;
                    continue;
                }
                if dry_run {
                    tracing::info!(key = %object.legacy, "would delete verified legacy object");
                } else {
                    if !verify_destination(storage, pacer, &object).await? {
                        anyhow::bail!(
                            "refusing deletion: sharded destination bytes do not match legacy source: {} (checkpoint: {sidecar_key})",
                            object.sharded
                        );
                    }
                    pacer.wait().await;
                    storage.delete(&object.legacy).await?;
                    changed += 1;
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
    Ok((changed, skipped, checkpoint))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_target(false).init();
    let args = Args::parse();
    if !args.dry_run && args.confirm.as_deref() != Some(CONFIRMATION) {
        anyhow::bail!("destructive mode requires --confirm={CONFIRMATION}");
    }
    let storage = args.common.storage()?;
    let mut pacer = RequestPacer::new(args.common.requests_per_second);
    let (_, _, verified_checkpoint) =
        scan(&storage, &args.common, &mut pacer, false, args.dry_run).await?;
    tracing::info!(checkpoint = ?verified_checkpoint, "all selected sharded destinations exist; beginning deletion pass");
    let (deleted, skipped, checkpoint) =
        scan(&storage, &args.common, &mut pacer, true, args.dry_run).await?;
    tracing::info!(deleted, skipped, dry_run = args.dry_run, checkpoint = ?checkpoint, "legacy deletion complete");
    Ok(())
}
