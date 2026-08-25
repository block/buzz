//! Resolve the repository that belongs to a project home channel.
//!
//! Channel-first projects bind a default `kind:30617` at create time. Creating
//! a task in that channel still has to land on *this* project, so the CLI finds
//! (or creates) a `kind:30617` bound to the same `buzz-channel` rather than
//! asking the caller to invent a second project.

use buzz_core::kind::KIND_GIT_REPO_ANNOUNCEMENT;
use nostr::Event;

use crate::client::BuzzClient;
use crate::commands::projects::{
    fetch_projects_for_channel, try_add_own_repo_to_channel_project, verify_default_repo_write,
    PROJECT_QUERY_EVENT_BOUND,
};
use crate::error::CliError;
use crate::validate::{validate_repo_id, validate_uuid};

pub struct ChannelProjectRepo {
    pub repo_owner: String,
    pub repo_id: String,
}

/// Find this channel's project repository, creating one when the project has none.
pub async fn resolve_or_ensure_repo_for_channel(
    client: &BuzzClient,
    channel: &str,
) -> Result<ChannelProjectRepo, CliError> {
    validate_uuid(channel)?;
    let projects = fetch_projects_for_channel(client, channel).await?;
    let repos = fetch_channel_repos(client, channel).await?;
    let project = pick_authoritative_project(&projects, &repos, channel)?;
    if let Some((_, repo)) = project {
        return Ok(repo);
    }

    let caller = client.keys().public_key().to_hex();
    if let Some(repo) = repos.iter().find_map(|event| {
        event
            .pubkey
            .to_hex()
            .eq_ignore_ascii_case(&caller)
            .then(|| repo_from_announcement(event, channel))
            .flatten()
    }) {
        let _ = try_add_own_repo_to_channel_project(client, channel, &repo.repo_id).await;
        return Ok(repo);
    }

    let Some(event) = projects.iter().find(|event| {
        event.pubkey.to_hex().eq_ignore_ascii_case(&caller) && !project_is_unlisted(event)
    }) else {
        return Err(CliError::Usage(
            "this channel is not a project home; pass --repo-owner and --repo-id".into(),
        ));
    };
    ensure_default_repo(client, channel, event).await
}

fn project_is_unlisted(event: &Event) -> bool {
    event.tags.iter().any(|tag| {
        matches!(tag.as_slice(), [name, value, ..] if name == "buzz-visibility" && value == "unlisted")
    })
}

fn project_dtag(event: &Event) -> Option<String> {
    first_tag_value(event, "d").map(String::from)
}

fn project_name(event: &Event) -> Option<String> {
    first_tag_value(event, "name").map(String::from)
}

fn first_tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| match tag.as_slice() {
        [tag_name, value, ..] if tag_name == name && !value.is_empty() => Some(value.as_str()),
        _ => None,
    })
}

fn project_member_repos(event: &Event) -> impl Iterator<Item = ChannelProjectRepo> + '_ {
    event.tags.iter().filter_map(|tag| match tag.as_slice() {
        [name, value, ..] if name == "a" => parse_repo_a_tag(value),
        _ => None,
    })
}

fn repo_authorizes_project(repo: &Event, project: &Event) -> bool {
    let signer = project.pubkey.to_hex();
    repo.pubkey.to_hex().eq_ignore_ascii_case(&signer)
        || repo.tags.iter().any(|tag| {
            tag.as_slice().first().map(String::as_str) == Some("maintainers")
                && tag.as_slice()[1..]
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(&signer))
        })
}

fn repo_from_announcement(event: &Event, channel: &str) -> Option<ChannelProjectRepo> {
    if event.kind.as_u16() != KIND_GIT_REPO_ANNOUNCEMENT as u16
        || repo_is_unlisted(event)
        || first_tag_value(event, "buzz-channel") != Some(channel)
    {
        return None;
    }
    Some(ChannelProjectRepo {
        repo_owner: event.pubkey.to_hex(),
        repo_id: first_tag_value(event, "d")?.to_string(),
    })
}

