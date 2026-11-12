//! User inventory, offline account batches, and synthetic deletion workflows.

use anyhow::Result;
use clap::Subcommand;
use uuid::Uuid;

/// Inspect user data, execute offline account batches, or run synthetic workflows.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print human-content cleanup batches; optionally execute them.
    PreviewBatches {
        /// Human public key, as npub or 64-character hex.
        #[arg(long, value_parser = content_batches::parse_pubkey)]
        pubkey: String,
        /// Explicit community UUID; includes historical content after leaving.
        #[arg(long)]
        community: Uuid,
        /// Print batches without changing data (default when no mode is supplied).
        #[arg(long, alias = "read-only", conflicts_with = "delete")]
        print_only: bool,
        /// Print and commit each batch. Stop relay/workers before using this mode.
        #[arg(long)]
        delete: bool,
        /// Trusted relay public key for historical actor/p attribution, if needed.
        #[arg(long, value_parser = content_batches::parse_pubkey)]
        relay_pubkey: Option<String>,
        /// Maximum row operations per printed batch.
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=100))]
        batch_size: u16,
        /// Fail visibly if the preview would exceed this many unique row operations.
        #[arg(long, default_value_t = 1_000_000, value_parser = clap::value_parser!(u32).range(1..=1_000_000))]
        max_items: u32,
    },
    /// Direct media discovery and inventory; print-only stops after step 2.
    Media(media::Options),
    /// Print account-table batches for one community; --delete executes offline.
    AccountBatches(account_batches::Args),
    /// Print real current memberships as JSON; does not save or execute a request.
    Inventory {
        /// Demo fixture (default), hex public key, or npub. Current memberships only.
        #[arg(long, default_value = "demo", value_parser = inventory::parse_subject)]
        pubkey: String,
    },
    /// Save a request. An identical resubmission returns the existing row.
    Submit {
        /// Save an independent read-only workflow request for --pubkey.
        #[arg(
            long = "read-only",
            alias = "rehearse",
            requires = "pubkey",
            conflicts_with = "target"
        )]
        rehearse: bool,
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
    /// Preview by default; --read-only or --delete runs the persisted worker.
    Run {
        /// Approved request UUID.
        id: Uuid,
        /// Preview or explicit offline execution controls.
        #[command(flatten)]
        execution: real::Args,
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
    if let Command::PreviewBatches {
        pubkey,
        community,
        print_only,
        delete,
        relay_pubkey,
        batch_size,
        max_items,
    } = command
    {
        anyhow::ensure!(
            !(print_only && delete),
            "--print-only conflicts with --delete"
        );
        return content_batches::run(
            buzz_db::user_deletion::content_batches::ContentBatchOptions {
                community,
                pubkey,
                relay_pubkey,
                batch_size,
                max_items,
            },
            delete,
        )
        .await;
    }
    let command = match command {
        Command::Media(options) => return media::run(options).await,
        other => other,
    };
    if let Command::AccountBatches(args) = command {
        return account_batches::run(args).await;
    }
    let db = if matches!(
        &command,
        Command::Inventory { .. }
            | Command::Run {
                execution: real::Args {
                    delete: false,
                    rehearse: false,
                    ..
                },
                ..
            }
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
        Command::PreviewBatches { .. } => {
            anyhow::bail!("batch command must use its standalone connection")
        }
        Command::Media(_) => anyhow::bail!("media command was already dispatched"),
        Command::AccountBatches(_) => {
            anyhow::bail!("account command must use its isolated connection")
        }
        Command::Inventory { pubkey } => {
            crate::print_json(&inventory::generate(&db, &pubkey).await?)?;
        }
        Command::Submit {
            rehearse,
            target,
            pubkey,
            requested_by,
            reason,
        } => {
            let target = target.unwrap_or_else(|| match pubkey.as_deref() {
                None | Some("demo") => "demo:default".to_owned(),
                Some(key) => key.to_owned(),
            });
            let users = db.user_deletion_store();
            let request = if rehearse {
                users
                    .submit_rehearsal(&target, &requested_by, reason.as_deref())
                    .await?
            } else {
                users
                    .submit(&target, &requested_by, reason.as_deref())
                    .await?
            };
            crate::print_json(&request)?;
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
            execution,
            id,
            executor_id,
            next_step,
            fail_stage,
            stage_delay_ms,
            demo_no_agents_in,
        } => {
            let request = store.get(id).await?;
            if !execution.delete && !execution.rehearse {
                return real::preview(&db, &request, &execution, next_step).await;
            }
            let is_real = !request.target.starts_with("demo:");
            if is_real
                && (fail_stage.is_some() || stage_delay_ms != 0 || !demo_no_agents_in.is_empty())
            {
                anyhow::bail!("dummy controls cannot be used for real deletion");
            }
            execution.validate_execution(&request)?;
            let source = if execution.rehearse {
                // Workflow progress uses the writer pool; handler reads cannot write
                // user data even if a preview accidentally acquires a write path.
                buzz_db::Db::connect_cold_start(
                    buzz_db::DbConfig {
                        database_url: crate::required_env("DATABASE_URL")?,
                        max_connections: 1,
                        min_connections: 0,
                        default_transaction_read_only: true,
                        ..buzz_db::DbConfig::default()
                    },
                    "user_rehearsal_source",
                )
                .await?
            } else {
                buzz_db::Db::from_pool(db.pool().clone())
            };
            let code = worker::run(
                &store,
                id,
                &executor_id.unwrap_or_else(crate::default_executor_id),
                &worker::RunOptions {
                    next_step,
                    real: is_real.then(|| real::Execution {
                        db: source,
                        args: execution,
                    }),
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
/// Standalone delegation of an owned community to community deletion.
pub mod owned_community;
mod real;
/// Read-only summaries of locally saved request progress and external waits.
pub mod summary;
mod transition;
mod worker;
mod workflow;

/// Media discovery, classification and protected execution.
pub mod media;
