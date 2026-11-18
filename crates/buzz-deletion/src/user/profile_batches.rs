//! Standalone profile command. No workflow stages are enabled here.
use anyhow::Result;
use buzz_db::user_deletion::profile_batches::ProfileBatchOptions;
use uuid::Uuid;

/// Scope and limits for offline profile cleanup.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Human public key (hex or npub); includes directly owned agents.
    #[arg(long, value_parser = parse_key)]
    pubkey: String,
    /// One explicit surviving community, including former memberships.
    #[arg(long)]
    community: Uuid,
    /// Trusted relay signer for verified profiles and archive events; repeatable.
    #[arg(long, value_parser = parse_key)]
    relay_pubkey: Vec<String>,
    /// Print proposed rows without writing anything (the default).
    #[arg(long, alias = "read-only", conflicts_with = "delete")]
    print_only: bool,
    /// Apply batches. Stop relay and external writers; finish content/account/media cleanup first.
    #[arg(long, conflicts_with = "print_only")]
    delete: bool,
    /// Maximum row mutations per transaction, 1–100.
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=100))]
    batch_size: u16,
    /// Maximum planned row mutations; exceeding the limit fails before deletion.
    #[arg(long, default_value_t = 100_000, value_parser = clap::value_parser!(u32).range(1..=1_000_000))]
    max_items: u32,
}

fn parse_key(value: &str) -> std::result::Result<String, String> {
    buzz_db::user_deletion::normalize_user_deletion_pubkey(value).map_err(|e| e.to_string())
}

pub(super) async fn run(args: Args) -> Result<i32> {
    let db = buzz_db::Db::connect_cold_start(
        buzz_db::DbConfig {
            database_url: crate::required_env("DATABASE_URL")?,
            max_connections: 1,
            min_connections: 0,
            default_transaction_read_only: !args.delete,
            ..Default::default()
        },
        "profile_batches",
    )
    .await?;
    // Never parse, require or print a private key during preview.
    let secret = if args.delete {
        std::env::var("BUZZ_RELAY_PRIVATE_KEY").ok()
    } else {
        None
    };
    db.user_deletion_store()
        .run_profile_batches(
            &ProfileBatchOptions {
                community: args.community,
                pubkey: args.pubkey,
                relay_pubkeys: args.relay_pubkey,
                batch_size: args.batch_size,
                max_items: args.max_items,
            },
            args.delete,
            secret.as_deref(),
            &mut std::io::stdout(),
        )
        .await?;
    Ok(0)
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
    fn preview_is_default_and_delete_is_explicit() {
        let base = [
            "profile-batches",
            "--pubkey",
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "--community",
            "00000000-0000-0000-0000-000000000001",
        ];
        for flag in [None, Some("--print-only"), Some("--read-only")] {
            assert!(
                !Cli::try_parse_from(base.into_iter().chain(flag))
                    .unwrap()
                    .args
                    .delete
            );
        }
        assert!(
            Cli::try_parse_from(base.into_iter().chain(["--delete"]))
                .unwrap()
                .args
                .delete
        );
        for flag in ["--print-only", "--read-only"] {
            assert!(Cli::try_parse_from(base.into_iter().chain([flag, "--delete"])).is_err());
        }
        for extra in [
            ["--batch-size", "0"],
            ["--batch-size", "101"],
            ["--max-items", "0"],
            ["--pubkey", "bad"],
            ["--agent-pubkey", base[2]],
        ] {
            assert!(Cli::try_parse_from(base.into_iter().chain(extra)).is_err());
        }
    }
}