fn pick_authoritative_project<'a>(
    projects: &'a [Event],
    repos: &'a [Event],
    channel: &str,
) -> Result<Option<(&'a Event, ChannelProjectRepo)>, CliError> {
    let mut matches = projects.iter().filter_map(|project| {
        if project_is_unlisted(project) {
            return None;
        }
        project_member_repos(project).find_map(|member| {
            repos.iter().find_map(|repo| {
                let bound = repo_from_announcement(repo, channel)?;
                (bound.repo_owner.eq_ignore_ascii_case(&member.repo_owner)
                    && bound.repo_id == member.repo_id
                    && repo_authorizes_project(repo, project))
                .then_some((project, bound))
            })
        })
    });
    let selected = matches.next();
    if matches.next().is_some() {
        return Err(CliError::Conflict(format!(
            "channel {channel} has multiple authoritative projects; pass --repo-owner and --repo-id"
        )));
    }
    Ok(selected)
}

pub(crate) fn parse_repo_a_tag(value: &str) -> Option<ChannelProjectRepo> {
    let mut parts = value.splitn(3, ':');
    let kind = parts.next()?;
    let owner = parts.next()?.trim();
    let id = parts.next()?.trim();
    if kind != "30617" || owner.len() != 64 || id.is_empty() {
        return None;
    }
    Some(ChannelProjectRepo {
        repo_owner: owner.to_ascii_lowercase(),
        repo_id: id.to_string(),
    })
}

fn repo_is_unlisted(event: &Event) -> bool {
    event.tags.iter().any(|tag| {
        matches!(
            tag.as_slice(),
            [name, value, ..] if name == "buzz-visibility" && value == "unlisted"
        )
    })
}

async fn fetch_channel_repos(client: &BuzzClient, channel: &str) -> Result<Vec<Event>, CliError> {
    let filter = serde_json::json!({
        "kinds": [KIND_GIT_REPO_ANNOUNCEMENT],
        "#buzz-channel": [channel],
    });
    client
        .query_all_bounded(filter, PROJECT_QUERY_EVENT_BOUND)
        .await?
        .into_iter()
        .map(|event| {
            serde_json::from_value(event).map_err(|error| {
                CliError::Other(format!("failed to parse relay response: {error}"))
            })
        })
        .collect()
}

pub(crate) fn require_repo_channel_binding(event: &Event, channel: &str) -> Result<(), CliError> {
    match first_tag_value(event, "buzz-channel") {
        Some(bound) if bound == channel => Ok(()),
        Some(bound) => Err(CliError::Conflict(format!(
            "repository {:?} is already bound to channel {bound}; pass --repo-owner and --repo-id",
            first_tag_value(event, "d").unwrap_or("<unknown>")
        ))),
        None => Err(CliError::Conflict(format!(
            "repository {:?} has no channel binding; bind it to {channel} or pass --repo-owner and --repo-id",
            first_tag_value(event, "d").unwrap_or("<unknown>")
        ))),
    }
}

async fn ensure_default_repo(
    client: &BuzzClient,
    channel: &str,
    project: &Event,
) -> Result<ChannelProjectRepo, CliError> {
    let slug = project_dtag(project)
        .ok_or_else(|| CliError::Other("project announcement is missing its d tag".into()))?;
    let repo_id = repo_id_from_project_slug(&slug)?;
    let name = project_name(project).unwrap_or_else(|| slug.clone());
    let name = truncate_repo_name(&name);
    let caller = client.keys().public_key().to_hex();

    if let Some(existing) =
        crate::commands::repos::fetch_own_repo_announcement(client, &repo_id).await?
    {
        require_repo_channel_binding(&existing, channel)?;
        let _ = try_add_own_repo_to_channel_project(client, channel, &repo_id).await;
        return Ok(ChannelProjectRepo {
            repo_owner: existing.pubkey.to_hex(),
            repo_id,
        });
    }

    let builder = crate::commands::repos::build_create_announcement(
        &repo_id,
        Some(&name),
        None,
        &[],
        None,
        &[],
        Some(channel),
    )?;
    let event = client.sign_event(builder)?;
    let raw = client.submit_event(event).await?;
    let winner = crate::commands::repos::fetch_own_repo_announcement(client, &repo_id).await?;
    verify_default_repo_write(&raw, winner.as_ref(), channel)?;
    let _ = try_add_own_repo_to_channel_project(client, channel, &repo_id).await;
    Ok(ChannelProjectRepo {
        repo_owner: caller,
        repo_id,
    })
}

