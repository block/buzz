//! One account cleanup handler; the mode chooses printing or deletion.
use anyhow::Result;
use buzz_db::entity_deletion::EntityLease;
use buzz_db::user_deletion::{
    account_batches::{AccountBatchOptions, AccountBatchSummary},
    ExecutionMode,
};
use uuid::Uuid;

/// Arguments for the account-table debugging command.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Human public key, in hex or npub form.
    #[arg(long, value_parser = parse_pubkey)]
    pub(super) pubkey: String,
    /// One surviving community selected by the caller.
    #[arg(long)]
    pub(super) community: Uuid,
    /// Print proposals only (default); does not change data, progress, or caches.
    #[arg(long, alias = "read-only", conflicts_with = "delete")]
    print_only: bool,
    /// Execute printed batches. Stop the relay and external workers first.
    #[arg(long, conflicts_with = "print_only")]
    pub(super) delete: bool,
    /// Maximum explicit row operations per transaction.
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub(super) batch_size: u16,
    /// Stop after this many row operations; rerun delete to continue.
    #[arg(long, default_value_t = 100_000, value_parser = clap::value_parser!(u32).range(1..=1_000_000))]
    pub(super) max_items: u32,
}

fn parse_pubkey(value: &str) -> std::result::Result<String, String> {
    buzz_db::user_deletion::normalize_user_deletion_pubkey(value).map_err(|error| error.to_string())
}

pub(super) async fn run(
    db: &buzz_db::Db,
    options: &AccountBatchOptions,
    mode: ExecutionMode,
    stage: Option<(&EntityLease, Uuid)>,
) -> Result<AccountBatchSummary> {
    Ok(db
        .user_deletion_store()
        .run_account_batches(options, mode, &mut std::io::stdout(), stage)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        args: Args,
    }

    #[test]
    fn modes_default_to_print_and_conflicts_are_errors() {
        let base = [
            "account-batches",
            "--pubkey",
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "--community",
            "00000000-0000-0000-0000-000000000001",
        ];
        for mode in [None, Some("--print-only"), Some("--read-only")] {
            let mut args = base.to_vec();
            args.extend(mode);
            assert!(!Cli::try_parse_from(args).unwrap().args.delete);
        }
        assert!(
            Cli::try_parse_from(base.into_iter().chain(["--delete"]))
                .unwrap()
                .args
                .delete
        );
        for preview in ["--print-only", "--read-only"] {
            assert!(Cli::try_parse_from(base.into_iter().chain([preview, "--delete"])).is_err());
        }
        for extra in [
            ["--batch-size", "0"],
            ["--batch-size", "101"],
            ["--max-items", "0"],
            ["--pubkey", "bad"],
        ] {
            assert!(Cli::try_parse_from(base.into_iter().chain(extra)).is_err());
        }
    }
}
