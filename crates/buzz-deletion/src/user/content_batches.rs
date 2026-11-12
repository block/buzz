//! Standalone batch command, deliberately outside the durable account worker.

use anyhow::Result;
use buzz_db::user_deletion::{content_batches::ContentBatchOptions, ExecutionMode};

pub(super) fn parse_pubkey(input: &str) -> std::result::Result<String, String> {
    buzz_db::user_deletion::normalize_user_deletion_pubkey(input).map_err(|error| error.to_string())
}

pub(super) async fn run(options: ContentBatchOptions, delete: bool) -> Result<i32> {
    let db = buzz_db::Db::connect_cold_start(
        buzz_db::DbConfig {
            database_url: crate::required_env("DATABASE_URL")?,
            max_connections: 1,
            min_connections: 0,
            default_transaction_read_only: !delete,
            ..buzz_db::DbConfig::default()
        },
        "user_batch_preview",
    )
    .await?;
    let stdout = std::io::stdout();
    let mode = if delete {
        ExecutionMode::Delete
    } else {
        ExecutionMode::ReadOnly
    };
    db.user_deletion_store()
        .run_message_batches(&options, mode, &mut stdout.lock())
        .await?;
    Ok(0)
}
