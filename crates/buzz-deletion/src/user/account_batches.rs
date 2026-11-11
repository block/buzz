//! Standalone offline account cleanup; never enters the workflow dispatcher.
use anyhow::Result;
use buzz_db::user_deletion::account_batches::AccountBatchOptions;
use uuid::Uuid;

/// Arguments for the account-table debugging command.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Human public key, in hex or npub form.
    #[arg(long, value_parser = parse_pubkey)]
    pubkey: String,
    /// One surviving community selected by the caller.
    #[arg(long)]
    community: Uuid,
    /// Print proposals only (default); does not change data, progress, or caches.
    #[arg(long, alias = "read-only", conflicts_with = "delete")]
    print_only: bool,
    /// Execute printed batches. Stop the relay and external workers first.
    #[arg(long, conflicts_with = "print_only")]
    delete: bool,
    /// Maximum explicit row operations per transaction.
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=100))]
    batch_size: u16,
    /// Stop after this many row operations; rerun delete to continue.
    #[arg(long, default_value_t = 100_000, value_parser = clap::value_parser!(u32).range(1..=1_000_000))]
    max_items: u32,
}

fn parse_pubkey(value: &str) -> std::result::Result<String, String> {
    buzz_db::user_deletion::normalize_user_deletion_pubkey(value).map_err(|error| error.to_string())
}

pub(super) async fn run(args: Args) -> Result<i32> {
    // Neither path runs startup migrations or connects to Redis. Preview uses a
    // read-only connection as well as an explicit read-only transaction.
    let db = buzz_db::Db::connect_cold_start(
        buzz_db::DbConfig {
            database_url: crate::required_env("DATABASE_URL")?,
            max_connections: 1,
            min_connections: 0,
            default_transaction_read_only: !args.delete,
            ..buzz_db::DbConfig::default()
        },
        "user_account_batches",
    )
    .await?;
    let options = AccountBatchOptions {
        community: args.community,
        pubkey: args.pubkey,
        batch_size: args.batch_size,
        max_items: args.max_items,
    };
    let store = db.user_deletion_store();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    let summary = if args.delete {
        store.delete_account_batches(&options, &mut writer).await?
    } else {
        store.preview_account_batches(&options, &mut writer).await?
    };
    Ok(if summary.supported_scope_exhausted {
        0
    } else {
        2
    })
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
