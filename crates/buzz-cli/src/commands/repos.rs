use buzz_core::{
    git_perms::{parse_protection_tag, parse_protection_tags, RefPattern},
    kind::KIND_GIT_REPO_ANNOUNCEMENT,
};
use nostr::{Event, EventBuilder, Tag, Timestamp};

use crate::client::BuzzClient;
use crate::commands::parse_write_response;
use crate::error::CliError;
use crate::validate::validate_repo_id;

fn parse_events(json: &str) -> Result<Vec<Event>, CliError> {
    serde_json::from_str(json)
        .map_err(|error| CliError::Other(format!("failed to parse relay response: {error}")))
}

pub(crate) async fn fetch_own_repo_announcement(
    client: &BuzzClient,
    repo_id: &str,
) -> Result<Option<Event>, CliError> {
    let filter = serde_json::json!({
        "kinds": [KIND_GIT_REPO_ANNOUNCEMENT],
        "authors": [client.keys().public_key().to_hex()],
        "#d": [repo_id],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let mut events = parse_events(&raw)?;
    events.sort_by_key(|event| std::cmp::Reverse(event.created_at));
    Ok(events.into_iter().next())
}

fn repo_id_from_event(event: &Event) -> Result<&str, CliError> {
    event
        .tags
        .iter()
        .find_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("d"))
                .then(|| values.get(1).map(String::as_str))
                .flatten()
        })
        .ok_or_else(|| CliError::Other("repository announcement is missing its d tag".into()))
}

fn tag_error(error: impl std::fmt::Display) -> CliError {
    CliError::Other(format!("failed to build protection tag: {error}"))
}

fn protection_pattern(tag: &Tag) -> Option<&str> {
    let values = tag.as_slice();
    (values.first().map(String::as_str) == Some("buzz-protect"))
        .then(|| values.get(1).map(String::as_str))
        .flatten()
}

fn has_tag_name(tag: &Tag, name: &str) -> bool {
    tag.as_slice().first().map(String::as_str) == Some(name)
}

/// Pubkeys named in the repository's `maintainers` tags (the NIP-34
/// multi-value tag), lowercased and deduplicated in announcement order.
/// Malformed values are skipped rather than failing the read: a foreign or
/// hand-edited announcement must still list and update cleanly.
pub(crate) fn maintainers_from_event(event: &Event) -> Vec<String> {
    let mut maintainers: Vec<String> = Vec::new();
    for tag in event
        .tags
        .iter()
        .filter(|tag| has_tag_name(tag, "maintainers"))
    {
        for value in &tag.as_slice()[1..] {
            if crate::validate::validate_hex64(value).is_err() {
                continue;
            }
            let value = value.to_ascii_lowercase();
            if !maintainers.contains(&value) {
                maintainers.push(value);
            }
        }
    }
    maintainers
}

/// Build the single `["maintainers", <pk>, ...]` tag for `pubkeys`, or
/// `None` when the list is empty so an announcement without maintainers
/// carries no tag at all. Every pubkey is shape-validated and lowercased;
/// duplicates collapse.
fn maintainers_tag(pubkeys: &[String]) -> Result<Option<Tag>, CliError> {
    let mut values: Vec<String> = Vec::with_capacity(pubkeys.len() + 1);
    for pubkey in pubkeys {
        crate::validate::validate_hex64(pubkey).map_err(|_| {
            CliError::Usage(format!("maintainer must be a 64-char hex pubkey: {pubkey}"))
        })?;
        let pubkey = pubkey.to_ascii_lowercase();
        if !values.contains(&pubkey) {
            values.push(pubkey);
        }
    }
    if values.is_empty() {
        return Ok(None);
    }
    values.insert(0, "maintainers".to_string());
    Tag::parse(values).map(Some).map_err(tag_error)
}

fn build_protection_tag(
    ref_pattern: &str,
    push_role: Option<&str>,
    no_force_push: bool,
    no_delete: bool,
    require_patch: bool,
) -> Result<Tag, CliError> {
    let mut values = vec!["buzz-protect".to_string(), ref_pattern.to_string()];
    if let Some(role) = push_role {
        values.push(format!("push:{role}"));
    }
    if no_force_push {
        values.push("no-force-push".into());
    }
    if no_delete {
        values.push("no-delete".into());
    }
    if require_patch {
        values.push("require-patch".into());
    }
    let rule_values: Vec<&str> = values[1..].iter().map(String::as_str).collect();
    parse_protection_tag(&rule_values)
        .map_err(|error| CliError::Usage(format!("invalid protection rule: {error}")))?;
    Tag::parse(values).map_err(tag_error)
}

