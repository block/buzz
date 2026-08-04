use buzz_core::{
    git_perms::{parse_protection_tag, parse_protection_tags, RefPattern},
    kind::KIND_GIT_REPO_ANNOUNCEMENT,
};
use nostr::{Event, EventBuilder, Tag, Timestamp};

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::validate_repo_id;

fn parse_events(json: &str) -> Result<Vec<Event>, CliError> {
    serde_json::from_str(json)
        .map_err(|error| CliError::Other(format!("failed to parse relay response: {error}")))
}

async fn fetch_own_repo_announcement(
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
}

const MAIN_REF: &str = "refs/heads/main";
const MAIN_PROTECTION: [&str; 5] = [
    "buzz-protect",
    MAIN_REF,
    "push:admin",
    "no-force-push",
    "no-delete",
];

enum RepoAnnouncementPlan {
    Replay(Event),
    Publish(EventBuilder),
}

const CREATE_RECONCILED_MESSAGE: &str = "reconciled: repository provisioning ready";

fn tag_values(tag: &Tag) -> &[String] {
    tag.as_slice()
}

fn github_mirror(url: &str) -> bool {
    super::repo_sync::validate_github_clone(url).is_ok()
}

fn existing_github_mirror(existing: Option<&Event>) -> Option<String> {
    existing
        .into_iter()
        .flat_map(|event| event.tags.iter())
        .filter(|tag| has_tag_name(tag, "clone"))
        .flat_map(|tag| tag_values(tag).iter().skip(1))
        .find(|url| github_mirror(url))
        .cloned()
}

fn canonical_clone_urls(
    buzz_url: &str,
    requested: &[String],
    existing: Option<&Event>,
) -> Result<Vec<String>, CliError> {
    let mut mirrors: Vec<String> = requested
        .iter()
        .filter(|url| url.as_str() != buzz_url)
        .cloned()
        .collect();
    mirrors.sort();
    mirrors.dedup();

    let mirror = if mirrors.is_empty() {
        existing_github_mirror(existing)
    } else if mirrors.len() == 1 && github_mirror(&mirrors[0]) {
        mirrors.pop()
    } else {
        return Err(CliError::Usage(
            "--clone accepts the active Buzz repository URL and at most one github.com mirror"
                .into(),
        ));
    };

    let mut urls = vec![buzz_url.to_string()];
    if let Some(mirror) = mirror {
        urls.push(mirror);
    }
    Ok(urls)
}

