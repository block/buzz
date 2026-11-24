//! User inventory, account cleanup and persisted deletion workflows.

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
    /// Preview or delete profiles in one community.
    ProfileBatches(profile_batches::Args),
    /// Print account-table batches for one community or apply them with --delete.
    AccountBatches(account_batches::Args),
    /// Capture frozen human/agent inputs across stored communities, read-only.
    AccountScope {
        /// Human public key, hex or npub.
        #[arg(long, value_parser = content_batches::parse_pubkey)]
        pubkey: String,
    },
    /// Print affected communities as JSON; does not save or execute a request.
    Inventory {
        /// Human public key, as hex or npub.
        #[arg(long, value_parser = content_batches::parse_pubkey)]
        pubkey: String,
    },
    /// Save a request. An identical resubmission returns the existing row.
    Submit {
        /// Keep read-only progress separate from deletion of this subject.
        #[arg(long, requires = "pubkey")]
        read_only: bool,
        /// Human public key, as hex or npub.
        #[arg(long, value_parser = content_batches::parse_pubkey)]
        pubkey: String,
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
        /// Approving operator identity (self-asserted).
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
        /// Execute implemented handlers. Never stop or restart a relay. Use isolated test data while account gating is unimplemented.
        #[arg(long)]
        delete: bool,
        /// Run the same workflow and record progress without applying source deletions.
        #[arg(long, conflicts_with = "delete")]
        read_only: bool,
        /// Write selected row keys and media identifiers as JSON lines to a new file.
        #[arg(long, value_name = "PATH")]
        write_row_ids: Option<std::path::PathBuf>,
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
    },
    /// List saved requests, newest first.
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
            crate::print_json(&db.user_deletion_store().generate_inventory(&pubkey).await?)?;
        }
        Command::Submit {
            read_only,
            pubkey,
            requested_by,
            reason,
        } => {
            crate::print_json(
                &db.user_deletion_store()
                    .submit_for_mode(&pubkey, &requested_by, reason.as_deref(), read_only)
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
            read_only,
            write_row_ids,
            relay_pubkey,
            batch_size,
            max_items,
            executor_id,
            next_step,
        } => {
            anyhow::ensure!(
                delete || read_only,
                "workflow execution requires --delete or --read-only"
            );
            let request = store.get(id).await?;
            // Earlier submissions may already identify the signer of historical records.
            let saved_signer = request
                .inventory_manifest
                .as_ref()
                .and_then(|manifest| manifest.get("discovery_relay_pubkey"))
                .and_then(|key| key.as_str());
            let relay_pubkey = match relay_pubkey.or_else(|| saved_signer.map(str::to_owned)) {
                Some(key) => Some(key),
                None => {
                    account_batches::configured_signer()?.map(|keys| keys.public_key().to_hex())
                }
            };
            let code = worker::run(
                &store,
                id,
                &executor_id.unwrap_or_else(crate::default_executor_id),
                &worker::RunOptions {
                    db: buzz_db::Db::from_pool(db.pool().clone()),
                    media: None,
                    row_ids: write_row_ids.as_deref().map(row_ids::create).transpose()?,
                    delete,
                    relay_pubkey,
                    batch_size,
                    max_items,
                    next_step,
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
/// Direct upload-only media cleanup and its workflow handler.
pub mod media;
/// Standalone delegation of an owned community to community deletion.
pub mod owned_community;
mod profile_batches;
mod row_ids;
/// Read-only summaries of locally saved request progress and external waits.
pub mod summary;
mod transition;
mod worker;
mod workflow;
