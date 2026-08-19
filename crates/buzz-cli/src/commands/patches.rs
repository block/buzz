use crate::client::BuzzClient;
use crate::commands::with_git_provenance;
use crate::error::CliError;
use crate::validate::{
    normalize_pubkey, read_file_or_stdin, read_or_stdin, reject_secret_key_input, sdk_err,
    validate_hex64, validate_repo_id,
};
use buzz_sdk::{GitAppliedPatchRef, GitPatchMeta, GitRepoCoord, GitStatus, GitStatusMeta};

#[allow(clippy::too_many_arguments)]
pub async fn cmd_send_patch(
    client: &BuzzClient,
    repo_owner: &str,
    repo_id: &str,
    patch: &str,
    euc: Option<&str>,
    to: &[String],
    reply_to: Option<&str>,
    root: bool,
    root_revision: bool,
    commit: Option<&str>,
    parent_commit: Option<&str>,
    commit_pgp_sig: Option<&str>,
    committer: Option<&str>,
) -> Result<(), CliError> {
    let repo_owner = normalize_pubkey(repo_owner)?;
    validate_repo_id(repo_id)?;
    let content = read_file_or_stdin(patch)?;
    let recipients = to
        .iter()
        .map(|pubkey| normalize_pubkey(pubkey))
        .collect::<Result<Vec<_>, _>>()?;

    let committer = match committer {
        Some(spec) => Some(parse_committer(spec)?),
        None => None,
    };

    let meta = GitPatchMeta {
        euc: euc.map(str::to_string),
        recipients,
        reply_to: reply_to.map(str::to_string),
        root,
        root_revision,
        commit: commit.map(str::to_string),
        parent_commit: parent_commit.map(str::to_string),
        commit_pgp_sig: commit_pgp_sig.map(str::to_string),
        committer,
    };

    let repo = GitRepoCoord {
        owner: repo_owner,
        id: repo_id.to_string(),
    };

    let builder =
        with_git_provenance(buzz_sdk::build_git_patch(&repo, &content, &meta).map_err(sdk_err)?)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

/// Parse `--committer 'name|email|timestamp|tz-offset-minutes'`.
fn parse_committer(spec: &str) -> Result<(String, String, String, String), CliError> {
    let parts: Vec<&str> = spec.split('|').collect();
    match parts.as_slice() {
        [name, email, ts, tz] => Ok((
            name.to_string(),
            email.to_string(),
            ts.to_string(),
            tz.to_string(),
        )),
        _ => Err(CliError::Usage(
            "--committer must be 'name|email|timestamp|tz-offset-minutes'".into(),
        )),
    }
}

pub async fn cmd_get_patch(client: &BuzzClient, event: &str) -> Result<(), CliError> {
    validate_hex64(event)?;
    let filter = serde_json::json!({
        "kinds": [1617],
        "ids": [event]
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_list_patches(
    client: &BuzzClient,
    repo_owner: &str,
    repo_id: &str,
    author: Option<&str>,
    limit: Option<u32>,
) -> Result<(), CliError> {
    let repo_owner = normalize_pubkey(repo_owner)?;
    validate_repo_id(repo_id)?;

    let a_value = format!("30617:{repo_owner}:{repo_id}");
    let mut filter = serde_json::json!({
        "kinds": [1617],
        "#a": [a_value]
    });

    if let Some(pk) = author {
        let pk = normalize_pubkey(pk)?;
        filter["authors"] = serde_json::json!([pk]);
    }
    if let Some(n) = limit {
        filter["limit"] = serde_json::json!(n);
    }

    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_patch_status(
    client: &BuzzClient,
    root: &str,
    status: &str,
    content: Option<&str>,
    repo_owner: Option<&str>,
    repo_id: Option<&str>,
    euc: Option<&str>,
    revision: Option<&str>,
    to: &[String],
    q: &[String],
    merge_commit: Option<&str>,
    applied_as_commit: &[String],
) -> Result<(), CliError> {
    validate_hex64(root)?;
    let status = parse_status(status)?;
    let body = match content {
        Some(c) => read_or_stdin(c)?,
        None => String::new(),
    };

    let repo = match (repo_owner, repo_id) {
        (Some(owner), Some(id)) => {
            let owner = normalize_pubkey(owner)?;
            validate_repo_id(id)?;
            Some(GitRepoCoord {
                owner,
                id: id.to_string(),
            })
        }
        (None, None) => None,
        _ => {
            return Err(CliError::Usage(
                "--repo-owner and --repo-id must be given together".into(),
            ))
        }
    };

    // NIP-34 expects status events to `p`-tag the repo owner (plus root/
    // revision authors) so they're discoverable by subscription. Default
    // to the repo owner when known; `--to` covers root-author / revision-
    // author / anyone else the caller wants to notify.
    let mut recipients = Vec::new();
    if let Some(ref repo) = repo {
        recipients.push(repo.owner.clone());
    }
    for recipient in to {
        let recipient = normalize_pubkey(recipient)?;
        if !recipients.contains(&recipient) {
            recipients.push(recipient);
        }
    }

    let applied_patches = q
        .iter()
        .map(|spec| parse_applied_patch_ref(spec))
        .collect::<Result<Vec<_>, _>>()?;

    let meta = GitStatusMeta {
        root_event: root.to_string(),
        accepted_revision_root: revision.map(str::to_string),
        repo,
        euc: euc.map(str::to_string),
        recipients,
        applied_patches,
        merge_commit: merge_commit.map(str::to_string),
        applied_as_commits: applied_as_commit.to_vec(),
    };

    let builder =
        with_git_provenance(buzz_sdk::build_git_status(status, &body, &meta).map_err(sdk_err)?)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

fn parse_applied_patch_ref(spec: &str) -> Result<GitAppliedPatchRef, CliError> {
    if let Some((_, trailing_hint)) = spec.rsplit_once(':') {
        reject_secret_key_input(trailing_hint)?;
    }
    let parsed = GitAppliedPatchRef::parse(spec).map_err(sdk_err)?;
    if parsed.pubkey.is_some() {
        return Ok(parsed);
    }

    let Some((prefix, candidate)) = spec.rsplit_once(':') else {
        return Ok(parsed);
    };
    let Some((_, relay_hint)) = prefix.split_once(':') else {
        return Ok(parsed);
    };
    if relay_hint.is_empty() || !candidate.starts_with("npub1") {
        return Ok(parsed);
    }

    let author_hex = normalize_pubkey(candidate)?;
    GitAppliedPatchRef::parse(&format!("{prefix}:{author_hex}")).map_err(sdk_err)
}

/// Parse the CLI's status word into a `GitStatus`. `merged` and `resolved`
/// are accepted as synonyms for the same underlying kind (1631) — NIP-34
/// uses "applied/merged" for patches and "resolved" for issues, but it's one
/// status kind either way. Shared by `buzz issues status`.
pub(crate) fn parse_status(s: &str) -> Result<GitStatus, CliError> {
    match s {
        "open" => Ok(GitStatus::Open),
        "merged" | "resolved" => Ok(GitStatus::AppliedOrResolved),
        "closed" => Ok(GitStatus::Closed),
        "draft" => Ok(GitStatus::Draft),
        other => Err(CliError::Usage(format!(
            "invalid status '{other}' — expected one of: open, merged, resolved, closed, draft"
        ))),
    }
}

pub async fn dispatch(cmd: crate::PatchesCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::PatchesCmd;
    match cmd {
        PatchesCmd::Send {
            repo_owner,
            repo_id,
            patch_file,
            euc,
            to,
            reply_to,
            root,
            root_revision,
            commit,
            parent_commit,
            commit_pgp_sig,
            committer,
        } => {
            cmd_send_patch(
                client,
                &repo_owner,
                &repo_id,
                &patch_file,
                euc.as_deref(),
                &to,
                reply_to.as_deref(),
                root,
                root_revision,
                commit.as_deref(),
                parent_commit.as_deref(),
                commit_pgp_sig.as_deref(),
                committer.as_deref(),
            )
            .await
        }
        PatchesCmd::Get { event } => cmd_get_patch(client, &event).await,
        PatchesCmd::List {
            repo_owner,
            repo_id,
            author,
            limit,
        } => cmd_list_patches(client, &repo_owner, &repo_id, author.as_deref(), limit).await,
        PatchesCmd::Status {
            root,
            status,
            content,
            repo_owner,
            repo_id,
            euc,
            revision,
            to,
            q,
            merge_commit,
            applied_as_commit,
        } => {
            cmd_patch_status(
                client,
                &root,
                &status,
                content.as_deref(),
                repo_owner.as_deref(),
                repo_id.as_deref(),
                euc.as_deref(),
                revision.as_deref(),
                &to,
                &q,
                merge_commit.as_deref(),
                &applied_as_commit,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_committer_valid() {
        let result = parse_committer("Jane Doe|jane@example.com|1700000000|-480").unwrap();
        assert_eq!(
            result,
            (
                "Jane Doe".to_string(),
                "jane@example.com".to_string(),
                "1700000000".to_string(),
                "-480".to_string()
            )
        );
    }

    #[test]
    fn parse_committer_rejects_wrong_field_count() {
        assert!(parse_committer("Jane Doe|jane@example.com").is_err());
        assert!(parse_committer("a|b|c|d|e").is_err());
    }

    #[test]
    fn parse_status_accepts_known_words() {
        assert!(matches!(parse_status("open").unwrap(), GitStatus::Open));
        assert!(matches!(
            parse_status("merged").unwrap(),
            GitStatus::AppliedOrResolved
        ));
        assert!(matches!(
            parse_status("resolved").unwrap(),
            GitStatus::AppliedOrResolved
        ));
        assert!(matches!(parse_status("closed").unwrap(), GitStatus::Closed));
        assert!(matches!(parse_status("draft").unwrap(), GitStatus::Draft));
    }

    #[test]
    fn parse_status_rejects_unknown_word() {
        let err = parse_status("merge").unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn applied_patch_ref_normalizes_npub_author_hint_to_protocol_hex() {
        let keys = nostr::Keys::generate();
        let hex = keys.public_key().to_hex();
        let npub = crate::validate::format_npub(&hex).expect("public key formats");
        let event_id = "a".repeat(64);

        let parsed =
            parse_applied_patch_ref(&format!("{event_id}:wss://relay.example/path:{npub}"))
                .expect("npub hint parses");

        assert_eq!(parsed.id, event_id);
        assert_eq!(parsed.relay.as_deref(), Some("wss://relay.example/path"));
        assert_eq!(parsed.pubkey.as_deref(), Some(hex.as_str()));
    }

    #[test]
    fn applied_patch_ref_keeps_legacy_hex_author_hint_compatible() {
        let keys = nostr::Keys::generate();
        let hex = keys.public_key().to_hex();
        let event_id = "b".repeat(64);

        let parsed = parse_applied_patch_ref(&format!("{event_id}:wss://relay.example/path:{hex}"))
            .expect("legacy hex hint parses");

        assert_eq!(parsed.pubkey.as_deref(), Some(hex.as_str()));
    }

    #[test]
    fn applied_patch_ref_rejects_secret_shaped_author_hint_without_echo() {
        let event_id = "c".repeat(64);
        for secret in [
            "nsec1secret-shaped",
            "nostr:nsec1secret-shaped",
            "NSEC1SECRET-SHAPED",
            "NOSTR:NSEC1SECRET-SHAPED",
        ] {
            let supplied = format!("{event_id}:wss://relay.example/path:{secret}");
            let error = parse_applied_patch_ref(&supplied)
                .expect_err("secret author hint must be rejected");
            assert!(!error.to_string().contains(secret));
            assert!(!error.to_string().contains(&supplied));
        }
    }
}
