//! Direct content cleanup and its workflow adapter share the database handler.
use anyhow::{ensure, Context, Result};
use buzz_db::{
    entity_deletion::{EntityDeletionStore, EntityLease, EntityStage},
    user_deletion::{content_batches::ContentBatchOptions, ExecutionMode, UserDeletionInventory},
    Db,
};
use uuid::Uuid;

/// Arguments for direct offline content cleanup.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Human public key, as npub or hex.
    #[arg(long, value_parser = parse_pubkey)]
    pubkey: String,
    /// Community whose content to inspect or delete.
    #[arg(long)]
    community: Uuid,
    /// Print selected rows without changing data (the default).
    #[arg(long, alias = "print-only", conflicts_with = "delete")]
    read_only: bool,
    /// Delete selected rows. Stop relay and external workers first.
    #[arg(long)]
    delete: bool,
    /// Trusted relay signer for historical actor/p attribution.
    #[arg(long, value_parser = parse_pubkey)]
    relay_pubkey: Option<String>,
    /// Maximum explicit row operations per batch.
    #[arg(long, default_value_t=100, value_parser=clap::value_parser!(u16).range(1..=100))]
    batch_size: u16,
    /// Maximum row operations per invocation.
    #[arg(long, default_value_t=1_000_000, value_parser=clap::value_parser!(u32).range(1..=1_000_000))]
    max_items: u32,
}

pub(super) fn parse_pubkey(input: &str) -> std::result::Result<String, String> {
    buzz_db::user_deletion::normalize_user_deletion_pubkey(input).map_err(|error| error.to_string())
}

pub(super) async fn run(args: Args) -> Result<i32> {
    ensure!(
        !(args.read_only && args.delete),
        "select one execution mode"
    );
    let db = Db::connect_cold_start(
        buzz_db::DbConfig {
            database_url: crate::required_env("DATABASE_URL")?,
            max_connections: 1,
            min_connections: 0,
            default_transaction_read_only: !args.delete,
            ..buzz_db::DbConfig::default()
        },
        "user_content_batches",
    )
    .await?;
    db.user_deletion_store()
        .run_message_batches(
            &ContentBatchOptions {
                community: args.community,
                pubkey: args.pubkey,
                relay_pubkey: args.relay_pubkey,
                batch_size: args.batch_size,
                max_items: args.max_items,
            },
            if args.delete {
                ExecutionMode::Delete
            } else {
                ExecutionMode::ReadOnly
            },
            &mut std::io::stdout(),
        )
        .await?;
    Ok(0)
}

pub(super) async fn dispatch(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &super::worker::RunOptions,
) -> Result<()> {
    ensure!(
        options.delete,
        "real workflow execution requires --delete; use content-batches --read-only to inspect"
    );
    let db = options.db.as_ref().context("missing content database")?;
    let request = store.get(lease.request_id).await?;
    let inventory = UserDeletionInventory::read(&request)?;
    let community = row
        .target_community_id
        .context("missing content community")?;
    ensure!(
        row.request_id == request.id
            && row.metadata_json["inputs"] == inventory.inputs(&request.target, Some(community))?,
        "content stage differs from approved inventory"
    );
    store.start_stage(lease, row.id).await?;
    let result = async {
        let summary = db
            .user_deletion_store()
            .delete_message_stage(
                &ContentBatchOptions {
                    community,
                    pubkey: request.target.clone(),
                    relay_pubkey: options.relay_pubkey.clone(),
                    batch_size: options.batch_size,
                    max_items: options.max_items,
                },
                lease,
                row.id,
                &mut std::io::stdout(),
            )
            .await?;
        // Agent cleanup is a separate handler; do not claim it is unnecessary.
        Ok::<_, anyhow::Error>(
            serde_json::json!({"content": summary, "account_erasure_complete": false}),
        )
    }
    .await;
    match result {
        Ok(result) => store.save_stage_success(lease, row.id, result).await?,
        Err(error) => {
            store
                .fail_stage(lease, row.id, &format!("{error:#}"), crate::RETRY_DELAY)
                .await?;
            return Err(error);
        }
    }
    Ok(())
}
