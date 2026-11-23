use crate::{client::BuzzClient, credentials::Credentials, error::CliError, RepoGrantArgs};
use buzz_core::governance::RepoCoordinate;
use nostr::{PublicKey, Timestamp};

pub(super) fn grant(args: RepoGrantArgs, client: &BuzzClient) -> Result<(), CliError> {
    println!(
        "{}",
        mint(&args, client.credentials(), Timestamp::now().as_secs())?
    );
    Ok(())
}

fn duration_seconds(value: &str) -> Result<u64, CliError> {
    let bad = || {
        CliError::Usage("--for must be a positive duration such as 30s, 30m, 2h, 7d or 1w".into())
    };
    if !value.is_ascii() {
        return Err(bad());
    }
    let (digits, unit) = value.split_at(value.len().saturating_sub(1));
    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        "w" => 604800,
        _ => return Err(bad()),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    digits
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(multiplier))
        .filter(|n| *n > 0)
        .ok_or_else(bad)
}

fn mint(args: &RepoGrantArgs, credentials: &Credentials, now: u64) -> Result<String, CliError> {
    let agent = PublicKey::from_hex(&args.agent).map_err(|_| {
        CliError::Usage("--agent must be a lowercase hexadecimal public key".into())
    })?;
    if agent.to_hex() != args.agent {
        return Err(CliError::Usage(
            "--agent must be a lowercase hexadecimal public key".into(),
        ));
    }
    let scope = match (&args.repo, args.all) {
        (Some(repo), false) => {
            let repo: RepoCoordinate = repo.parse().map_err(CliError::Usage)?;
            format!("{}:{}", repo.owner, repo.name)
        }
        (None, true) => "*".into(),
        _ => {
            return Err(CliError::Usage(
                "choose exactly one of --repo or --all".into(),
            ))
        }
    };
    let mut conditions = format!("repo={scope}&git={}", args.git);
    if let Some(duration) = &args.duration {
        let seconds = duration_seconds(duration)?;
        let from = now.saturating_sub(60);
        let until = from
            .checked_add(seconds)
            .filter(|t| *t <= u32::MAX as u64)
            .ok_or_else(|| CliError::Usage("--for exceeds the NIP-OA timestamp range".into()))?;
        if until <= now {
            return Err(CliError::Usage(
                "--for must exceed 60s to leave a usable lifetime after clock skew".into(),
            ));
        }
        conditions.push_str(&format!("&created_at>{from}&created_at<{until}"));
    }
    credentials.compute_git_grant(&agent, &conditions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Cmd, ReposCmd};
    use clap::Parser;
    use nostr::Keys;

    #[test]
    fn cli_grants_round_trip_through_sdk() {
        let owner = Keys::generate();
        let agent = Keys::generate().public_key();
        let credentials = Credentials::from_keys(owner.clone());
        let repo = format!("30617:{}:example", owner.public_key().to_hex());
        for (scope, value, level, duration) in [
            ("--repo", Some(repo.as_str()), "read", Some("7d")),
            ("--all", None, "push", None),
            ("--all", None, "manage", Some("2h")),
        ] {
            let agent_hex = agent.to_hex();
            let mut argv = vec![
                "buzz", "repos", "grant", "--agent", &agent_hex, "--git", level, scope,
            ];
            if let Some(value) = value {
                argv.push(value);
            }
            if let Some(duration) = duration {
                argv.extend(["--for", duration]);
            }
            let cli = Cli::try_parse_from(argv).unwrap();
            let Cmd::Repos(ReposCmd::Grant(args)) = cli.command else {
                panic!("grant command");
            };
            let json = mint(&args, &credentials, 1_800_000_000).unwrap();
            assert!(buzz_sdk::nip_oa::verify_git_grant(&json, &agent, 1_800_000_000).is_ok());
            assert!(buzz_sdk::nip_oa::verify_auth_tag(&json, &agent).is_err());
            assert!(buzz_sdk::nip_oa::verify_git_grant(
                &json,
                &Keys::generate().public_key(),
                1_800_000_000
            )
            .is_err());
            let tag: Vec<String> = serde_json::from_str(&json).unwrap();
            assert_eq!(tag[1], owner.public_key().to_hex());
            let expected_scope = value
                .map(|r| r.strip_prefix("30617:").unwrap())
                .unwrap_or("*");
            let suffix = match duration {
                Some("7d") => "&created_at>1799999940&created_at<1800604740",
                Some("2h") => "&created_at>1799999940&created_at<1800007140",
                None => "",
                _ => unreachable!(),
            };
            assert_eq!(tag[2], format!("repo={expected_scope}&git={level}{suffix}"));
        }
    }

    #[test]
    fn grant_flags_require_one_scope_and_one_level() {
        let key = Keys::generate().public_key().to_hex();
        for tail in [
            vec!["--git", "read"],
            vec!["--all"],
            vec!["--all", "--git", "admin"],
            vec!["--all", "--repo", "30617:owner:repo", "--git", "read"],
            vec!["--all", "--git", "read", "set", "--file", "grant.json"],
        ] {
            let mut argv = vec!["buzz", "repos", "grant", "--agent", &key];
            argv.extend(tail);
            assert!(Cli::try_parse_from(argv).is_err());
        }
    }

    #[test]
    fn unsupported_signer_returns_an_error() {
        let keys = Keys::generate();
        let credentials =
            Credentials::from_signer(std::sync::Arc::new(keys.clone()), keys.public_key());
        let error = credentials
            .compute_git_grant(&Keys::generate().public_key(), "repo=*&git=read")
            .unwrap_err();
        assert!(error.to_string().contains("this signer does not support"));
    }

    #[test]
    fn invalid_grants_do_not_mint() {
        let keys = Keys::generate();
        let credentials = Credentials::from_keys(keys.clone());
        let mut args = RepoGrantArgs {
            repo: None,
            all: true,
            agent: Keys::generate().public_key().to_hex(),
            git: "read".into(),
            duration: None,
        };
        for duration in [
            "",
            "0s",
            "1s",
            "60s",
            "-1h",
            "1",
            "1.5h",
            "1h2m",
            "18446744073709551615w",
            "💥",
        ] {
            args.duration = Some(duration.into());
            assert!(
                mint(&args, &credentials, 1_800_000_000).is_err(),
                "{duration}"
            );
        }
        args.duration = Some("61s".into());
        assert!(mint(&args, &credentials, u32::MAX as u64).is_err());
        args.duration = None;
        args.agent = keys.public_key().to_hex();
        assert!(mint(&args, &credentials, 1_800_000_000).is_err());
        args.agent = Keys::generate().public_key().to_hex();
        args.repo = Some(format!(
            "30617:{}:bad&git=manage",
            keys.public_key().to_hex()
        ));
        args.all = false;
        assert!(mint(&args, &credentials, 1_800_000_000).is_err());
    }
}
