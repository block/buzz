//! CLI arguments and connection setup for the shared content handler.
use anyhow::{ensure, Result};
use buzz_db::{
    user_deletion::{content_batches::ContentBatchOptions, ExecutionMode},
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

pub(super) async fn run_command(args: Args) -> Result<i32> {
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
            None,
            &mut std::io::stdout(),
        )
        .await?;
    Ok(0)
}