enum RepoChange {
    SetProtection(Box<Tag>),
    RemoveProtection(String),
    /// Bind (or rebind) the repo to a channel: replaces every existing
    /// `buzz-channel` tag with exactly one carrying the validated UUID.
    BindChannel(String),
    /// Replace the maintainer list: every existing `maintainers` tag is
    /// dropped and exactly one is written (none when the list is empty).
    SetMaintainers(Vec<String>),
}

fn build_updated_repo_announcement(
    existing: &Event,
    change: RepoChange,
) -> Result<EventBuilder, CliError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| CliError::Other(format!("read system clock: {error}")))?
        .as_secs();
    build_updated_repo_announcement_at(existing, change, now)
}

/// `now` is the wall clock in seconds; split out so the timestamp rule is
/// unit-testable without a real clock.
fn build_updated_repo_announcement_at(
    existing: &Event,
    change: RepoChange,
    now: u64,
) -> Result<EventBuilder, CliError> {
    let repo_id = repo_id_from_event(existing)?;
    // What to strip beyond `auth` (always stripped), and what to append.
    let (removed_pattern, removed_tag_name, replacement) = match change {
        RepoChange::SetProtection(tag) => {
            let pattern = protection_pattern(&tag)
                .ok_or_else(|| CliError::Other("replacement is not a protection tag".into()))?
                .to_string();
            (Some(pattern), None, Some(*tag))
        }
        RepoChange::RemoveProtection(pattern) => {
            RefPattern::parse(&pattern)
                .map_err(|error| CliError::Usage(format!("invalid ref pattern: {error}")))?;
            (Some(pattern), None, None)
        }
        RepoChange::BindChannel(channel) => {
            crate::validate::validate_uuid(&channel)?;
            let tag = Tag::parse(["buzz-channel", channel.as_str()]).map_err(tag_error)?;
            (None, Some("buzz-channel"), Some(tag))
        }
        RepoChange::SetMaintainers(pubkeys) => {
            (None, Some("maintainers"), maintainers_tag(&pubkeys)?)
        }
    };

    let mut tags: Vec<Tag> = existing
        .tags
        .iter()
        .filter(|tag| {
            if has_tag_name(tag, "auth") {
                return false;
            }
            if removed_tag_name.is_some_and(|name| has_tag_name(tag, name)) {
                return false;
            }
            removed_pattern.is_none() || protection_pattern(tag) != removed_pattern.as_deref()
        })
        .cloned()
        .collect();
    if let Some(tag) = replacement {
        tags.push(tag);
    }

    let raw_tags: Vec<Vec<String>> = tags.iter().map(|tag| tag.as_slice().to_vec()).collect();
    parse_protection_tags(&raw_tags).map_err(|error| {
        CliError::Other(format!(
            "repository contains invalid protection rules; refusing update: {error}"
        ))
    })?;

    // Stamp the update at wall-clock time, but never at or behind the observed
    // head: relays accept replaceable events only within a short drift window
    // of their own clock (±15 minutes on buzz-relay), so "head + 1" is refused
    // for every announcement older than that, and anything at or below the
    // head would lose the NIP-33 last-write-wins race to it. The read happens
    // in the same invocation as the write, so an intervening update still
    // surfaces as the relay's conflict reply rather than being leapfrogged.
    let next_created_at = existing
        .created_at
        .as_secs()
        .checked_add(1)
        .ok_or_else(|| CliError::Other("repository timestamp cannot be advanced".into()))?
        .max(now);
    buzz_sdk::build_repo_announcement_with_tags(repo_id, &existing.content, tags)
        .map_err(|error| CliError::Other(format!("failed to build repository update: {error}")))
        .map(|builder| builder.custom_created_at(Timestamp::from(next_created_at)))
}

fn protection_rules_json(event: &Event) -> Result<serde_json::Value, CliError> {
    let raw_tags: Vec<Vec<String>> = event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect();
    let (unknown_rules, validation_error) = match parse_protection_tags(&raw_tags) {
        Ok(parsed) => (parsed.unknown_rules, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let protections: Vec<serde_json::Value> = event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("buzz-protect")).then(|| {
                serde_json::json!({
                    "ref": values.get(1).map(String::as_str).unwrap_or(""),
                    "rules": values.get(2..).unwrap_or_default(),
                })
            })
        })
        .collect();

    Ok(serde_json::json!({
        "repo_id": repo_id_from_event(event)?,
        "protections": protections,
        "unknown_rules": unknown_rules,
        "validation_error": validation_error,
    }))
}

fn validate_write_response(raw: &str) -> Result<String, CliError> {
    parse_write_response(
        raw,
        "repository changed concurrently; fetch the latest rules and retry",
    )
}

async fn submit_repo_update(client: &BuzzClient, builder: EventBuilder) -> Result<(), CliError> {
    let event = client.sign_event(builder)?;
    let raw = client.submit_event(event).await?;
    println!("{}", validate_write_response(&raw)?);
    Ok(())
}

