//! User inventory, offline account cleanup and persisted deletion workflows.

use anyhow::{bail, Result};
use buzz_db::user_deletion::ExecutionMode;
use clap::Subcommand;
use uuid::Uuid;

/// Inspect user data, run account cleanup or execute persisted workflows.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Inspect or delete one human's community content.
    ContentBatches(content_batches::Args),
    /// Run media cleanup directly, without a workflow request.
    MediaBatches(media::Args),
    /// Preview or delete profiles in one community; deletion requires stopped writers.
    ProfileBatches(profile_batches::Args),
    /// Print account-table batches for one community; --delete executes offline.
    AccountBatches(account_batches::Args),
    /// Capture frozen human/agent inputs across stored communities, read-only.
    AccountScope {
        /// Human public key, hex or npub.
        #[arg(long, value_parser = inventory::parse_subject)]
        pubkey: String,
    },
    /// Print real current memberships as JSON; does not save or execute a request.
    Inventory {
        /// Demo fixture (default), hex public key, or npub. Current memberships only.
        #[arg(long, default_value = "demo", value_parser = inventory::parse_subject)]
        pubkey: String,
    },
    /// Save a request. An identical resubmission returns the existing row.
    Submit {
        /// Optional demo request label, for example demo:alice. Conflicts with --pubkey.
        #[arg(long, conflicts_with = "pubkey")]
        target: Option<String>,
        /// Demo fixture (default), hex public key, or npub to discover and save.
        #[arg(long, value_parser = inventory::parse_subject)]
        pubkey: Option<String>,
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
    /// Run an approved workflow; real handlers require explicit --delete.
    Run {
        /// Approved request UUID.
        id: Uuid,
        /// Execute implemented real handlers; stop relay and external writers first.
        #[arg(long)]
        delete: bool,
        /// Trusted relay signer for historical content attribution.
        #[arg(long, value_parser=content_batches::parse_pubkey)]
        relay_pubkey: Option<String>,
        /// Maximum explicit row operations per batch.
        #[arg(long, default_value_t=100, value_parser=clap::value_parser!(u16).range(1..=100))]
        batch_size: u16,
        /// Maximum row operations in one handler invocation.
        #[arg(long, default_value_t=100_000, value_parser=clap::value_parser!(u32).range(1..=1_000_000))]
        max_items: u32,
        /// Worker identity; defaults to hostname and process id.
        #[arg(long)]
        executor_id: Option<String>,
        /// Execute one handler, transition to the next stage, and release ownership.
        #[arg(long)]
        next_step: bool,
        /// Inject a retryable failure when this dummy stage is reached.
        #[arg(long, value_parser = workflow::parse_stage)]
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

/// Run the selected user-deletion command against PostgreSQL.
pub async fn run(command: Command) -> Result<i32> {
    let command = match command {
        Command::ContentBatches(args) => return content_batches::run_command(args).await,
        Command::MediaBatches(args) => return media::run(args).await,
        Command::ProfileBatches(args) => return profile_batches::run(args).await,
        command => command,
    };
    if let Command::AccountBatches(args) = command {
        let mode = if args.delete {
            ExecutionMode::Delete
        } else {
            ExecutionMode::ReadOnly
        };
        // Command setup stays outside the handler. Neither mode runs startup
        // migrations or connects to Redis; preview defaults to read-only SQL.
        let db = buzz_db::Db::connect_cold_start(
            buzz_db::DbConfig {
                database_url: crate::required_env("DATABASE_URL")?,
                max_connections: 1,
                min_connections: 0,
                default_transaction_read_only: mode == ExecutionMode::ReadOnly,
                ..buzz_db::DbConfig::default()
            },
            "user_account_batches",
        )
        .await?;
        let summary = account_batches::run_command(&db, args, mode).await?;
        return Ok(if summary.supported_scope_exhausted {
            0
        } else {
            2
        });
    }
    let db = if matches!(
        command,
        Command::Inventory { .. } | Command::AccountScope { .. }
    ) {
        buzz_db::Db::connect_cold_start(
            buzz_db::DbConfig {
                database_url: crate::required_env("DATABASE_URL")?,
                max_connections: 1,
                min_connections: 0,
                default_transaction_read_only: true,
                ..buzz_db::DbConfig::default()
            },
            "user_inventory",
        )
        .await?
    } else {
        crate::connect_database().await?
    };
    let store = db.entity_deletion_store();
    match command {
        Command::ContentBatches(_) | Command::MediaBatches(_) | Command::ProfileBatches(_) => {
            bail!("direct cleanup command dispatched above")
        }
        Command::AccountBatches(_) => anyhow::bail!("account command dispatched above"),
        Command::AccountScope { pubkey } => {
            crate::print_json(
                &db.user_deletion_store()
                    .generate_account_scope(&pubkey)
                    .await?,
            )?;
        }
        Command::Inventory { pubkey } => {
            crate::print_json(&inventory::generate(&db, &pubkey).await?)?;
        }
        Command::Submit {
            target,
            pubkey,
            requested_by,
            reason,
        } => {
            let target = target.unwrap_or_else(|| match pubkey.as_deref() {
                None | Some("demo") => "demo:default".to_owned(),
                Some(key) => key.to_owned(),
            });
            crate::print_json(
                &db.user_deletion_store()
                    .submit(&target, &requested_by, reason.as_deref())
                    .await?,
            )?;
        }
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
            delete,
            relay_pubkey,
            batch_size,
            max_items,
            executor_id,
            next_step,
            fail_stage,
            stage_delay_ms,
            demo_no_agents_in,
        } => {
            let request = store.get(id).await?;
            let real = !request.target.starts_with("demo:");
            if real {
                anyhow::ensure!(delete, "real workflow execution requires --delete; use account-batches --read-only to preview");
                anyhow::ensure!(
                    fail_stage.is_none() && stage_delay_ms == 0 && demo_no_agents_in.is_empty(),
                    "dummy controls cannot be used for real deletion"
                );
            }
            let code = worker::run(
                &store,
                id,
                &executor_id.unwrap_or_else(crate::default_executor_id),
                &worker::RunOptions {
                    db: real.then(|| buzz_db::Db::from_pool(db.pool().clone())),
                    media: None,
                    delete,
                    relay_pubkey,
                    batch_size,
                    max_items,
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

mod account_batches;
mod content_batches;
mod handlers;
mod inventory;
/// Direct upload-only media cleanup and its workflow handler.
pub mod media;
/// Standalone delegation of an owned community to community deletion.
pub mod owned_community;
mod profile_batches;
/// Read-only summaries of locally saved request progress and external waits.
pub mod summary;
mod transition;
mod worker;
mod workflow;