pub(crate) fn repo_id_from_project_slug(slug: &str) -> Result<String, CliError> {
    if validate_repo_id(slug).is_ok() {
        return Ok(slug.to_string());
    }
    let mut out = String::new();
    for ch in slug.chars() {
        if out.len() >= 64 {
            break;
        }
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
            out.push(ch);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.starts_with('.') {
        out.remove(0);
    }
    if out.ends_with('-') {
        out.pop();
    }
    validate_repo_id(&out)?;
    Ok(out)
}

pub(crate) fn truncate_repo_name(name: &str) -> String {
    if name.len() <= 128 {
        return name.to_string();
    }
    let end = name
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= 128)
        .last()
        .unwrap_or(0);
    name[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_id_from_project_slug_keeps_valid_ids() {
        assert_eq!(
            repo_id_from_project_slug("space-invaders-3d").unwrap(),
            "space-invaders-3d"
        );
    }

    #[test]
    fn repo_id_from_project_slug_sanitizes_invalid_characters() {
        assert_eq!(
            repo_id_from_project_slug("Space Invaders 3D!").unwrap(),
            "Space-Invaders-3D"
        );
    }

    fn signed_event(keys: &nostr::Keys, kind: u16, tags: Vec<nostr::Tag>) -> Event {
        nostr::EventBuilder::new(nostr::Kind::Custom(kind), "")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    fn tag(parts: &[&str]) -> nostr::Tag {
        nostr::Tag::parse(parts.iter().copied()).unwrap()
    }

    #[test]
    fn truncate_repo_name_respects_utf8_byte_limit() {
        let name = "界".repeat(100);
        let truncated = truncate_repo_name(&name);
        assert_eq!(truncated, "界".repeat(42));
        assert_eq!(truncated.len(), 126);
        assert!(truncated.len() <= 128);
    }

    #[test]
    fn authoritative_project_requires_repo_owner_consent() {
        let owner = nostr::Keys::generate();
        let attacker = nostr::Keys::generate();
        let channel = "11111111-1111-4111-8111-111111111111";
        let owner_hex = owner.public_key().to_hex();
        let repo_coord = format!("30617:{owner_hex}:game");
        let repo = signed_event(
            &owner,
            30617,
            vec![tag(&["d", "game"]), tag(&["buzz-channel", channel])],
        );
        let hostile = signed_event(
            &attacker,
            30621,
            vec![
                tag(&["d", "spoof"]),
                tag(&["buzz-channel", channel]),
                tag(&["a", &repo_coord]),
            ],
        );
        assert!(pick_authoritative_project(&[hostile], &[repo], channel)
            .unwrap()
            .is_none());
    }

    #[test]
    fn authorized_project_selects_channel_bound_member() {
        let owner = nostr::Keys::generate();
        let channel = "11111111-1111-4111-8111-111111111111";
        let owner_hex = owner.public_key().to_hex();
        let repo_coord = format!("30617:{owner_hex}:game");
        let repo = signed_event(
            &owner,
            30617,
            vec![tag(&["d", "game"]), tag(&["buzz-channel", channel])],
        );
        let project = signed_event(
            &owner,
            30621,
            vec![
                tag(&["d", "game"]),
                tag(&["buzz-channel", channel]),
                tag(&["a", &repo_coord]),
            ],
        );
        let (_, selected) = pick_authoritative_project(&[project], &[repo], channel)
            .unwrap()
            .unwrap();
        assert_eq!(selected.repo_owner, owner_hex);
        assert_eq!(selected.repo_id, "game");
    }

    #[test]
    fn ambiguous_authoritative_projects_fail_closed() {
        let owner = nostr::Keys::generate();
        let channel = "11111111-1111-4111-8111-111111111111";
        let owner_hex = owner.public_key().to_hex();
        let repo_coord = format!("30617:{owner_hex}:game");
        let repo = signed_event(
            &owner,
            30617,
            vec![tag(&["d", "game"]), tag(&["buzz-channel", channel])],
        );
        let projects = ["one", "two"].map(|slug| {
            signed_event(
                &owner,
                30621,
                vec![
                    tag(&["d", slug]),
                    tag(&["buzz-channel", channel]),
                    tag(&["a", &repo_coord]),
                ],
            )
        });
        assert!(matches!(
            pick_authoritative_project(&projects, &[repo], channel),
            Err(CliError::Conflict(_))
        ));
    }

    #[test]
    fn existing_repo_must_bind_requested_channel() {
        let owner = nostr::Keys::generate();
        let requested = "11111111-1111-4111-8111-111111111111";
        let other = "22222222-2222-4222-8222-222222222222";
        let matching = signed_event(
            &owner,
            30617,
            vec![tag(&["d", "game"]), tag(&["buzz-channel", requested])],
        );
        assert!(require_repo_channel_binding(&matching, requested).is_ok());

        let foreign = signed_event(
            &owner,
            30617,
            vec![tag(&["d", "game"]), tag(&["buzz-channel", other])],
        );
        assert!(matches!(
            require_repo_channel_binding(&foreign, requested),
            Err(CliError::Conflict(_))
        ));

        let unbound = signed_event(&owner, 30617, vec![tag(&["d", "game"])]);
        assert!(matches!(
            require_repo_channel_binding(&unbound, requested),
            Err(CliError::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn ensure_default_repo_rejects_dominated_foreign_winning_head() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let requested = "11111111-1111-4111-8111-111111111111";
        let foreign = "22222222-2222-4222-8222-222222222222";
        let keys = nostr::Keys::generate();
        let project = signed_event(
            &keys,
            buzz_core::kind::KIND_PROJECT as u16,
            vec![tag(&["d", "game"]), tag(&["buzz-channel", requested])],
        );
        let winner = crate::commands::repos::build_create_announcement(
            "game",
            Some("game"),
            None,
            &[],
            None,
            &[],
            Some(foreign),
        )
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0; 65_536];
                let read = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..read]);
                let index = server_requests.fetch_add(1, Ordering::SeqCst);
                let body = match index {
                    0 => "[]".to_string(),
                    1 if request.starts_with("POST /events ") => serde_json::json!({
                        "accepted": true, "message": "duplicate"
                    })
                    .to_string(),
                    2 => serde_json::json!([winner]).to_string(),
                    _ => panic!("unexpected request {index}: {request}"),
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = crate::client::BuzzClient::new(base_url, keys, None, None).unwrap();

        let result = ensure_default_repo(&client, requested, &project).await;

        assert!(matches!(result, Err(CliError::Conflict(_))));
        assert_eq!(
            requests.load(Ordering::SeqCst),
            3,
            "verification must fail before trying to update the project"
        );
        server.abort();
    }

    #[test]
    fn later_maintainer_value_authorizes_project() {
        let owner = nostr::Keys::generate();
        let maintainer = nostr::Keys::generate();
        let unrelated = nostr::Keys::generate().public_key().to_hex();
        let channel = "11111111-1111-4111-8111-111111111111";
        let owner_hex = owner.public_key().to_hex();
        let maintainer_hex = maintainer.public_key().to_hex();
        let repo_coord = format!("30617:{owner_hex}:game");
        let repo = signed_event(
            &owner,
            30617,
            vec![
                tag(&["d", "game"]),
                tag(&["buzz-channel", channel]),
                tag(&["maintainers", &unrelated, &maintainer_hex]),
            ],
        );
        let project = signed_event(
            &maintainer,
            30621,
            vec![
                tag(&["d", "suite"]),
                tag(&["buzz-channel", channel]),
                tag(&["a", &repo_coord]),
            ],
        );
        assert!(pick_authoritative_project(&[project], &[repo], channel)
            .unwrap()
            .is_some());
    }

    #[test]
    fn parse_repo_a_tag_reads_nip34_coordinate() {
        let owner = "a".repeat(64);
        let parsed = parse_repo_a_tag(&format!("30617:{owner}:game")).unwrap();
        assert_eq!(parsed.repo_owner, owner);
        assert_eq!(parsed.repo_id, "game");
    }
}
