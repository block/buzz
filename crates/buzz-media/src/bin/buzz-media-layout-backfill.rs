//! Idempotently copy legacy media payloads into the sharded layout.

use anyhow::{Context, Result};
use buzz_media::migration::{objects_for_sidecar, parse_sidecar_key, RequestPacer};
use clap::Parser;

mod media_layout_common;
use media_layout_common::{verify_destination, CommonArgs, DestinationVerification};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackfillDecision {
    Skip,
    Copy,
}

fn decide_backfill(
    verification: DestinationVerification,
    sharded_key: &str,
) -> Result<BackfillDecision> {
    match verification {
        DestinationVerification::LegacySourceAbsent | DestinationVerification::Verified => {
            Ok(BackfillDecision::Skip)
        }
        DestinationVerification::DestinationMissing => Ok(BackfillDecision::Copy),
        DestinationVerification::ByteMismatch => {
            anyhow::bail!("destination verification failed: {sharded_key}")
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "buzz-media-layout-backfill")]
struct Args {
    #[command(flatten)]
    common: CommonArgs,
    /// Resume after this sidecar key. The final log line prints the next value.
    #[arg(long, env = "BUZZ_MEDIA_MIGRATION_START_AFTER")]
    start_after: Option<String>,
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
    let mut start_after = args.start_after.clone();
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
                let verification = verify_destination(&storage, &mut pacer, &object).await?;
                if matches!(verification, DestinationVerification::LegacySourceAbsent) {
                    tracing::info!(source = %object.legacy, destination = %object.sharded, "legacy source absent; skipping");
                }
                match decide_backfill(verification, &object.sharded)? {
                    BackfillDecision::Skip => {
                        skipped += 1;
                        continue;
                    }
                    BackfillDecision::Copy => {}
                }
                if args.dry_run {
                    tracing::info!(source = %object.legacy, destination = %object.sharded, "would copy");
                } else {
                    pacer.wait().await;
                    storage.copy(&object.legacy, &object.sharded).await?;
                    match verify_destination(&storage, &mut pacer, &object).await? {
                        DestinationVerification::Verified => {}
                        DestinationVerification::LegacySourceAbsent => {
                            anyhow::bail!(
                                "legacy source disappeared before verification: {}",
                                object.legacy
                            );
                        }
                        DestinationVerification::DestinationMissing => {
                            anyhow::bail!("destination verification failed: {}", object.sharded);
                        }
                        DestinationVerification::ByteMismatch => {
                            anyhow::bail!("destination verification failed: {}", object.sharded);
                        }
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

#[cfg(test)]
mod tests {
    use super::{decide_backfill, BackfillDecision};
    use crate::media_layout_common::DestinationVerification;

    #[test]
    fn backfill_skips_absent_legacy_source() {
        let decision = decide_backfill(
            DestinationVerification::LegacySourceAbsent,
            "media/aa/bb/hash.png",
        )
        .expect("absent legacy source should be skipped");

        assert_eq!(decision, BackfillDecision::Skip);
    }

    #[test]
    fn backfill_skips_already_verified_destination() {
        let decision = decide_backfill(DestinationVerification::Verified, "media/aa/bb/hash.png")
            .expect("verified destination should be skipped");

        assert_eq!(decision, BackfillDecision::Skip);
    }

    #[test]
    fn backfill_copies_when_destination_is_missing() {
        let decision = decide_backfill(
            DestinationVerification::DestinationMissing,
            "media/aa/bb/hash.png",
        )
        .expect("missing destination should be copied");

        assert_eq!(decision, BackfillDecision::Copy);
    }

    #[test]
    fn backfill_fails_closed_on_byte_mismatch() {
        let error = decide_backfill(
            DestinationVerification::ByteMismatch,
            "media/aa/bb/hash.png",
        )
        .expect_err("byte mismatch must fail");

        assert!(
            error
                .to_string()
                .contains("destination verification failed"),
            "unexpected error: {error}"
        );
    }
}
