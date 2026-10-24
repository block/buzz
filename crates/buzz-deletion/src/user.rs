//! PostgreSQL-only intake commands for synthetic user-deletion requests.

use anyhow::Result;
use clap::Subcommand;
use uuid::Uuid;

/// Submit and inspect synthetic requests; these commands never execute deletion.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Save a request. An identical resubmission returns the existing row.
    Submit {
        /// Synthetic user label, for example demo:alice.
        #[arg(long)]
        target: String,
        /// Submitting operator identity.
        #[arg(long)]
        requested_by: String,
        /// Optional explanation.
        #[arg(long)]
        reason: Option<String>,
    },
    /// List synthetic requests, newest first.
    List {
        /// Maximum number of rows.
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=1000))]
        limit: u16,
    },
    /// Show a saved request by UUID.
    Inspect {
        /// Request UUID returned by submit.
        id: Uuid,
    },
}

/// Run intake commands using the existing deletion database connection.
pub async fn run(command: Command) -> Result<i32> {
    let db = crate::connect_database().await?;
    let store = db.entity_deletion_store();
    match command {
        Command::Submit {
            target,
            requested_by,
            reason,
        } => crate::print_json(
            &store
                .submit(&target, &requested_by, reason.as_deref())
                .await?,
        )?,
        Command::List { limit } => crate::print_json(&store.list(limit).await?)?,
        Command::Inspect { id } => crate::print_json(&store.get(id).await?)?,
    }
    Ok(0)
}
