//! PostgreSQL-only intake, approval, and execution of synthetic user-deletion requests.

use anyhow::Result;
use clap::Subcommand;
use uuid::Uuid;

/// Prepare and execute synthetic requests; these commands never delete real data.
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
    /// Approve the saved inventory; execution initializes the first stage.
    Approve {
        /// Request UUID returned by submit.
        id: Uuid,
        /// Approving operator identity (self-asserted for this demo).
        #[arg(long)]
        approved_by: String,
        /// Inventory digest from inspect; approval must match this exact inventory.
        #[arg(long)]
        inventory_digest: String,
    },
    /// Run dummy stages for an approved request; no real user data is changed.
    Run {
        /// Approved request UUID.
        id: Uuid,
        /// Worker identity; defaults to hostname and process id.
        #[arg(long)]
        executor_id: Option<String>,
        /// Execute one handler, transition to the next stage, and release ownership.
        #[arg(long)]
        next_step: bool,
        /// Inject a retryable failure when this dummy stage is reached.
        #[arg(long, value_parser = ["gate_user", "purge_db", "purge_agents"])]
        fail_stage: Option<String>,
        /// Delay each dummy action for inspection and shutdown testing (up to 60s).
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=60_000))]
        stage_delay_ms: u64,
        /// Simulate no agents in this community; may be repeated. Saves a runtime branch result.
        #[arg(long)]
        demo_no_agents_in: Vec<Uuid>,
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

/// Run synthetic workflow commands using the existing deletion database connection.
pub async fn run(command: Command) -> Result<i32> {
    let db = crate::connect_database().await?;
    let store = db.entity_deletion_store();
    match command {
        Command::Submit {
            target,
            requested_by,
            reason,
        } => crate::print_json(
            &db.user_deletion_store()
                .submit(&target, &requested_by, reason.as_deref())
                .await?,
        )?,
        Command::Approve {
            id,
            approved_by,
            inventory_digest,
        } => {
            db.user_deletion_store()
                .approve(id, &approved_by, &inventory_digest)
                .await?;
            crate::print_json(&store.inspect(id).await?)?;
        }
        Command::Run {
            id,
            executor_id,
            next_step,
            fail_stage,
            stage_delay_ms,
            demo_no_agents_in,
        } => {
            let code = worker::run(
                &store,
                id,
                &executor_id.unwrap_or_else(crate::default_executor_id),
                &worker::RunOptions {
                    next_step,
                    fail_stage,
                    stage_delay: std::time::Duration::from_millis(stage_delay_ms),
                    no_agents_in: demo_no_agents_in,
                },
                &crate::shutdown_token(),
            )
            .await?;
            crate::print_json(&store.inspect(id).await?)?;
            return Ok(code);
        }
        Command::List { limit } => crate::print_json(&store.list(limit).await?)?,
        Command::Inspect { id } => crate::print_json(&store.inspect(id).await?)?,
    }
    Ok(0)
}

mod handlers;
mod transition;
mod worker;
mod workflow;
