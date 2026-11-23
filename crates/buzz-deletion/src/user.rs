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
    /// Save a request; read-only submissions start independent rehearsals.
    Submit {
        /// Start a fresh rehearsal with independent approval and saved progress.
        #[arg(
            long,
            alias = "rehearse",
            requires = "pubkey",
            conflicts_with = "target"
        )]
        read_only: bool,
        /// Reuse this UUID when retrying submission; otherwise generate a fresh one.
        #[arg(long, requires = "read_only")]
        run_id: Option<Uuid>,
        /// Trusted relay public key for historical content attribution in the inventory.
        #[arg(long, requires = "read_only")]
        relay_pubkey: Option<String>,
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
    /// Preview without writes by default; --read-only advances an approved rehearsal.
    Run {
        /// Approved request UUID.
        id: Uuid,
        /// Execute implemented real handlers; stop relay and external writers first.
        #[arg(long)]
        delete: bool,
        /// Persist real previews; only journal and community handoff records may change.
        #[arg(long, alias="rehearse", conflicts_with_all=["delete", "print_only", "stage", "community"])]
        read_only: bool,
        /// Print proposals without saving progress or submitting community children.
        #[arg(long, conflicts_with = "delete")]
        print_only: bool,
        /// Inspect one handler without advancing; implies stateless preview.
        #[arg(long, value_parser=workflow::parse_stage, conflicts_with="delete")]
        stage: Option<String>,
        /// Inspect one community from the saved inventory.
        #[arg(long, requires = "stage", conflicts_with = "delete")]
        community: Option<Uuid>,
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
    /// List requests, newest first.
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
        &command,
        Command::Inventory { .. }
            | Command::AccountScope { .. }
            | Command::Run {
                print_only: true,
                ..
            }
            | Command::Run { stage: Some(_), .. }
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
            read_only,
            run_id,
            relay_pubkey,
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
            let request = if read_only {
                let run_id = run_id.unwrap_or_else(Uuid::new_v4);
                // Print before the first write so a lost response can be retried.
                eprintln!("rehearsal run ID: {run_id}");
                users
                    .submit_rehearsal_with_relay(
                        &target,
                        &requested_by,
                        reason.as_deref(),
                        run_id,
                        relay_pubkey.as_deref(),
                    )
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
            id,
            delete,
            read_only,
            print_only,
            stage,
            community,
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
            let relay_pubkey = inventory::rehearsal_relay_key(
                request.inventory_manifest.as_ref(),
                relay_pubkey.as_deref(),
            )?;
            let real = !request.target.starts_with("demo:");
            if real {
                anyhow::ensure!(
                    fail_stage.is_none() && stage_delay_ms == 0 && demo_no_agents_in.is_empty(),
                    "dummy controls cannot be used for real deletion"
                );
            }
            let source = if !delete {
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
            let options = worker::RunOptions {
                db: real.then_some(source),
                media: None,
                delete,
                read_only,
                community_store: read_only.then(|| db.deletion_store()),
                relay_pubkey,
                batch_size,
                max_items,
                next_step,
                fail_stage,
                stage_delay: std::time::Duration::from_millis(stage_delay_ms),
                no_agents_in: demo_no_agents_in,
            };
            if (real && !delete && !read_only) || print_only || stage.is_some() {
                return rehearsal_preview::run(
                    &db,
                    &request,
                    &options,
                    stage.as_deref(),
                    community,
                )
                .await;
            }
            let code = worker::run(
                &store,
                id,
                &executor_id.unwrap_or_else(crate::default_executor_id),
                &options,
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
mod rehearsal;
mod rehearsal_preview;
/// Read-only summaries of locally saved request progress and external waits.
pub mod summary;
mod transition;
mod worker;
mod workflow;

#[cfg(test)]
mod rehearsal_cli_tests {
    use super::*;
    use clap::Parser;
    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        command: Command,
    }
    #[test]
    fn rehearsal_submission_and_execution_modes_are_explicit() {
        let key = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        let id = "00000000-0000-0000-0000-000000000001";
        let submit = [
            "user-deletions",
            "submit",
            "--pubkey",
            key,
            "--requested-by",
            "operator",
        ];
        assert!(
            Cli::try_parse_from(submit.into_iter().chain(["--read-only", "--run-id", id])).is_ok()
        );
        assert!(Cli::try_parse_from(submit.into_iter().chain(["--run-id", id])).is_err());
        assert!(Cli::try_parse_from(submit.into_iter().chain([
            "--read-only",
            "--relay-pubkey",
            key
        ]))
        .is_ok());
        assert!(Cli::try_parse_from(submit.into_iter().chain(["--relay-pubkey", key])).is_err());
        let run = ["user-deletions", "run", id];
        for mode in ["--read-only", "--print-only", "--delete"] {
            assert!(Cli::try_parse_from(run.into_iter().chain([mode, "--next-step"])).is_ok());
        }
        for modes in [
            ["--read-only", "--delete"],
            ["--read-only", "--print-only"],
            ["--delete", "--print-only"],
        ] {
            assert!(Cli::try_parse_from(run.into_iter().chain(modes)).is_err());
        }
        assert!(Cli::try_parse_from(run.into_iter().chain([
            "--read-only",
            "--stage",
            "purge_profile"
        ]))
        .is_err());
        assert!(Cli::try_parse_from(run.into_iter().chain([
            "--print-only",
            "--stage",
            "purge_profile",
            "--community",
            id
        ]))
        .is_ok());
    }
}
