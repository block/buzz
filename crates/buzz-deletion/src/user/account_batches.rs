//! One account cleanup handler; the mode chooses printing or deletion.
use anyhow::Result;
use buzz_db::entity_deletion::EntityLease;
use buzz_db::user_deletion::{
    account_batches::{
        AccountBatchOptions, AccountBatchSummary, AccountScope, GlobalAccountBatchOptions,
    },
    ExecutionMode,
};
use uuid::Uuid;

/// Arguments for the account-table debugging command.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Human public key, in hex or npub form.
    #[arg(long, value_parser = parse_pubkey, required_unless_present = "scope_file", conflicts_with = "scope_file")]
    pub(super) pubkey: Option<String>,
    /// One surviving community selected by the caller.
    #[arg(long, required_unless_present_any = ["global", "scope_file"], conflicts_with_all = ["global", "scope_file"])]
    pub(super) community: Option<Uuid>,
    /// Explicitly clean deployment-wide listeners, feedback and DB roles.
    #[arg(long, group = "wide_scope", requires = "actor", conflicts_with_all = ["community", "scope_file"])]
    pub(super) global: bool,
    /// Audit identity performing global role revocation (hex or npub).
    #[arg(long, value_parser = parse_pubkey, requires = "wide_scope")]
    pub(super) actor: Option<String>,
    /// Output of account-scope selecting communities; agents are fetched on every run.
    #[arg(long, group = "wide_scope", requires = "actor", conflicts_with_all = ["community", "global", "pubkey"])]
    pub(super) scope_file: Option<std::path::PathBuf>,
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
    let store = store(db, mode)?;
    // Workflow stdout is a single inspection document; direct commands emit NDJSON.
    if stage.is_some() {
        Ok(store
            .run_account_batches(options, mode, &mut std::io::stderr(), stage)
            .await?)
    } else {
        Ok(store
            .run_account_batches(options, mode, &mut std::io::stdout(), stage)
            .await?)
    }
}

/// Run the explicitly selected scope; global work is never inferred from a
/// community or from the absence of community memberships.
pub(super) async fn run_command(
    db: &buzz_db::Db,
    args: Args,
    mode: ExecutionMode,
) -> Result<AccountBatchSummary> {
    if let Some(path) = &args.scope_file {
        // Bound input before parsing; a bad file must fail before any database mutation.
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() <= 4 * 1024 * 1024,
            "account scope exceeds 4 MiB"
        );
        let scope: AccountScope = serde_json::from_slice(&bytes)?;
        scope.validate()?;
        if mode == ExecutionMode::Delete && configured_operator(&scope.target)? {
            anyhow::bail!("target is a config-backed operator; remove that authority from every relay configuration before offline cleanup");
        }
        return Ok(store(db, mode)?
            .run_account_scope(
                &scope,
                args.actor
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("scope cleanup requires --actor"))?,
                args.batch_size,
                args.max_items,
                mode,
                &mut std::io::stdout(),
            )
            .await?);
    }
    let pubkey = args
        .pubkey
        .ok_or_else(|| anyhow::anyhow!("missing --pubkey"))?;
    if args.global {
        if mode == ExecutionMode::Delete && configured_operator(&pubkey)? {
            anyhow::bail!("target is a config-backed operator; remove that authority from every relay configuration before offline cleanup");
        }
        Ok(store(db, mode)?
            .run_global_account_batches(
                &GlobalAccountBatchOptions {
                    pubkey,
                    actor: args
                        .actor
                        .ok_or_else(|| anyhow::anyhow!("global cleanup requires --actor"))?,
                    batch_size: args.batch_size,
                    max_items: args.max_items,
                },
                mode,
                &mut std::io::stdout(),
            )
            .await?)
    } else {
        run(
            db,
            &AccountBatchOptions {
                community: args
                    .community
                    .ok_or_else(|| anyhow::anyhow!("missing --community"))?,
                pubkey,
                batch_size: args.batch_size,
                max_items: args.max_items,
            },
            mode,
            None,
        )
        .await
    }
}

// Preview never needs secret material and never invokes the signing path.
fn store(
    db: &buzz_db::Db,
    mode: ExecutionMode,
) -> Result<buzz_db::user_deletion::UserDeletionStore> {
    let store = db.user_deletion_store();
    if mode == ExecutionMode::ReadOnly {
        return Ok(store);
    }
    match std::env::var("BUZZ_RELAY_PRIVATE_KEY") {
        Ok(value) => Ok(store.with_account_snapshot_signer(
            nostr::Keys::parse(&value)
                .map_err(|_| anyhow::anyhow!("invalid BUZZ_RELAY_PRIVATE_KEY"))?,
        )),
        Err(std::env::VarError::NotPresent) => Ok(store),
        Err(_) => anyhow::bail!("invalid BUZZ_RELAY_PRIVATE_KEY"),
    }
}

// This catches local configuration; an offline CLI cannot attest every relay's
// deployed config. Do not treat absence here as proof of authority revocation.
fn configured_operator(target: &str) -> Result<bool> {
    let list = std::env::var("RELAY_OPERATOR_PUBKEYS").unwrap_or_default();
    let owner = std::env::var("RELAY_OWNER_PUBKEY").unwrap_or_default();
    configured_operator_in(target, &list, &owner)
}

fn configured_operator_in(target: &str, list: &str, owner: &str) -> Result<bool> {
    let mut keys = Vec::new();
    for value in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        keys.push(parse_pubkey(value).map_err(anyhow::Error::msg)?);
    }
    if keys.is_empty() && !owner.trim().is_empty() {
        keys.push(parse_pubkey(owner.trim()).map_err(anyhow::Error::msg)?);
    }
    Ok(keys.iter().any(|key| key == target))
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
    fn global_scope_requires_explicit_actor_and_config_uses_owner_fallback() {
        let target = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        assert!(Cli::try_parse_from(["account-batches", "--pubkey", target, "--global"]).is_err());
        assert!(Cli::try_parse_from([
            "account-batches",
            "--pubkey",
            target,
            "--global",
            "--actor",
            target
        ])
        .is_ok());
        assert!(Cli::try_parse_from([
            "account-batches",
            "--scope-file",
            "scope.json",
            "--actor",
            target
        ])
        .is_ok());
        assert!(Cli::try_parse_from([
            "account-batches",
            "--scope-file",
            "scope.json",
            "--pubkey",
            target,
            "--actor",
            target
        ])
        .is_err());
        assert!(configured_operator_in(target, "", target).unwrap());
        assert!(configured_operator_in(target, &target.to_uppercase(), "").unwrap());
        assert!(!configured_operator_in(
            target,
            "2222222222222222222222222222222222222222222222222222222222222222",
            target
        )
        .unwrap());
        assert!(configured_operator_in(target, "bad", "").is_err());
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