fn replace_optional_metadata(
    tags: &mut Vec<Tag>,
    name: &str,
    value: Option<&str>,
) -> Result<(), CliError> {
    if let Some(value) = value {
        tags.retain(|tag| !has_tag_name(tag, name));
        tags.push(Tag::parse([name, value]).map_err(tag_error)?);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn plan_repo_announcement(
    existing: Option<&Event>,
    repo_id: &str,
    owner: &str,
    relay_url: &str,
    name: Option<&str>,
    description: Option<&str>,
    clone_urls: &[String],
    web_url: Option<&str>,
    relays: &[String],
    channel: &str,
) -> Result<RepoAnnouncementPlan, CliError> {
    validate_repo_id(repo_id)?;
    crate::validate::validate_hex64(owner)?;
    let channel = uuid::Uuid::parse_str(channel)
        .map_err(|error| CliError::Usage(format!("channel must be a valid UUID: {error}")))?
        .to_string();
    let buzz_url = format!("{}/git/{owner}/{repo_id}", relay_url.trim_end_matches('/'));
    let clone_urls = canonical_clone_urls(&buzz_url, clone_urls, existing)?;

    // Keep SDK validation authoritative for standard NIP-34 metadata.
    let clone_refs: Vec<&str> = clone_urls.iter().map(String::as_str).collect();
    let relay_refs: Vec<&str> = relays.iter().map(String::as_str).collect();
    buzz_sdk::build_repo_announcement(
        repo_id,
        name,
        description,
        &clone_refs,
        web_url,
        &relay_refs,
    )
    .map_err(|error| CliError::Usage(error.to_string()))?;

    let mut tags: Vec<Tag> = existing
        .map(|event| {
            event
                .tags
                .iter()
                .filter(|tag| {
                    !matches!(
                        tag_values(tag).first().map(String::as_str),
                        Some("auth" | "d" | "buzz-channel" | "clone")
                    ) && protection_pattern(tag) != Some(MAIN_REF)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    replace_optional_metadata(&mut tags, "name", name)?;
    replace_optional_metadata(&mut tags, "description", description)?;
    replace_optional_metadata(&mut tags, "web", web_url)?;
    if !relays.is_empty() {
        tags.retain(|tag| !has_tag_name(tag, "relays"));
        let mut values = vec!["relays".to_string()];
        values.extend(relays.iter().cloned());
        tags.push(Tag::parse(values).map_err(tag_error)?);
    }

    tags.insert(0, Tag::parse(["d", repo_id]).map_err(tag_error)?);
    let mut clone_tag = vec!["clone".to_string()];
    clone_tag.extend(clone_urls);
    tags.push(Tag::parse(clone_tag).map_err(tag_error)?);
    tags.push(Tag::parse(["buzz-channel", channel.as_str()]).map_err(tag_error)?);
    tags.push(Tag::parse(MAIN_PROTECTION).map_err(tag_error)?);

    let raw_tags: Vec<Vec<String>> = tags.iter().map(|tag| tag.as_slice().to_vec()).collect();
    parse_protection_tags(&raw_tags).map_err(|error| {
        CliError::Other(format!(
            "repository contains invalid protection rules; refusing update: {error}"
        ))
    })?;

    if let Some(existing) = existing.filter(|event| {
        let semantic_tags: Vec<&Tag> = event
            .tags
            .iter()
            .filter(|tag| !has_tag_name(tag, "auth"))
            .collect();
        semantic_tags.len() == tags.len()
            && semantic_tags
                .iter()
                .zip(&tags)
                .all(|(left, right)| left.as_slice() == right.as_slice())
    }) {
        // A semantically current announcement does not prove the relay-side
        // bare repository exists. Replay the exact signed event so the relay
        // can reconcile provisioning without creating a newer announcement.
        return Ok(RepoAnnouncementPlan::Replay(existing.clone()));
    }

    let content = existing.map(|event| event.content.as_str()).unwrap_or("");
    let mut builder = buzz_sdk::build_repo_announcement_with_tags(repo_id, content, tags)
        .map_err(|error| CliError::Other(format!("failed to build repository update: {error}")))?;
    if let Some(existing) = existing {
        let next_created_at = existing
            .created_at
            .as_secs()
            .checked_add(1)
            .ok_or_else(|| CliError::Other("repository timestamp cannot be advanced".into()))?;
        builder = builder.custom_created_at(Timestamp::from(next_created_at));
    }
    Ok(RepoAnnouncementPlan::Publish(builder))
}

fn build_updated_repo_announcement(
    existing: &Event,
    change: RepoChange,
) -> Result<EventBuilder, CliError> {
    let repo_id = repo_id_from_event(existing)?;
    // What to strip beyond `auth` (always stripped), and what to append.
    let (removed_pattern, removed_channel, replacement) = match change {
        RepoChange::SetProtection(tag) => {
            let pattern = protection_pattern(&tag)
                .ok_or_else(|| CliError::Other("replacement is not a protection tag".into()))?
                .to_string();
            (Some(pattern), false, Some(*tag))
        }
        RepoChange::RemoveProtection(pattern) => {
            RefPattern::parse(&pattern)
                .map_err(|error| CliError::Usage(format!("invalid ref pattern: {error}")))?;
            (Some(pattern), false, None)
        }
        RepoChange::BindChannel(channel) => {
            crate::validate::validate_uuid(&channel)?;
            let tag = Tag::parse(["buzz-channel", channel.as_str()]).map_err(tag_error)?;
            (None, true, Some(tag))
        }
    };

    let mut tags: Vec<Tag> = existing
        .tags
        .iter()
        .filter(|tag| {
            if has_tag_name(tag, "auth") {
                return false;
            }
            if removed_channel && has_tag_name(tag, "buzz-channel") {
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

    // Advance only the observed head. Using wall-clock time here would let a
    // delayed writer leapfrog an intervening update and silently erase metadata.
    let next_created_at = existing
        .created_at
        .as_secs()
        .checked_add(1)
        .ok_or_else(|| CliError::Other("repository timestamp cannot be advanced".into()))?;
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
    let response: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| CliError::Other(format!("relay response is not JSON: {error} ({raw})")))?;
    let accepted = response
        .get("accepted")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let message = response
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !accepted {
        return Err(CliError::Other(format!("relay rejected event: {message}")));
    }
    if message == "duplicate" || message.starts_with("duplicate:") {
        return Err(CliError::Conflict(
            "repository changed concurrently; fetch the latest rules and retry".into(),
        ));
    }
    Ok(normalize_write_response(raw))
}

fn validate_create_replay_response(raw: &str) -> Result<String, CliError> {
    let response: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| CliError::Other(format!("relay response is not JSON: {error} ({raw})")))?;
    let accepted = response
        .get("accepted")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let message = response
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !accepted {
        return Err(CliError::Other(format!("relay rejected event: {message}")));
    }
    if message == "duplicate" || message.starts_with("duplicate:") {
        return Err(CliError::Conflict(
            "repository provisioning was not reconciled; retry create against the relay".into(),
        ));
    }
    if message != CREATE_RECONCILED_MESSAGE {
        return Err(CliError::Other(format!(
            "relay did not confirm repository provisioning reconciliation: {message}"
        )));
    }
    Ok(normalize_write_response(raw))
}

async fn submit_repo_update(client: &BuzzClient, builder: EventBuilder) -> Result<(), CliError> {
    let event = client.sign_event(builder)?;
    let raw = client.submit_event(event).await?;
    println!("{}", validate_write_response(&raw)?);
    Ok(())
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
    channel: &str,
) -> Result<(), CliError> {
    let existing = fetch_own_repo_announcement(client, repo_id).await?;
    let plan = plan_repo_announcement(
        existing.as_ref(),
        repo_id,
        &client.keys().public_key().to_hex(),
        client.relay_url(),
        name,
        description,
        clone_urls,
        web_url,
        relays,
        channel,
    )?;
    match plan {
        RepoAnnouncementPlan::Replay(event) => {
            let raw = client.submit_event(event).await?;
            println!("{}", validate_create_replay_response(&raw)?);
            Ok(())
        }
        RepoAnnouncementPlan::Publish(builder) => submit_repo_update(client, builder).await,
    }
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

pub async fn dispatch(cmd: crate::ReposCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::{ReposCmd, ReposProtectCmd};
    match cmd {
        ReposCmd::Create {
            id,
            name,
            description,
            clone_urls,
            web,
            relays,
            channel,
        } => {
            cmd_create_repo(
                client,
                &id,
                name.as_deref(),
                description.as_deref(),
                &clone_urls,
                web.as_deref(),
                &relays,
                &channel,
            )
            .await
        }
        ReposCmd::Get { id, owner } => cmd_get_repo(client, &id, owner.as_deref()).await,
        ReposCmd::List { owner, limit } => cmd_list_repos(client, owner.as_deref(), limit).await,
        ReposCmd::Status { id } => {
            let announcement = current_repo(client, &id).await?;
            crate::commands::repo_sync::cmd_status(client, &announcement).await
        }
        ReposCmd::ImportMain { id, commit } => {
            let announcement = current_repo(client, &id).await?;
            crate::commands::repo_sync::cmd_import_main(client, &announcement, &commit).await
        }
        ReposCmd::MirrorMain {
            id,
            commit,
            expected_buzz_main,
        } => {
            let announcement = current_repo(client, &id).await?;
            crate::commands::repo_sync::cmd_mirror_main(
                client,
                &announcement,
                &commit,
                &expected_buzz_main,
            )
            .await
        }
        ReposCmd::Bind { id, channel } => cmd_bind_repo(client, &id, &channel).await,
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
    }
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    use super::{
        build_protection_tag, build_updated_repo_announcement, plan_repo_announcement,
        protection_rules_json, validate_create_replay_response, validate_write_response,
        RepoAnnouncementPlan, RepoChange,
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

        let updated = build_updated_repo_announcement(
            &existing,
            RepoChange::SetProtection(Box::new(replacement)),
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

        let updated =
            build_updated_repo_announcement(&existing, RepoChange::BindChannel(channel.clone()))
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

        let updated =
            build_updated_repo_announcement(&existing, RepoChange::BindChannel(channel.clone()))
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

    #[test]
    fn desired_create_is_buzz_first_and_atomically_protected() {
        let channel = uuid::Uuid::new_v4().to_string();
        let keys = Keys::generate();
        let owner = keys.public_key().to_hex();
        let mirror = "https://github.com/example/demo.git".to_string();
        let RepoAnnouncementPlan::Publish(builder) = plan_repo_announcement(
            None,
            "demo",
            &owner,
            "https://relay.example/",
            Some("Demo"),
            None,
            std::slice::from_ref(&mirror),
            None,
            &[],
            &channel,
        )
        .expect("plan create") else {
            panic!("new repository must publish");
        };
        let event = builder.sign_with_keys(&keys).expect("sign create");

        assert_eq!(event.kind, Kind::Custom(30617));
        assert_eq!(event.pubkey, keys.public_key());
        assert_eq!(
            event
                .tags
                .iter()
                .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("d"))
                .count(),
            1
        );
        assert!(event.tags.iter().any(|tag| tag.as_slice() == ["d", "demo"]));
        assert_eq!(
            event
                .tags
                .iter()
                .find(|tag| tag.as_slice().first().map(String::as_str) == Some("clone"))
                .expect("clone tag")
                .as_slice(),
            [
                "clone",
                &format!("https://relay.example/git/{owner}/demo"),
                mirror.as_str(),
            ]
        );
        assert_eq!(
            event
                .tags
                .iter()
                .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("buzz-channel"))
                .count(),
            1
        );
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["buzz-channel", channel.as_str()]));
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == super::MAIN_PROTECTION));
        assert_eq!(
            event
                .tags
                .iter()
                .filter(|tag| super::protection_pattern(tag) == Some(super::MAIN_REF))
                .count(),
            1
        );
    }

    #[test]
    fn desired_update_promotes_github_only_and_preserves_unrelated_metadata() {
        let channel = uuid::Uuid::new_v4().to_string();
        let owner = "a".repeat(64);
        let existing = signed_repo(
            vec![
                tag(&["d", "demo"]),
                tag(&["clone", "https://github.com/example/demo.git"]),
                tag(&["buzz-channel", &uuid::Uuid::new_v4().to_string()]),
                tag(&["buzz-protect", "refs/heads/main", "push:member"]),
                tag(&["buzz-protect", "refs/tags/*", "no-delete"]),
                tag(&["future-metadata", "preserve-me"]),
                tag(&["auth", &"b".repeat(64), "kind=30617", &"c".repeat(128)]),
            ],
            "repository content",
            50,
        );
        let RepoAnnouncementPlan::Publish(builder) = plan_repo_announcement(
            Some(&existing),
            "demo",
            &owner,
            "https://relay.example",
            None,
            None,
            &[],
            None,
            &[],
            &channel,
        )
        .expect("plan update") else {
            panic!("legacy repository needs an update")
        };
        let event = builder
            .sign_with_keys(&Keys::generate())
            .expect("sign update");

        assert_eq!(event.content, "repository content");
        assert_eq!(event.created_at.as_secs(), 51);
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["future-metadata", "preserve-me"]));
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["buzz-protect", "refs/tags/*", "no-delete"]));
        assert_eq!(
            event
                .tags
                .iter()
                .find(|tag| super::has_tag_name(tag, "clone"))
                .expect("clone")
                .as_slice(),
            [
                "clone",
                &format!("https://relay.example/git/{owner}/demo"),
                "https://github.com/example/demo.git"
            ]
        );
    }

    #[test]
    fn semantically_exact_desired_state_replays_the_exact_fetched_event() {
        let channel = uuid::Uuid::new_v4().to_string();
        let owner = "a".repeat(64);
        let RepoAnnouncementPlan::Publish(builder) = plan_repo_announcement(
            None,
            "demo",
            &owner,
            "https://relay.example",
            Some("Demo"),
            None,
            &[],
            None,
            &[],
            &channel,
        )
        .expect("initial plan") else {
            panic!("initial plan must publish")
        };
        let canonical = builder
            .sign_with_keys(&Keys::generate())
            .expect("sign initial state");
        let mut stored_tags: Vec<Tag> = canonical.tags.iter().cloned().collect();
        stored_tags.push(tag(&[
            "auth",
            &"b".repeat(64),
            "kind=30617",
            &"c".repeat(128),
        ]));
        let existing = signed_repo(
            stored_tags,
            &canonical.content,
            canonical.created_at.as_secs(),
        );

        let RepoAnnouncementPlan::Replay(replayed) = plan_repo_announcement(
            Some(&existing),
            "demo",
            &owner,
            "https://relay.example",
            Some("Demo"),
            None,
            &[],
            None,
            &[],
            &channel,
        )
        .expect("rerun plan") else {
            panic!("semantically current state must reconcile by replay")
        };
        assert_eq!(replayed, existing);
    }

    #[test]
    fn desired_state_rejects_non_github_secondary_clone() {
        let error = plan_repo_announcement(
            None,
            "demo",
            &"a".repeat(64),
            "https://relay.example",
            None,
            None,
            &["https://gitlab.com/example/demo.git".to_string()],
            None,
            &[],
            &uuid::Uuid::new_v4().to_string(),
        )
        .err()
        .expect("non-GitHub secondary clone must fail");
        assert!(matches!(error, crate::error::CliError::Usage(_)));

        let malformed = plan_repo_announcement(
            None,
            "demo",
            &"a".repeat(64),
            "https://relay.example",
            None,
            None,
            &["not a clone URL".to_string()],
            None,
            &[],
            &uuid::Uuid::new_v4().to_string(),
        )
        .err()
        .expect("malformed secondary clone must fail");
        assert!(matches!(malformed, crate::error::CliError::Usage(_)));

        for unsafe_clone in [
            "https://token@github.com/example/demo.git",
            "https://github.com/example/demo.git?ref=main",
            "http://github.com/example/demo.git",
        ] {
            let error = plan_repo_announcement(
                None,
                "demo",
                &"a".repeat(64),
                "https://relay.example",
                None,
                None,
                &[unsafe_clone.to_string()],
                None,
                &[],
                &uuid::Uuid::new_v4().to_string(),
            )
            .err()
            .expect("unsafe GitHub clone must fail");
            assert!(matches!(error, crate::error::CliError::Usage(_)));
        }
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

    #[test]
    fn create_replay_requires_the_explicit_reconciled_success() {
        let output = validate_create_replay_response(
            r#"{"event_id":"abc","accepted":true,"message":"reconciled: repository provisioning ready","extra":"ignored"}"#,
        )
        .expect("explicit reconciliation success");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output).expect("normalized JSON"),
            serde_json::json!({
                "event_id": "abc",
                "accepted": true,
                "message": "reconciled: repository provisioning ready",
            })
        );

        let duplicate = validate_create_replay_response(
            r#"{"event_id":"abc","accepted":true,"message":"duplicate: already stored"}"#,
        )
        .expect_err("ordinary duplicate remains a conflict");
        assert!(matches!(duplicate, crate::error::CliError::Conflict(_)));

        for raw in [
            r#"{"event_id":"abc","accepted":true,"message":"saved"}"#,
            r#"{"event_id":"abc","accepted":false,"message":"reconciled: repository provisioning ready"}"#,
            "not json",
        ] {
            assert!(
                validate_create_replay_response(raw).is_err(),
                "unexpected replay acceptance: {raw}"
            );
        }
    }

    #[test]
    fn rejected_or_malformed_write_response_fails_closed() {
        assert!(validate_write_response(
            r#"{"event_id":"abc","accepted":false,"message":"denied"}"#
        )
        .is_err());
        assert!(validate_write_response("not json").is_err());
    }
}