/// Build the kind:30617 announcement for `repos create`, including the
/// `buzz-channel` binding when requested.
///
/// Pure (no I/O) so the emitted tags are unit-testable. Exactly one
/// validated `buzz-channel` tag is appended — the tag is the git ACL
/// (issue #3527: without it the relay 404s every clone/fetch/push), so the
/// UUID is shape-validated here and its existence/membership is the relay's
/// authority at git-access time, same posture as `repos bind`.
///
/// `maintainers` become one NIP-34 `maintainers` tag; clients trust issue and
/// pull-request lifecycle events from those pubkeys as they do the owner's.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_create_announcement(
    repo_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    clone_urls: &[String],
    web_url: Option<&str>,
    relays: &[String],
    channel: Option<&str>,
    maintainers: &[String],
) -> Result<EventBuilder, CliError> {
    validate_repo_id(repo_id)?;

    let clone_refs: Vec<&str> = clone_urls.iter().map(|s| s.as_str()).collect();
    let relay_refs: Vec<&str> = relays.iter().map(|s| s.as_str()).collect();

    let mut builder = buzz_sdk::build_repo_announcement(
        repo_id,
        name,
        description,
        &clone_refs,
        web_url,
        &relay_refs,
    )
    .map_err(|e| CliError::Other(format!("build_repo_announcement failed: {e}")))?;

    if let Some(channel) = channel {
        crate::validate::validate_uuid(channel)?;
        builder = builder.tag(Tag::parse(["buzz-channel", channel]).map_err(tag_error)?);
    }
    if let Some(tag) = maintainers_tag(maintainers)? {
        builder = builder.tag(tag);
    }
    Ok(builder)
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_create_repo(
    client: &BuzzClient,
    repo_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    clone_urls: &[String],
    web_url: Option<&str>,
    relays: &[String],
    channel: Option<&str>,
    maintainers: &[String],
) -> Result<(), CliError> {
    let builder = build_create_announcement(
        repo_id,
        name,
        description,
        clone_urls,
        web_url,
        relays,
        channel,
        maintainers,
    )?;
    let event = client.sign_event(builder)?;
    let owner = event.pubkey.to_hex();
    let resp = client.submit_event(event).await?;
    // `link` renders as a rich preview card in Buzz Desktop when included in
    // a chat message — agents announce repos with it (see base_prompt.md).
    let link = crate::links::repo_link(&owner, repo_id);
    crate::client::print_create_response(&resp, "link", &link);
    if let Some(channel) = channel {
        // Best-effort: a repo announced into a project home channel should
        // join that project instead of rendering as a second project card.
        let _ = crate::commands::projects::try_add_own_repo_to_channel_project(
            client, channel, repo_id,
        )
        .await;
    }
    Ok(())
}

pub async fn cmd_get_repo(
    client: &BuzzClient,
    repo_id: &str,
    owner: Option<&str>,
) -> Result<(), CliError> {
    validate_repo_id(repo_id)?;

    let mut filter = serde_json::json!({
        "kinds": [30617],
        "#d": [repo_id]
    });

    // If owner specified, filter by author pubkey; otherwise return any match.
    // Note: without --owner, multiple repos with the same name (different owners) may be returned.
    if let Some(pk) = owner {
        crate::validate::validate_hex64(pk)?;
        filter["authors"] = serde_json::json!([pk]);
    }

    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_list_repos(
    client: &BuzzClient,
    owner: Option<&str>,
    limit: Option<u32>,
) -> Result<(), CliError> {
    // Default to self if no owner specified.
    let pubkey = match owner {
        Some(pk) => {
            crate::validate::validate_hex64(pk)?;
            pk.to_string()
        }
        None => client.keys().public_key().to_hex(),
    };

    let mut filter = serde_json::json!({
        "kinds": [30617],
        "authors": [pubkey]
    });

    if let Some(n) = limit {
        filter["limit"] = serde_json::json!(n);
    }

    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

async fn current_repo(client: &BuzzClient, repo_id: &str) -> Result<Event, CliError> {
    validate_repo_id(repo_id)?;
    fetch_own_repo_announcement(client, repo_id)
        .await?
        .ok_or_else(|| {
            CliError::NotFound(format!(
                "repository {repo_id:?} was not found for the current identity"
            ))
        })
}

async fn cmd_protect_list(client: &BuzzClient, repo_id: &str) -> Result<(), CliError> {
    let event = current_repo(client, repo_id).await?;
    println!("{}", protection_rules_json(&event)?);
    Ok(())
}

async fn cmd_protect_set(
    client: &BuzzClient,
    repo_id: &str,
    ref_pattern: &str,
    push_role: Option<crate::RepoPushRole>,
    no_force_push: bool,
    no_delete: bool,
    require_patch: bool,
) -> Result<(), CliError> {
    let push_role = push_role.map(|role| match role {
        crate::RepoPushRole::Owner => "owner",
        crate::RepoPushRole::Admin => "admin",
        crate::RepoPushRole::Member => "member",
    });
    let tag = build_protection_tag(
        ref_pattern,
        push_role,
        no_force_push,
        no_delete,
        require_patch,
    )?;
    let event = current_repo(client, repo_id).await?;
    let builder =
        build_updated_repo_announcement(&event, RepoChange::SetProtection(Box::new(tag)))?;
    submit_repo_update(client, builder).await
}

async fn cmd_protect_remove(
    client: &BuzzClient,
    repo_id: &str,
    ref_pattern: &str,
) -> Result<(), CliError> {
    RefPattern::parse(ref_pattern)
        .map_err(|error| CliError::Usage(format!("invalid ref pattern: {error}")))?;
    let event = current_repo(client, repo_id).await?;
    if !event
        .tags
        .iter()
        .any(|tag| protection_pattern(tag) == Some(ref_pattern))
    {
        return Err(CliError::NotFound(format!(
            "repository {repo_id:?} has no protection rule for {ref_pattern:?}"
        )));
    }
    let builder = build_updated_repo_announcement(
        &event,
        RepoChange::RemoveProtection(ref_pattern.to_string()),
    )?;
    submit_repo_update(client, builder).await
}

/// Bind (or rebind) a repository to a channel — the fix path for issue
/// #3527's permanently-404 repos. Publishes a read-modify-write update of
/// the caller's own kind:30617 with exactly one `buzz-channel` tag; all
/// other metadata (protections, name, description, future tags) is
/// preserved by the same machinery `repos protect` uses.
///
/// The UUID is validated for *shape* only — deliberately. Channel existence
/// and the caller's membership are the relay's authority at git-access
/// time; a CLI-side network pre-check would just be TOCTOU with extra
/// latency.
async fn cmd_bind_repo(client: &BuzzClient, repo_id: &str, channel: &str) -> Result<(), CliError> {
    let event = current_repo(client, repo_id).await?;
    let builder =
        build_updated_repo_announcement(&event, RepoChange::BindChannel(channel.to_string()))?;
    submit_repo_update(client, builder).await
}

fn maintainers_json(event: &Event) -> Result<serde_json::Value, CliError> {
    Ok(serde_json::json!({
        "repo_id": repo_id_from_event(event)?,
        "owner": event.pubkey.to_hex(),
        "maintainers": maintainers_from_event(event),
    }))
}

async fn cmd_maintainers_list(client: &BuzzClient, repo_id: &str) -> Result<(), CliError> {
    let event = current_repo(client, repo_id).await?;
    println!("{}", maintainers_json(&event)?);
    Ok(())
}

/// Add maintainers to one of your repositories. Already-listed pubkeys are
/// kept in place; the owner never needs listing and is dropped if given.
async fn cmd_maintainers_add(
    client: &BuzzClient,
    repo_id: &str,
    pubkeys: &[String],
) -> Result<(), CliError> {
    for pubkey in pubkeys {
        crate::validate::validate_hex64(pubkey)?;
    }
    let event = current_repo(client, repo_id).await?;
    let owner = event.pubkey.to_hex();
    let mut maintainers = maintainers_from_event(&event);
    for pubkey in pubkeys {
        let pubkey = pubkey.to_ascii_lowercase();
        if pubkey != owner && !maintainers.contains(&pubkey) {
            maintainers.push(pubkey);
        }
    }
    if maintainers == maintainers_from_event(&event) {
        println!("{}", maintainers_json(&event)?);
        return Ok(());
    }
    let builder = build_updated_repo_announcement(&event, RepoChange::SetMaintainers(maintainers))?;
    submit_repo_update(client, builder).await
}

async fn cmd_maintainers_remove(
    client: &BuzzClient,
    repo_id: &str,
    pubkeys: &[String],
) -> Result<(), CliError> {
    for pubkey in pubkeys {
        crate::validate::validate_hex64(pubkey)?;
    }
    let event = current_repo(client, repo_id).await?;
    let current = maintainers_from_event(&event);
    let removed: Vec<String> = pubkeys.iter().map(|p| p.to_ascii_lowercase()).collect();
    let maintainers: Vec<String> = current
        .iter()
        .filter(|pubkey| !removed.contains(pubkey))
        .cloned()
        .collect();
    if maintainers.len() == current.len() {
        return Err(CliError::NotFound(format!(
            "repository {repo_id:?} lists none of the given pubkeys as maintainers"
        )));
    }
    let builder = build_updated_repo_announcement(&event, RepoChange::SetMaintainers(maintainers))?;
    submit_repo_update(client, builder).await
}

pub async fn dispatch(cmd: crate::ReposCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::{ReposCmd, ReposMaintainersCmd, ReposProtectCmd};
    match cmd {
        ReposCmd::Create {
            id,
            name,
            description,
            clone_urls,
            web,
            relays,
            channel,
            maintainers,
        } => {
            cmd_create_repo(
                client,
                &id,
                name.as_deref(),
                description.as_deref(),
                &clone_urls,
                web.as_deref(),
                &relays,
                channel.as_deref(),
                &maintainers,
            )
            .await
        }
        ReposCmd::Get { id, owner } => cmd_get_repo(client, &id, owner.as_deref()).await,
        ReposCmd::List { owner, limit } => cmd_list_repos(client, owner.as_deref(), limit).await,
        ReposCmd::Bind { id, channel } => cmd_bind_repo(client, &id, &channel).await,
        ReposCmd::DefaultBranch(command) => {
            super::repo_default_branch::dispatch(command, client).await
        }
        ReposCmd::Protect(command) => match command {
            ReposProtectCmd::List { id } => cmd_protect_list(client, &id).await,
            ReposProtectCmd::Set {
                id,
                ref_pattern,
                push,
                no_force_push,
                no_delete,
                require_patch,
            } => {
                cmd_protect_set(
                    client,
                    &id,
                    &ref_pattern,
                    push,
                    no_force_push,
                    no_delete,
                    require_patch,
                )
                .await
            }
            ReposProtectCmd::Remove { id, ref_pattern } => {
                cmd_protect_remove(client, &id, &ref_pattern).await
            }
        },
        ReposCmd::Maintainers(command) => match command {
            ReposMaintainersCmd::List { id } => cmd_maintainers_list(client, &id).await,
            ReposMaintainersCmd::Add { id, pubkeys } => {
                cmd_maintainers_add(client, &id, &pubkeys).await
            }
            ReposMaintainersCmd::Remove { id, pubkeys } => {
                cmd_maintainers_remove(client, &id, &pubkeys).await
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    use super::{
        build_create_announcement, build_protection_tag, build_updated_repo_announcement,
        build_updated_repo_announcement_at, protection_rules_json, validate_write_response,
        RepoChange,
    };

    fn signed_repo(tags: Vec<Tag>, content: &str, created_at: u64) -> nostr::Event {
        EventBuilder::new(Kind::Custom(30617), content)
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(&Keys::generate())
            .expect("sign repository event")
    }

    fn tag(parts: &[&str]) -> Tag {
        Tag::parse(parts.iter().copied()).expect("valid test tag")
    }

    #[test]
    fn protection_update_preserves_metadata_and_replaces_only_matching_pattern() {
        let existing = signed_repo(
            vec![
                tag(&["d", "demo"]),
                tag(&["name", "Demo"]),
                tag(&["buzz-channel", "channel-id"]),
                tag(&["future-metadata", "preserve-me"]),
                tag(&["auth", &"a".repeat(64), "kind=30617", &"b".repeat(128)]),
                tag(&["buzz-protect", "refs/heads/main", "push:member"]),
                tag(&["buzz-protect", "refs/tags/*", "no-delete"]),
            ],
            "repository content",
            100,
        );
        let replacement = build_protection_tag("refs/heads/main", Some("admin"), true, true, false)
            .expect("valid replacement");

        let updated = build_updated_repo_announcement_at(
            &existing,
            RepoChange::SetProtection(Box::new(replacement)),
            0,
        )
        .expect("build update")
        .sign_with_keys(&Keys::generate())
        .expect("sign update");

        assert_eq!(updated.content, "repository content");
        assert_eq!(updated.created_at.as_secs(), 101);
        assert!(!updated
            .tags
            .iter()
            .any(|tag| tag.as_slice().first().map(String::as_str) == Some("auth")));
        assert!(updated
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["buzz-channel", "channel-id"]));
        assert!(updated
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["future-metadata", "preserve-me"]));
        assert!(updated.tags.iter().any(|tag| {
            tag.as_slice()
                == [
                    "buzz-protect",
                    "refs/heads/main",
                    "push:admin",
                    "no-force-push",
                    "no-delete",
                ]
        }));
        assert!(updated
            .tags
            .iter()
            .any(|tag| { tag.as_slice() == ["buzz-protect", "refs/tags/*", "no-delete"] }));
        assert_eq!(
            updated
                .tags
                .iter()
                .filter(|tag| {
                    let values = tag.as_slice();
                    values.first().map(String::as_str) == Some("buzz-protect")
                        && values.get(1).map(String::as_str) == Some("refs/heads/main")
                })
                .count(),
            1
        );
    }

    #[test]
    fn protection_remove_preserves_other_patterns() {
        let existing = signed_repo(
            vec![
                tag(&["d", "demo"]),
                tag(&["buzz-protect", "refs/heads/main", "no-delete"]),
                tag(&["buzz-protect", "refs/heads/release", "push:owner"]),
            ],
            "",
            10,
        );

        let updated = build_updated_repo_announcement(
            &existing,
            RepoChange::RemoveProtection("refs/heads/main".into()),
        )
        .expect("build removal")
        .sign_with_keys(&Keys::generate())
        .expect("sign removal");

        assert!(!updated
            .tags
            .iter()
            .any(|tag| tag.as_slice().get(1).map(String::as_str) == Some("refs/heads/main")));
        assert!(updated
            .tags
            .iter()
            .any(|tag| { tag.as_slice() == ["buzz-protect", "refs/heads/release", "push:owner"] }));
    }

    #[test]
    fn protection_set_requires_at_least_one_rule() {
        assert!(build_protection_tag("refs/heads/main", None, false, false, false).is_err());
    }

    #[test]
    fn protection_update_rejects_malformed_existing_rules() {
        let existing = signed_repo(
            vec![
                tag(&["d", "demo"]),
                tag(&["buzz-protect", "refs/heads/main"]),
            ],
            "",
            10,
        );
        let replacement =
            build_protection_tag("refs/heads/release", Some("admin"), false, false, false)
                .expect("valid replacement");

        let error = build_updated_repo_announcement(
            &existing,
            RepoChange::SetProtection(Box::new(replacement)),
        )
        .expect_err("malformed existing rule must fail closed");

        assert!(error
            .to_string()
            .contains("repository contains invalid protection rules"));
    }

    #[test]
    fn protection_update_enforces_repository_rule_limit() {
        let mut tags = vec![tag(&["d", "demo"])];
        for index in 0..50 {
            tags.push(tag(&[
                "buzz-protect",
                &format!("refs/heads/branch-{index}"),
                "push:member",
            ]));
        }
        let existing = signed_repo(tags, "", 10);
        let replacement =
            build_protection_tag("refs/heads/main", Some("admin"), false, false, false)
                .expect("valid replacement");

        let error = build_updated_repo_announcement(
            &existing,
            RepoChange::SetProtection(Box::new(replacement)),
        )
        .expect_err("the 51st rule must be rejected");

        assert!(error.to_string().contains("exceeds max 50"));
    }

    #[test]
    fn protection_list_keeps_unknown_rules_visible() {
        let existing = signed_repo(
            vec![
                tag(&["d", "demo"]),
                tag(&[
                    "buzz-protect",
                    "refs/heads/main",
                    "push:admin",
                    "future-rule",
                ]),
            ],
            "",
            10,
        );

        let json = protection_rules_json(&existing).expect("list protections");
        assert_eq!(json["repo_id"], "demo");
        assert_eq!(json["protections"][0]["ref"], "refs/heads/main");
        assert_eq!(
            json["protections"][0]["rules"],
            serde_json::json!(["push:admin", "future-rule"])
        );
        assert_eq!(json["validation_error"], serde_json::Value::Null);
    }

    #[test]
    fn protection_list_surfaces_malformed_rules_for_recovery() {
        let existing = signed_repo(
            vec![
                tag(&["d", "demo"]),
                tag(&["buzz-protect", "refs/heads/main"]),
            ],
            "",
            10,
        );

        let json = protection_rules_json(&existing).expect("list malformed protections");
        assert_eq!(json["protections"][0]["ref"], "refs/heads/main");
        assert!(json["validation_error"]
            .as_str()
            .is_some_and(|error| error.contains("needs pattern + at least one rule")));
    }

    #[test]
    fn bind_channel_replaces_duplicates_and_preserves_everything_else() {
        let channel = uuid::Uuid::new_v4().to_string();
        let existing = signed_repo(
            vec![
                tag(&["d", "demo"]),
                tag(&["name", "Demo"]),
                // Two stale bindings — e.g. from a buggy or vanilla client.
                tag(&["buzz-channel", "old-and-broken"]),
                tag(&["buzz-channel", &uuid::Uuid::new_v4().to_string()]),
                tag(&["auth", &"a".repeat(64), "kind=30617", &"b".repeat(128)]),
                tag(&["buzz-protect", "refs/heads/main", "push:admin"]),
                tag(&["future-metadata", "preserve-me"]),
            ],
            "repository content",
            100,
        );

        let updated = build_updated_repo_announcement_at(
            &existing,
            RepoChange::BindChannel(channel.clone()),
            0,
        )
        .expect("build bind update")
        .sign_with_keys(&Keys::generate())
        .expect("sign bind update");

        assert_eq!(updated.content, "repository content");
        assert_eq!(updated.created_at.as_secs(), 101);
        // Exactly one binding remains, and it is the requested one.
        let bindings: Vec<_> = updated
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("buzz-channel"))
            .collect();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].as_slice(), ["buzz-channel", channel.as_str()]);
        // Auth stripped (relay re-stamps); everything else preserved.
        assert!(!updated
            .tags
            .iter()
            .any(|tag| tag.as_slice().first().map(String::as_str) == Some("auth")));
        assert!(updated
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["buzz-protect", "refs/heads/main", "push:admin"]));
        assert!(updated
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["future-metadata", "preserve-me"]));
        assert!(updated
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["name", "Demo"]));
    }

    #[test]
    fn bind_channel_adds_binding_to_unbound_repo() {
        let channel = uuid::Uuid::new_v4().to_string();
        let existing = signed_repo(vec![tag(&["d", "demo"])], "", 10);

        let updated = build_updated_repo_announcement_at(
            &existing,
            RepoChange::BindChannel(channel.clone()),
            0,
        )
        .expect("build bind update")
        .sign_with_keys(&Keys::generate())
        .expect("sign bind update");

        assert!(updated
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["buzz-channel", channel.as_str()]));
    }

    #[test]
    fn bind_channel_rejects_malformed_uuid() {
        let existing = signed_repo(vec![tag(&["d", "demo"])], "", 10);

        let error =
            build_updated_repo_announcement(&existing, RepoChange::BindChannel("nope".into()))
                .expect_err("malformed channel id must not build an update");

        assert!(matches!(error, crate::error::CliError::Usage(_)));
    }

    /// Issue #3527: `repos create --channel` must emit exactly one
    /// `buzz-channel` tag so the primary create command stops producing
    /// repos the relay 404s forever.
    #[test]
    fn create_with_channel_emits_exactly_one_binding_tag() {
        let channel = uuid::Uuid::new_v4().to_string();
        let event = build_create_announcement(
            "demo",
            Some("Demo"),
            None,
            &["https://relay.example/git/owner/demo".to_string()],
            None,
            &[],
            Some(&channel),
            &[],
        )
        .expect("build create announcement")
        .sign_with_keys(&Keys::generate())
        .expect("sign create announcement");

        assert_eq!(event.kind, Kind::Custom(30617));
        let bindings: Vec<_> = event
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("buzz-channel"))
            .collect();
        assert_eq!(bindings.len(), 1, "exactly one buzz-channel tag");
        assert_eq!(bindings[0].as_slice(), ["buzz-channel", channel.as_str()]);
        // The standard metadata still rides along.
        assert!(event.tags.iter().any(|tag| tag.as_slice() == ["d", "demo"]));
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["name", "Demo"]));
    }

    #[test]
    fn create_without_channel_emits_no_binding_tag() {
        let event = build_create_announcement("demo", None, None, &[], None, &[], None, &[])
            .expect("build create announcement")
            .sign_with_keys(&Keys::generate())
            .expect("sign create announcement");

        assert!(
            !event
                .tags
                .iter()
                .any(|tag| tag.as_slice().first().map(String::as_str) == Some("buzz-channel")),
            "no --channel means no binding tag (vanilla NIP-34 stays possible)"
        );
    }

    #[test]
    fn create_rejects_malformed_channel_uuid() {
        let error =
            build_create_announcement("demo", None, None, &[], None, &[], Some("nope"), &[])
                .expect_err("malformed channel id must not build an announcement");
        assert!(matches!(error, crate::error::CliError::Usage(_)));
    }

    #[test]
    fn create_with_maintainers_emits_one_multi_value_tag() {
        let alice = "a".repeat(64);
        let bob = "B".repeat(64);
        let event = build_create_announcement(
            "demo",
            None,
            None,
            &[],
            None,
            &[],
            None,
            &[alice.clone(), bob.clone(), alice.clone()],
        )
        .expect("build create announcement")
        .sign_with_keys(&Keys::generate())
        .expect("sign create announcement");

        let tags: Vec<_> = event
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("maintainers"))
            .collect();
        assert_eq!(tags.len(), 1, "exactly one maintainers tag");
        assert_eq!(
            tags[0].as_slice(),
            [
                "maintainers",
                alice.as_str(),
                bob.to_ascii_lowercase().as_str()
            ],
            "lowercased, deduplicated, announcement order"
        );
        assert_eq!(
            super::maintainers_from_event(&event),
            vec![alice, bob.to_ascii_lowercase()]
        );
    }

    #[test]
    fn create_without_maintainers_emits_no_tag() {
        let event = build_create_announcement("demo", None, None, &[], None, &[], None, &[])
            .expect("build create announcement")
            .sign_with_keys(&Keys::generate())
            .expect("sign create announcement");
        assert!(!event
            .tags
            .iter()
            .any(|tag| tag.as_slice().first().map(String::as_str) == Some("maintainers")));
        assert!(super::maintainers_from_event(&event).is_empty());
    }

    #[test]
    fn create_rejects_malformed_maintainer() {
        let error =
            build_create_announcement("demo", None, None, &[], None, &[], None, &["nope".into()])
                .expect_err("malformed maintainer pubkey must not build an announcement");
        assert!(matches!(error, crate::error::CliError::Usage(_)));
    }

    #[test]
    fn set_maintainers_replaces_every_existing_tag_and_preserves_the_rest() {
        let old = "1".repeat(64);
        let kept = "2".repeat(64);
        let added = "3".repeat(64);
        let existing = signed_repo(
            vec![
                tag(&["d", "demo"]),
                tag(&["name", "Demo"]),
                tag(&["buzz-channel", "channel-id"]),
                tag(&["auth", &"a".repeat(64), "", &"b".repeat(128)]),
                tag(&["buzz-protect", "refs/heads/main", "push:member"]),
                tag(&["maintainers", &old]),
                tag(&["maintainers", &kept, &old]),
            ],
            "repository content",
            100,
        );
        assert_eq!(
            super::maintainers_from_event(&existing),
            vec![old.clone(), kept.clone()]
        );

        let updated = build_updated_repo_announcement_at(
            &existing,
            RepoChange::SetMaintainers(vec![kept.clone(), added.clone()]),
            0,
        )
        .expect("build update")
        .sign_with_keys(&Keys::generate())
        .expect("sign update");

        assert_eq!(updated.content, "repository content");
        assert_eq!(updated.created_at.as_secs(), 101);
        let tags: Vec<_> = updated
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("maintainers"))
            .collect();
        assert_eq!(tags.len(), 1);
        assert_eq!(
            tags[0].as_slice(),
            ["maintainers", kept.as_str(), added.as_str()]
        );
        assert!(!updated
            .tags
            .iter()
            .any(|tag| tag.as_slice().first().map(String::as_str) == Some("auth")));
        assert!(updated
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["buzz-channel", "channel-id"]));
        assert!(updated
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["buzz-protect", "refs/heads/main", "push:member"]));
        assert!(updated
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["name", "Demo"]));
    }

    #[test]
    fn update_is_stamped_at_wall_clock_but_never_behind_the_head() {
        let existing = signed_repo(vec![tag(&["d", "demo"])], "", 100);
        // Old head: a "head + 1" stamp would be refused by the relay's drift
        // window, so the update carries the wall clock.
        let updated = build_updated_repo_announcement_at(
            &existing,
            RepoChange::SetMaintainers(vec!["1".repeat(64)]),
            5_000,
        )
        .expect("build update")
        .sign_with_keys(&Keys::generate())
        .expect("sign update");
        assert_eq!(updated.created_at.as_secs(), 5_000);

        // Head ahead of the clock (skewed writer): stay strictly after it so
        // the replaceable write still wins last-write-wins.
        let skewed = signed_repo(vec![tag(&["d", "demo"])], "", 10_000);
        let updated = build_updated_repo_announcement_at(
            &skewed,
            RepoChange::SetMaintainers(vec!["1".repeat(64)]),
            5_000,
        )
        .expect("build update")
        .sign_with_keys(&Keys::generate())
        .expect("sign update");
        assert_eq!(updated.created_at.as_secs(), 10_001);
    }

    #[test]
    fn set_maintainers_empty_drops_the_tag() {
        let existing = signed_repo(
            vec![tag(&["d", "demo"]), tag(&["maintainers", &"1".repeat(64)])],
            "",
            100,
        );
        let updated =
            build_updated_repo_announcement(&existing, RepoChange::SetMaintainers(vec![]))
                .expect("build update")
                .sign_with_keys(&Keys::generate())
                .expect("sign update");
        assert!(!updated
            .tags
            .iter()
            .any(|tag| tag.as_slice().first().map(String::as_str) == Some("maintainers")));
        assert!(updated
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["d", "demo"]));
    }

    #[test]
    fn maintainers_from_event_skips_malformed_values() {
        let good = "4".repeat(64);
        let existing = signed_repo(
            vec![
                tag(&["d", "demo"]),
                tag(&["maintainers", "npub1notahexkey", &good]),
            ],
            "",
            100,
        );
        assert_eq!(super::maintainers_from_event(&existing), vec![good]);
    }

    #[test]
    fn duplicate_write_response_is_a_conflict() {
        let error = validate_write_response(
            r#"{"event_id":"abc","accepted":true,"message":"duplicate: superseded"}"#,
        )
        .expect_err("dominated writes must not report success");

        assert!(matches!(error, crate::error::CliError::Conflict(_)));
    }

    #[test]
    fn successful_write_response_is_normalized() {
        let output = validate_write_response(
            r#"{"event_id":"abc","accepted":true,"message":"saved","extra":"ignored"}"#,
        )
        .expect("accepted write");

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output).expect("normalized JSON"),
            serde_json::json!({
                "event_id": "abc",
                "accepted": true,
                "message": "saved",
            })
        );
    }
}
