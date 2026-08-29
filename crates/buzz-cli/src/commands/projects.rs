//! `buzz projects` commands — NIP-MP kind:30621 write path.
//!
//! All mutations follow a read-modify-write pattern:
//!   1. Fetch the caller's own live head via `kinds:[30621] + authors:[self] + #d:[slug]`.
//!   2. Mutate the tag set (strip `auth`, apply change).
//!   3. Re-validate the full envelope through Layer A before submitting.
//!   4. Set `created_at = max(client_now, head.created_at + 1)` so the
//!      replacement dominates the observed head and uses wall clock for
//!      ordinary stale heads. Unusually future heads may still hit the relay's
//!      timestamp-drift guard until time advances.
//!
//! Limitations recorded in this phase:
//!   - Relay hints are read-preserved but not authored (`--repo` carries
//!     a coordinate only; existing hinted tags survive RMW unchanged).
//!   - `delete` targets signer-self only (NIP-OA owner-delete path deferred).
//!   - Deletion durability against later arrival (watermark follow-up) is
//!     not in scope.

use buzz_core::kind::KIND_PROJECT;
use buzz_sdk::{
    build_delete_addressable, build_project, build_project_with_tags, ProjectMemberCoord,
    PROJECT_D_MAX_LEN,
};
use nostr::{Event, EventBuilder, PublicKey, Tag, Timestamp};

use crate::agent_management::{build_project_channel, CreateProjectChannelDraft};
use crate::client::BuzzClient;
use crate::commands::parse_write_response;
use crate::commands::project_channel::{
    repo_id_from_project_slug, require_repo_channel_binding, truncate_repo_name,
};
use crate::commands::repos::{build_create_announcement, fetch_own_repo_announcement};
use crate::error::CliError;

async fn cmd_add_channel_draft(
    client: &BuzzClient,
    home_channel: String,
    name: String,
    description: Option<String>,
    visibility: String,
    ttl_seconds: Option<u64>,
    template_name: Option<String>,
) -> Result<(), CliError> {
    let owner_hex = client
        .auth_tag_owner_hex()
        .ok_or_else(|| CliError::Auth("project channel requests require BUZZ_AUTH_TAG".into()))?;
    let owner = PublicKey::parse(&owner_hex)
        .map_err(|error| CliError::Auth(format!("invalid owner attestation: {error}")))?;
    let built = build_project_channel(
        client.keys(),
        &owner,
        CreateProjectChannelDraft {
            home_channel_id: home_channel,
            name,
            description,
            visibility,
            ttl_seconds,
            template_name,
        },
    )?;
    let response = client.publish_ephemeral_event(built.event).await?;
    let mut output: serde_json::Value = serde_json::from_str(&response)
        .map_err(|error| CliError::Other(format!("invalid relay response: {error}")))?;
    if let Some(object) = output.as_object_mut() {
        object.insert("request_id".into(), built.request_id.into());
        object.insert("action".into(), "add-channel".into());
        object.insert("saved".into(), false.into());
        object.insert(
            "message".into(),
            "Project channel draft sent to Buzz Desktop for owner review. The channel is not created until the owner approves it."
                .into(),
        );
    }
    println!("{output}");
    Ok(())
}

// ── Buzz repo-ID grammar (bare --repo shorthand) ─────────────────────────────

/// Pattern for a Buzz-hosted repo identifier (bare `--repo` shorthand).
/// `[a-zA-Z0-9._-]{1,64}` — no colons, so guaranteed collision-free with
/// `30617:<owner>:<d>` full coordinates.
fn is_bare_repo_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// Expand a CLI `--repo` argument into a full `30617:<owner>:<d>` coordinate.
///
/// Bare form (`[a-zA-Z0-9._-]{1,64}`): owner defaults to the caller's pubkey.
/// Full form (`30617:<owner-hex>:<d>`): used verbatim.
fn expand_repo_coord(s: &str, caller_pubkey: &str) -> Result<ProjectMemberCoord, CliError> {
    if is_bare_repo_id(s) {
        // Bare form: expand to full coordinate with caller as owner.
        let full = format!("30617:{caller_pubkey}:{s}");
        ProjectMemberCoord::parse_full(&full)
            .map_err(|e| CliError::Usage(format!("invalid repo coordinate: {e}")))
    } else {
        // Full form: must be parseable as a complete coordinate.
        ProjectMemberCoord::parse_full(s)
            .map_err(|e| CliError::Usage(format!("invalid repo coordinate: {e}")))
    }
}

// ── Head-fetch helper ─────────────────────────────────────────────────────────

fn parse_events(json: &str) -> Result<Vec<Event>, CliError> {
    serde_json::from_str(json)
        .map_err(|e| CliError::Other(format!("failed to parse relay response: {e}")))
}

/// Fetch listed kind:30621 heads whose `buzz-channel` is `channel`.
fn project_tags_match_channel<'a>(tags: impl IntoIterator<Item = &'a Tag>, channel: &str) -> bool {
    tags.into_iter()
        .any(|tag| tag_name(tag) == Some("buzz-channel") && tag_value(tag) == Some(channel))
}

pub(crate) const PROJECT_QUERY_EVENT_BOUND: u32 = 10_000;

pub(crate) async fn fetch_projects_for_channel(
    client: &BuzzClient,
    channel: &str,
) -> Result<Vec<Event>, CliError> {
    fetch_projects_for_channel_bounded(client, channel, PROJECT_QUERY_EVENT_BOUND).await
}

async fn fetch_projects_for_channel_bounded(
    client: &BuzzClient,
    channel: &str,
    max_events: u32,
) -> Result<Vec<Event>, CliError> {
    let filter = serde_json::json!({
        "kinds": [KIND_PROJECT],
        "#buzz-channel": [channel],
    });
    let events: Vec<Event> = client
        .query_all_bounded(filter, max_events)
        .await?
        .into_iter()
        .map(|event| {
            serde_json::from_value(event)
                .map_err(|e| CliError::Other(format!("failed to parse relay response: {e}")))
        })
        .collect::<Result<_, _>>()?;
    Ok(events
        .into_iter()
        .filter(|event| project_tags_match_channel(event.tags.iter(), channel))
        .collect())
}

fn project_is_unlisted(event: &Event) -> bool {
    event.tags.iter().any(|tag| {
        matches!(
            tag.as_slice(),
            [name, value, ..] if name == "buzz-visibility" && value == "unlisted"
        )
    })
}

fn project_slug(event: &Event) -> Option<String> {
    event.tags.iter().find_map(|tag| match tag.as_slice() {
        [name, value, ..] if name == "d" && !value.is_empty() => Some(value.clone()),
        _ => None,
    })
}

/// Add repos to a project the caller owns. Returns the relay write JSON.
pub async fn add_repos_to_own_project(
    client: &BuzzClient,
    slug: &str,
    repos: &[String],
) -> Result<String, CliError> {
    validate_project_slug(slug)?;
    let caller_pubkey = client.keys().public_key().to_hex();

    let new_members: Vec<ProjectMemberCoord> = repos
        .iter()
        .map(|r| expand_repo_coord(r, &caller_pubkey))
        .collect::<Result<Vec<_>, _>>()?;

    let mut seen = std::collections::HashSet::new();
    for m in &new_members {
        if !seen.insert(m.coord.clone()) {
            return Err(CliError::Usage(format!(
                "duplicate --repo coordinate in this invocation: {:?}",
                m.coord
            )));
        }
    }

    let head = fetch_own_project(client, slug)
        .await?
        .ok_or_else(|| CliError::NotFound(format!("project {slug:?} not found")))?;
    let next_ts = next_timestamp(&head, Timestamp::now())?;

    let mut tags: Vec<Tag> = head.tags.iter().cloned().collect();
    let existing_coords: std::collections::HashSet<String> = head
        .tags
        .iter()
        .filter(|t| tag_name(t) == Some("a"))
        .filter_map(|t| tag_value(t).map(String::from))
        .collect();
    let mut added = 0usize;
    for m in &new_members {
        if !existing_coords.contains(m.coord.as_str()) {
            let parts = m.to_tag_parts();
            let parts_ref: Vec<&str> = parts.iter().map(String::as_str).collect();
            tags.push(
                Tag::parse(parts_ref.iter().copied())
                    .map_err(|e| CliError::Other(format!("member tag construction failed: {e}")))?,
            );
            added += 1;
        }
    }

    if added == 0 {
        return Err(CliError::Conflict(format!(
            "all requested repositories are already members of project {slug:?}"
        )));
    }

    let builder = rebuild_project(&head.content, tags, next_ts)?;
    let event = client.sign_event(builder)?;
    client.submit_event(event).await
}

/// If this channel is already a project the caller owns, attach `repo_id`.
pub async fn try_add_own_repo_to_channel_project(
    client: &BuzzClient,
    channel: &str,
    repo_id: &str,
) -> Result<(), CliError> {
    let projects = fetch_projects_for_channel(client, channel).await?;
    let caller = client.keys().public_key().to_hex();
    let Some(event) = projects.iter().find(|candidate| {
        candidate.pubkey.to_hex().eq_ignore_ascii_case(&caller) && !project_is_unlisted(candidate)
    }) else {
        return Ok(());
    };
    let Some(slug) = project_slug(event) else {
        return Ok(());
    };
    match add_repos_to_own_project(client, &slug, &[repo_id.to_string()]).await {
        Ok(_) | Err(CliError::Conflict(_)) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Fetch a project head by slug and optional owner pubkey.
async fn fetch_project(
    client: &BuzzClient,
    slug: &str,
    owner: Option<&str>,
) -> Result<Option<Event>, CliError> {
    let pubkey = match owner {
        Some(pk) => {
            crate::validate::validate_hex64(pk)?;
            pk.to_string()
        }
        None => client.keys().public_key().to_hex(),
    };
    let filter = serde_json::json!({
        "kinds": [KIND_PROJECT],
        "authors": [pubkey],
        "#d": [slug],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let mut events = parse_events(&raw)?;
    events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
    Ok(events.into_iter().next())
}

/// Fetch the caller's own live kind:30621 head for `slug`.
async fn fetch_own_project(client: &BuzzClient, slug: &str) -> Result<Option<Event>, CliError> {
    fetch_project(client, slug, None).await
}

// ── Tag helpers ───────────────────────────────────────────────────────────────

fn tag_name(tag: &Tag) -> Option<&str> {
    tag.as_slice().first().map(String::as_str)
}

fn tag_value(tag: &Tag) -> Option<&str> {
    tag.as_slice().get(1).map(String::as_str)
}

fn make_tag(parts: &[&str]) -> Result<Tag, CliError> {
    Tag::parse(parts.iter().copied())
        .map_err(|e| CliError::Other(format!("tag construction failed: {e}")))
}

// ── Submit helper ─────────────────────────────────────────────────────────────

/// Submit a project event and print the relay's write response.
///
/// `link_slug` carries the project's d-tag on creates whose slug fits the
/// `buzz://` link charset; the response then also carries a `link` field,
/// which renders as a rich preview card in Buzz Desktop when included in a
/// chat message — agents announce projects with it (see base_prompt.md).
async fn submit_project(
    client: &BuzzClient,
    builder: EventBuilder,
    link_slug: Option<&str>,
) -> Result<(), CliError> {
    let event = client.sign_event(builder)?;
    let owner = event.pubkey.to_hex();
    let raw = client.submit_event(event).await?;
    let response = parse_write_response(&raw, "project changed concurrently; retry")?;
    match link_slug {
        Some(slug) => crate::client::print_create_response(
            &response,
            "link",
            &crate::links::project_link(&owner, slug),
        ),
        None => println!("{response}"),
    }
    Ok(())
}

// ── Build helpers ─────────────────────────────────────────────────────────────

/// Choose the later of client wall clock and the instant after the observed head.
///
/// The relay remains authoritative for timestamp drift: a sufficiently future
/// head can require a timestamp that the relay will temporarily reject.
fn next_timestamp(head: &Event, now: Timestamp) -> Result<Timestamp, CliError> {
    let after_head = head
        .created_at
        .as_secs()
        .checked_add(1)
        .ok_or_else(|| CliError::Other("project timestamp cannot be advanced".into()))?;
    Ok(Timestamp::from(after_head.max(now.as_secs())))
}

/// Strip `auth` from a tag list and pass the resulting envelope through
/// Layer A validation. Returns a validated `EventBuilder` at `next_ts`.
fn rebuild_project(
    content: &str,
    tags: Vec<Tag>,
    next_ts: Timestamp,
) -> Result<EventBuilder, CliError> {
    // Strip auth tags.
    let clean_tags: Vec<Tag> = tags
        .into_iter()
        .filter(|t| tag_name(t) != Some("auth"))
        .collect();

    build_project_with_tags(content, clean_tags)
        .map_err(|e| CliError::Other(format!("envelope validation failed: {e}")))
        .map(|b| b.custom_created_at(next_ts))
}

// ── Command implementations ───────────────────────────────────────────────────

/// `buzz projects create`
pub async fn cmd_create(
    client: &BuzzClient,
    slug: &str,
    repos: &[String],
    name: Option<&str>,
    description: Option<&str>,
    channel: Option<&str>,
    visibility: Option<&str>,
) -> Result<(), CliError> {
    // ── Local validation (all checks before any .await) ───────────────────
    validate_project_slug(slug)?;

    let caller_pubkey = client.keys().public_key().to_hex();

    // Expand and validate repo coordinates.
    let mut members: Vec<ProjectMemberCoord> = repos
        .iter()
        .map(|r| expand_repo_coord(r, &caller_pubkey))
        .collect::<Result<Vec<_>, _>>()?;

    if members.is_empty() && channel.is_none() {
        return Err(CliError::Usage(
            "pass --channel to create a default repository, or --repo to attach an existing one"
                .into(),
        ));
    }

    // Dedupe: preserve first occurrence, reject duplicates with Usage.
    let mut seen = std::collections::HashSet::new();
    for m in &members {
        if !seen.insert(m.coord.clone()) {
            return Err(CliError::Usage(format!(
                "duplicate --repo coordinate in this invocation: {:?}",
                m.coord
            )));
        }
    }

    // Validate optional metadata (early, before any network call).
    if let Some(ch) = channel {
        crate::validate::validate_uuid(ch)?;
    }
    if let Some(vis) = visibility {
        validate_visibility(vis)?;
    }
    if let Some(n) = name {
        if n.len() > 256 {
            return Err(CliError::Usage(format!(
                "project name must not exceed 256 bytes (got {})",
                n.len()
            )));
        }
    }

    // ── Network: collision preflight ──────────────────────────────────────
    if fetch_own_project(client, slug).await?.is_some() {
        return Err(CliError::Conflict(format!(
            "project {slug:?} already exists; use 'buzz projects update' to modify it"
        )));
    }
    if let Some(channel) = channel {
        if let Some(existing) = fetch_projects_for_channel(client, channel)
            .await?
            .into_iter()
            .find(|event| {
                event.pubkey.to_hex().eq_ignore_ascii_case(&caller_pubkey)
                    && !project_is_unlisted(event)
            })
        {
            let existing_slug = project_slug(&existing).unwrap_or_else(|| slug.to_string());
            return Err(CliError::Conflict(format!(
                "you already own project {existing_slug:?} for channel {channel}; update that project instead"
            )));
        }
    }

    if members.is_empty() {
        let home = channel.ok_or_else(|| {
            CliError::Usage(
                "pass --channel to create a default repository, or --repo to attach an existing one"
                    .into(),
            )
        })?;
        let repo_id = ensure_default_create_repo(client, slug, name, description, home).await?;
        members.push(expand_repo_coord(&repo_id, &caller_pubkey)?);
    }

    // ── Build via Layer B (enforces all writer policy) ────────────────────
    let builder = build_project(slug, name, description, &members, channel, visibility)
        .map_err(|e| CliError::Usage(e.to_string()))?;

    // Slugs wider than the link charset stay linkless rather than emitting a
    // `link` no client can parse.
    submit_project(
        client,
        builder,
        crate::links::is_linkable_dtag(slug).then_some(slug),
    )
    .await
}

/// `buzz projects get`
pub async fn cmd_get(client: &BuzzClient, slug: &str, owner: Option<&str>) -> Result<(), CliError> {
    validate_project_slug(slug)?;
    let resp = match fetch_project(client, slug, owner).await? {
        Some(event) => serde_json::json!({
            "event_id": event.id.to_hex(),
            "pubkey": event.pubkey.to_hex(),
            "created_at": event.created_at.as_secs(),
            "kind": event.kind.as_u16(),
            "tags": event.tags.iter().map(|t| t.as_slice().to_vec()).collect::<Vec<_>>(),
            "content": event.content,
        }),
        None => {
            let owner_desc = owner.unwrap_or("current identity");
            return Err(CliError::NotFound(format!(
                "project {slug:?} not found for {owner_desc}"
            )));
        }
    };
    println!("{resp}");
    Ok(())
}

/// `buzz projects list`
pub async fn cmd_list(
    client: &BuzzClient,
    owner: Option<&str>,
    limit: Option<u32>,
) -> Result<(), CliError> {
    let pubkey = match owner {
        Some(pk) => {
            crate::validate::validate_hex64(pk)?;
            pk.to_string()
        }
        None => client.keys().public_key().to_hex(),
    };
    let mut filter = serde_json::json!({
        "kinds": [KIND_PROJECT],
        "authors": [pubkey],
    });
    if let Some(n) = limit {
        filter["limit"] = serde_json::json!(n);
    }
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

/// `buzz projects add-repo`
pub async fn cmd_add_repo(
    client: &BuzzClient,
    slug: &str,
    repos: &[String],
) -> Result<(), CliError> {
    let raw = add_repos_to_own_project(client, slug, repos).await?;
    let response = parse_write_response(&raw, "project changed concurrently; retry")?;
    println!("{response}");
    Ok(())
}

/// `buzz projects remove-repo`
pub async fn cmd_remove_repo(
    client: &BuzzClient,
    slug: &str,
    repos: &[String],
) -> Result<(), CliError> {
    validate_project_slug(slug)?;
    let caller_pubkey = client.keys().public_key().to_hex();

    // ── Local validation before any .await ────────────────────────────────
    let to_remove: Vec<ProjectMemberCoord> = repos
        .iter()
        .map(|r| expand_repo_coord(r, &caller_pubkey))
        .collect::<Result<Vec<_>, _>>()?;

    // ── Network: fetch head ───────────────────────────────────────────────
    let head = fetch_own_project(client, slug)
        .await?
        .ok_or_else(|| CliError::NotFound(format!("project {slug:?} not found")))?;
    let next_ts = next_timestamp(&head, Timestamp::now())?;

    // Verify all requested repos exist in the project.
    let existing_coords: std::collections::HashSet<String> = head
        .tags
        .iter()
        .filter(|t| tag_name(t) == Some("a"))
        .filter_map(|t| tag_value(t).map(String::from))
        .collect();
    for m in &to_remove {
        if !existing_coords.contains(m.coord.as_str()) {
            return Err(CliError::NotFound(format!(
                "project {slug:?} does not contain member {:?}",
                m.coord
            )));
        }
    }

    let remove_coords: std::collections::HashSet<&str> =
        to_remove.iter().map(|m| m.coord.as_str()).collect();

    // Keep all tags except auth and the removed members.
    let tags: Vec<Tag> = head
        .tags
        .iter()
        .filter(|t| {
            if tag_name(t) == Some("auth") {
                return false;
            }
            if tag_name(t) == Some("a") {
                if let Some(coord) = tag_value(t) {
                    return !remove_coords.contains(coord);
                }
            }
            true
        })
        .cloned()
        .collect();

    // Single rebuild validates the full envelope and strips any remaining auth.
    let builder = rebuild_project(&head.content, tags, next_ts)?;
    submit_project(client, builder, None).await
}

/// `buzz projects update`
///
/// Requires at least one setter or clearer; a no-op call is a usage error.
#[allow(clippy::too_many_arguments)]
pub async fn cmd_update(
    client: &BuzzClient,
    slug: &str,
    name: Option<&str>,
    clear_name: bool,
    description: Option<&str>,
    clear_description: bool,
    channel: Option<&str>,
    clear_channel: bool,
    visibility: Option<&str>,
    clear_visibility: bool,
) -> Result<(), CliError> {
    // Guard: at least one mutation required. The clap `ArgGroup` with
    // `required(true).multiple(true)` enforces this at parse time; this
    // runtime check is a defense-in-depth safety net for callers that invoke
    // `cmd_update` directly (e.g. tests and future programmatic callers).
    let has_mutation = name.is_some()
        || clear_name
        || description.is_some()
        || clear_description
        || channel.is_some()
        || clear_channel
        || visibility.is_some()
        || clear_visibility;
    if !has_mutation {
        return Err(CliError::Usage(
            "buzz projects update requires at least one of: \
             --name, --clear-name, --description, --clear-description, \
             --channel, --clear-channel, --visibility, --clear-visibility"
                .into(),
        ));
    }

    validate_project_slug(slug)?;
    if let Some(ch) = channel {
        crate::validate::validate_uuid(ch)?;
    }
    if let Some(vis) = visibility {
        validate_visibility(vis)?;
    }

    let head = fetch_own_project(client, slug)
        .await?
        .ok_or_else(|| CliError::NotFound(format!("project {slug:?} not found")))?;
    let next_ts = next_timestamp(&head, Timestamp::now())?;

    // Build the new tag set. For each singleton metadata field:
    //   - setter present: replace value (strip old, append new)
    //   - clear flag set: drop the tag
    //   - neither: keep existing
    // Non-singleton / non-metadata tags (d, a, unknown) are preserved as-is.
    let singleton_fields = ["name", "description", "buzz-channel", "buzz-visibility"];
    let mut tags: Vec<Tag> = head
        .tags
        .iter()
        .filter(|t| {
            if tag_name(t) == Some("auth") {
                return false;
            }
            // Drop singletons we're replacing or clearing.
            if let Some(field) = tag_name(t) {
                if singleton_fields.contains(&field) {
                    let clear = match field {
                        "name" => clear_name || name.is_some(),
                        "description" => clear_description || description.is_some(),
                        "buzz-channel" => clear_channel || channel.is_some(),
                        "buzz-visibility" => clear_visibility || visibility.is_some(),
                        _ => false,
                    };
                    return !clear;
                }
            }
            true
        })
        .cloned()
        .collect();

    // Append new singleton values.
    if let Some(n) = name {
        tags.push(make_tag(&["name", n])?);
    }
    if let Some(d) = description {
        tags.push(make_tag(&["description", d])?);
    }
    if let Some(ch) = channel {
        tags.push(make_tag(&["buzz-channel", ch])?);
    }
    if let Some(vis) = visibility {
        tags.push(make_tag(&["buzz-visibility", vis])?);
    }

    let builder = build_project_with_tags(&head.content, tags)
        .map_err(|e| CliError::Other(format!("envelope validation failed: {e}")))?
        .custom_created_at(next_ts);
    submit_project(client, builder, None).await
}

/// `buzz projects delete`
///
/// Head-based and verified:
///   1. Fetch own live head — `NotFound` if absent.
///   2. Build tombstone at `max(client_now, head.created_at + 1)`.
///   3. Submit.
///   4. Re-query the coordinate; if a newer head survived → `Conflict`.
pub async fn cmd_delete(client: &BuzzClient, slug: &str) -> Result<(), CliError> {
    validate_project_slug(slug)?;

    let head = fetch_own_project(client, slug)
        .await?
        .ok_or_else(|| CliError::NotFound(format!("project {slug:?} not found")))?;
    let next_ts = next_timestamp(&head, Timestamp::now())?;

    let pubkey_hex = client.keys().public_key().to_hex();
    let tombstone = build_delete_addressable(KIND_PROJECT, &pubkey_hex, slug)
        .map_err(|e| CliError::Other(format!("failed to build delete event: {e}")))?
        .custom_created_at(next_ts);

    let event = client.sign_event(tombstone)?;
    let raw = client.submit_event(event).await?;
    parse_write_response(&raw, "delete event was dominated; a newer head exists")?;

    // Post-submit verification: re-query to confirm the head is gone.
    if let Some(survivor) = fetch_own_project(client, slug).await? {
        // A newer head survived the tombstone.
        return Err(CliError::Conflict(format!(
            "project {slug:?} still exists (head at {}); a concurrent write raced the delete",
            survivor.created_at.as_secs()
        )));
    }

    println!("{}", serde_json::json!({ "deleted": slug, "status": "ok" }));
    Ok(())
}

async fn ensure_default_create_repo(
    client: &BuzzClient,
    slug: &str,
    name: Option<&str>,
    description: Option<&str>,
    channel: &str,
) -> Result<String, CliError> {
    let repo_id = repo_id_from_project_slug(slug)?;
    if let Some(existing) = fetch_own_repo_announcement(client, &repo_id).await? {
        require_repo_channel_binding(&existing, channel)?;
        return Ok(repo_id);
    }
    let raw_name = name.unwrap_or(slug);
    let display_name = truncate_repo_name(raw_name);
    let builder = build_create_announcement(
        &repo_id,
        Some(&display_name),
        description,
        &[],
        None,
        &[],
        Some(channel),
    )?;
    let event = client.sign_event(builder)?;
    let raw = client.submit_event(event).await?;
    let winner = fetch_own_repo_announcement(client, &repo_id).await?;
    verify_default_repo_write(&raw, winner.as_ref(), channel)?;
    Ok(repo_id)
}

pub(crate) fn verify_default_repo_write(
    raw: &str,
    winner: Option<&Event>,
    channel: &str,
) -> Result<(), CliError> {
    match parse_write_response(
        raw,
        "default repository changed concurrently; checking the winning head",
    ) {
        Ok(_) | Err(CliError::Conflict(_)) => {}
        Err(error) => return Err(error),
    }
    let winner = winner.ok_or_else(|| {
        CliError::Conflict(
            "default repository write was not authoritative; retry project creation".into(),
        )
    })?;
    require_repo_channel_binding(winner, channel)
}

// ── Validation helpers ────────────────────────────────────────────────────────

/// Validate a project slug: non-empty, ≤1024 bytes, verbatim.
/// Does NOT impose the Buzz repo-ID grammar — project slugs are more permissive.
fn validate_project_slug(slug: &str) -> Result<(), CliError> {
    if slug.is_empty() {
        return Err(CliError::Usage("project slug must not be empty".into()));
    }
    if slug.len() > PROJECT_D_MAX_LEN {
        return Err(CliError::Usage(format!(
            "project slug must not exceed {PROJECT_D_MAX_LEN} bytes (got {})",
            slug.len()
        )));
    }
    Ok(())
}

/// Validate a `buzz-visibility` value at the writer level.
fn validate_visibility(vis: &str) -> Result<(), CliError> {
    if vis != "listed" && vis != "unlisted" {
        return Err(CliError::Usage(format!(
            "visibility must be 'listed' or 'unlisted' (got {vis:?})"
        )));
    }
    Ok(())
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

pub async fn dispatch(cmd: crate::ProjectsCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::ProjectsCmd;
    match cmd {
        ProjectsCmd::Create {
            slug,
            repo,
            name,
            description,
            channel,
            visibility,
        } => {
            cmd_create(
                client,
                &slug,
                &repo,
                name.as_deref(),
                description.as_deref(),
                channel.as_deref(),
                visibility.map(|v| v.as_str()),
            )
            .await
        }
        ProjectsCmd::Get { slug, owner } => cmd_get(client, &slug, owner.as_deref()).await,
        ProjectsCmd::List { owner, limit } => cmd_list(client, owner.as_deref(), limit).await,
        ProjectsCmd::AddRepo { slug, repo } => cmd_add_repo(client, &slug, &repo).await,
        ProjectsCmd::AddChannel {
            home_channel,
            name,
            description,
            visibility,
            ttl,
            template,
        } => {
            cmd_add_channel_draft(
                client,
                home_channel,
                name,
                description,
                visibility.to_string(),
                ttl,
                template,
            )
            .await
        }
        ProjectsCmd::RemoveRepo { slug, repo } => cmd_remove_repo(client, &slug, &repo).await,
        ProjectsCmd::Update {
            slug,
            name,
            clear_name,
            description,
            clear_description,
            channel,
            clear_channel,
            visibility,
            clear_visibility,
        } => {
            cmd_update(
                client,
                &slug,
                name.as_deref(),
                clear_name,
                description.as_deref(),
                clear_description,
                channel.as_deref(),
                clear_channel,
                visibility.map(|v| v.as_str()),
                clear_visibility,
            )
            .await
        }
        ProjectsCmd::Delete { slug } => cmd_delete(client, &slug).await,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use buzz_sdk::{validate_project_envelope, PROJECT_MEMBER_CAP};
    use nostr::Tag;

    use super::*;

    async fn run_default_repo_create_race(
        winning_channel: &str,
    ) -> (Result<(), CliError>, Vec<u16>) {
        use std::sync::{Arc, Mutex};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let requested_channel = "11111111-1111-4111-8111-111111111111";
        let keys = nostr::Keys::generate();
        let winner = build_create_announcement(
            "app",
            Some("App"),
            None,
            &[],
            None,
            &[],
            Some(winning_channel),
        )
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
        let winner_json = serde_json::to_value(winner).unwrap();
        let posted_kinds = Arc::new(Mutex::new(Vec::new()));
        let server_kinds = posted_kinds.clone();
        let repo_queries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server_repo_queries = repo_queries.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0; 65_536];
                let read = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..read]);
                let (status, body) = if request.starts_with("POST /query ") {
                    let is_repo_query = request.contains("30617");
                    let repo_query_index = is_repo_query.then(|| {
                        server_repo_queries.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    });
                    if repo_query_index == Some(1) {
                        ("200 OK", serde_json::json!([winner_json]).to_string())
                    } else {
                        ("200 OK", "[]".to_string())
                    }
                } else if request.starts_with("POST /events ") {
                    let json_start = request.find("\r\n\r\n").unwrap() + 4;
                    let event: serde_json::Value =
                        serde_json::from_str(&request[json_start..]).unwrap();
                    let kind = event["kind"].as_u64().unwrap() as u16;
                    server_kinds.lock().unwrap().push(kind);
                    if kind == buzz_core::kind::KIND_GIT_REPO_ANNOUNCEMENT as u16 {
                        (
                            "200 OK",
                            serde_json::json!({
                                "event_id": event["id"], "accepted": true, "message": "duplicate"
                            })
                            .to_string(),
                        )
                    } else {
                        (
                            "200 OK",
                            serde_json::json!({
                                "event_id": event["id"], "accepted": true, "message": ""
                            })
                            .to_string(),
                        )
                    }
                } else {
                    ("404 Not Found", "{}".to_string())
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = crate::client::BuzzClient::new(base_url, keys, None, None).unwrap();
        let result = cmd_create(
            &client,
            "app",
            &[],
            Some("App"),
            None,
            Some(requested_channel),
            None,
        )
        .await;
        server.abort();
        let kinds = posted_kinds.lock().unwrap().clone();
        (result, kinds)
    }

    #[tokio::test]
    async fn create_does_not_publish_project_after_default_repo_loses_to_foreign_home() {
        let (result, posted_kinds) =
            run_default_repo_create_race("22222222-2222-4222-8222-222222222222").await;

        assert!(matches!(result, Err(CliError::Conflict(_))));
        assert_eq!(
            posted_kinds,
            vec![buzz_core::kind::KIND_GIT_REPO_ANNOUNCEMENT as u16],
            "the command must stop before publishing kind:30621"
        );
    }

    #[tokio::test]
    async fn create_is_idempotent_when_dominated_default_repo_winner_matches_home() {
        let (result, posted_kinds) =
            run_default_repo_create_race("11111111-1111-4111-8111-111111111111").await;

        result.expect("matching winning repo head permits project publication");
        assert_eq!(
            posted_kinds,
            vec![
                buzz_core::kind::KIND_GIT_REPO_ANNOUNCEMENT as u16,
                buzz_core::kind::KIND_PROJECT as u16,
            ]
        );
    }

    // ── Coordinate expansion ──────────────────────────────────────────────────

    const OWNER_HEX: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OWNER_B_HEX: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[tokio::test]
    async fn project_lookup_scopes_the_production_query_before_the_global_bound() {
        use std::sync::{Arc, Mutex};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let channel = "11111111-1111-4111-8111-111111111111";
        let request_body = Arc::new(Mutex::new(None));
        let captured_body = request_body.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0; 65_536];
            let read = socket.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..read]);
            let body = request.split("\r\n\r\n").nth(1).unwrap().to_owned();
            *captured_body.lock().unwrap() = Some(body);
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]",
                )
                .await
                .unwrap();
        });
        let client =
            crate::client::BuzzClient::new(base_url, nostr::Keys::generate(), None, None).unwrap();

        let projects = fetch_projects_for_channel(&client, channel).await.unwrap();
        assert!(projects.is_empty());
        server.await.unwrap();
        let body: serde_json::Value =
            serde_json::from_str(request_body.lock().unwrap().as_deref().unwrap()).unwrap();
        assert_eq!(body[0]["#buzz-channel"], serde_json::json!([channel]));
        assert_eq!(body[0]["kinds"], serde_json::json!([KIND_PROJECT]));
    }

    #[tokio::test]
    async fn channel_scoping_prevents_unrelated_heads_from_consuming_the_bound() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let channel = "11111111-1111-4111-8111-111111111111";
        let target = build_project("target", None, None, &[], Some(channel), None)
            .unwrap()
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap();
        let decoy_channel = "22222222-2222-4222-8222-222222222222";
        let decoys = ["decoy-a", "decoy-b"].map(|slug| {
            build_project(slug, None, None, &[], Some(decoy_channel), None)
                .unwrap()
                .sign_with_keys(&nostr::Keys::generate())
                .unwrap()
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0; 65_536];
            let read = socket.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..read]);
            let body = request.split("\r\n\r\n").nth(1).unwrap();
            let filter: serde_json::Value = serde_json::from_str(body).unwrap();
            let response_body = if filter[0]["#buzz-channel"] == serde_json::json!([channel]) {
                serde_json::to_string(&[target]).unwrap()
            } else {
                serde_json::to_string(&decoys).unwrap()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let client =
            crate::client::BuzzClient::new(base_url, nostr::Keys::generate(), None, None).unwrap();

        let projects = fetch_projects_for_channel_bounded(&client, channel, 1)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(project_slug(&projects[0]).as_deref(), Some("target"));
    }

    #[test]
    fn project_channel_matching_ignores_unrelated_claims() {
        let expected = "11111111-1111-4111-8111-111111111111";
        let tags = make_head_tags(&[
            make_test_tag(&["buzz-channel", "22222222-2222-4222-8222-222222222222"]),
            make_test_tag(&["name", "Unrelated"]),
        ]);
        assert!(!project_tags_match_channel(tags.iter(), expected));

        let tags = make_head_tags(&[make_test_tag(&["buzz-channel", expected])]);
        assert!(project_tags_match_channel(tags.iter(), expected));
    }

    #[test]
    fn expand_repo_coord_bare_expands_with_caller_pubkey() {
        let coord = expand_repo_coord("my-repo", OWNER_HEX).unwrap();
        assert_eq!(coord.coord, format!("30617:{OWNER_HEX}:my-repo"));
    }

    #[test]
    fn expand_repo_coord_full_passes_through() {
        let full = format!("30617:{OWNER_HEX}:some-repo");
        let coord = expand_repo_coord(&full, OWNER_B_HEX).unwrap();
        // Owner from the full coord, not the caller.
        assert_eq!(coord.coord, full);
    }

    #[test]
    fn expand_repo_coord_full_cross_owner() {
        let full = format!("30617:{OWNER_B_HEX}:infra");
        let coord = expand_repo_coord(&full, OWNER_HEX).unwrap();
        assert_eq!(coord.coord, full);
    }

    #[test]
    fn expand_repo_coord_rejects_uppercase_owner() {
        let upper = "30617:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA:buzz";
        assert!(expand_repo_coord(upper, OWNER_HEX).is_err());
    }

    #[test]
    fn expand_repo_coord_rejects_coordinate_shaped_bare_value() {
        // A value with a colon is never a bare id.
        let not_bare = "30617:something";
        // parse_full will fail because it's not a valid full coordinate either.
        assert!(expand_repo_coord(not_bare, OWNER_HEX).is_err());
    }

    // ── validate_project_slug ─────────────────────────────────────────────────

    #[test]
    fn validate_project_slug_accepts_normal() {
        assert!(validate_project_slug("my-project").is_ok());
        assert!(validate_project_slug("platform:v2").is_ok()); // colons allowed — more permissive than repo-id
    }

    #[test]
    fn validate_project_slug_rejects_empty() {
        assert!(validate_project_slug("").is_err());
    }

    #[test]
    fn validate_project_slug_rejects_over_1024() {
        let long = "a".repeat(1025);
        assert!(validate_project_slug(&long).is_err());
    }

    #[test]
    fn validate_project_slug_accepts_1024() {
        let at_limit = "a".repeat(1024);
        assert!(validate_project_slug(&at_limit).is_ok());
    }

    // ── validate_visibility ───────────────────────────────────────────────────

    #[test]
    fn validate_visibility_accepts_listed_and_unlisted() {
        assert!(validate_visibility("listed").is_ok());
        assert!(validate_visibility("unlisted").is_ok());
    }

    #[test]
    fn validate_visibility_rejects_unknown_token() {
        assert!(validate_visibility("chartreuse").is_err());
        assert!(validate_visibility("").is_err());
    }

    // ── is_bare_repo_id ───────────────────────────────────────────────────────

    #[test]
    fn bare_repo_id_accepts_valid() {
        assert!(is_bare_repo_id("buzz"));
        assert!(is_bare_repo_id("my-repo_1.0"));
    }

    #[test]
    fn bare_repo_id_rejects_colon() {
        assert!(!is_bare_repo_id("30617:something"));
        assert!(!is_bare_repo_id("has:colon"));
    }

    #[test]
    fn bare_repo_id_rejects_empty() {
        assert!(!is_bare_repo_id(""));
    }

    #[test]
    fn bare_repo_id_rejects_over_64() {
        let long = "a".repeat(65);
        assert!(!is_bare_repo_id(&long));
    }

    // ── tag helpers ───────────────────────────────────────────────────────────

    fn make_test_tag(parts: &[&str]) -> Tag {
        Tag::parse(parts.iter().copied()).unwrap()
    }

    // ── rebuild_project: hinted / unknown tag preservation ───────────────────

    #[test]
    fn rebuild_project_preserves_hinted_member_tags() {
        // A member 'a' tag with a relay hint must survive RMW untouched.
        let coord = format!("30617:{OWNER_HEX}:buzz");
        let hint = "wss://relay.example.com";
        let tags = vec![
            make_test_tag(&["d", "platform"]),
            Tag::parse(["a", &coord, hint]).unwrap(),
        ];
        let ts = Timestamp::from(1_700_000_001u64);
        let b = rebuild_project("", tags, ts).unwrap();
        let ev = b.sign_with_keys(&nostr::Keys::generate()).expect("sign");
        let a_tag = ev
            .tags
            .iter()
            .find(|t| tag_name(t) == Some("a"))
            .expect("a tag present");
        assert_eq!(
            a_tag.as_slice(),
            &["a".to_string(), coord, hint.to_string()],
            "relay hint must survive rebuild"
        );
    }

    #[test]
    fn rebuild_project_preserves_unknown_tags() {
        let tags = vec![
            make_test_tag(&["d", "platform"]),
            make_test_tag(&["future-metadata", "value"]),
        ];
        let ts = Timestamp::from(1_700_000_001u64);
        let b = rebuild_project("", tags, ts).unwrap();
        let ev = b.sign_with_keys(&nostr::Keys::generate()).expect("sign");
        assert!(ev
            .tags
            .iter()
            .any(|t| tag_name(t) == Some("future-metadata")));
    }

    #[test]
    fn rebuild_project_strips_auth_tag() {
        let tags = vec![
            make_test_tag(&["d", "platform"]),
            make_test_tag(&["auth", &"a".repeat(64), "kind=30617", &"b".repeat(128)]),
        ];
        let ts = Timestamp::from(1_700_000_001u64);
        let b = rebuild_project("", tags, ts).unwrap();
        let ev = b.sign_with_keys(&nostr::Keys::generate()).expect("sign");
        assert!(
            !ev.tags.iter().any(|t| tag_name(t) == Some("auth")),
            "auth tag must be stripped"
        );
    }

    #[test]
    fn rebuild_project_rejects_over_cap_foreign_head() {
        // A foreign head with 65 members must fail Layer A on republish.
        let mut tags = vec![make_test_tag(&["d", "wide"])];
        for i in 0..=64u32 {
            let coord = format!("30617:{OWNER_HEX}:repo-{i:02}");
            tags.push(make_test_tag(&["a", &coord]));
        }
        assert_eq!(
            tags.iter().filter(|t| tag_name(t) == Some("a")).count(),
            65,
            "65 a-tags"
        );
        let ts = Timestamp::from(1_700_000_001u64);
        // rebuild_project strips auth, but 65 a-tags still exceeds cap.
        assert!(
            rebuild_project("", tags, ts).is_err(),
            "over-cap foreign head must fail rebuild"
        );
    }

    #[test]
    fn rebuild_project_at_exact_cap_succeeds() {
        let mut tags = vec![make_test_tag(&["d", "wide"])];
        for i in 0..PROJECT_MEMBER_CAP {
            let coord = format!("30617:{OWNER_HEX}:repo-{i:02}");
            tags.push(make_test_tag(&["a", &coord]));
        }
        let ts = Timestamp::from(1_700_000_001u64);
        assert!(rebuild_project("", tags, ts).is_ok());
    }

    // ── clear-flag semantics ──────────────────────────────────────────────────

    /// Build a minimal head Event for testing update semantics without the relay.
    fn make_head_tags(extra: &[Tag]) -> Vec<Tag> {
        let mut tags = vec![make_test_tag(&["d", "platform"])];
        tags.extend_from_slice(extra);
        tags
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_update_tags(
        head_tags: Vec<Tag>,
        name: Option<&str>,
        clear_name: bool,
        description: Option<&str>,
        clear_description: bool,
        channel: Option<&str>,
        clear_channel: bool,
        visibility: Option<&str>,
        clear_visibility: bool,
    ) -> Vec<Tag> {
        // Replicate the tag-mutation logic from cmd_update (sans relay I/O).
        let singleton_fields = ["name", "description", "buzz-channel", "buzz-visibility"];
        let mut tags: Vec<Tag> = head_tags
            .iter()
            .filter(|t| {
                if tag_name(t) == Some("auth") {
                    return false;
                }
                if let Some(field) = tag_name(t) {
                    if singleton_fields.contains(&field) {
                        let clear = match field {
                            "name" => clear_name || name.is_some(),
                            "description" => clear_description || description.is_some(),
                            "buzz-channel" => clear_channel || channel.is_some(),
                            "buzz-visibility" => clear_visibility || visibility.is_some(),
                            _ => false,
                        };
                        return !clear;
                    }
                }
                true
            })
            .cloned()
            .collect();
        if let Some(n) = name {
            tags.push(make_test_tag(&["name", n]));
        }
        if let Some(d) = description {
            tags.push(make_test_tag(&["description", d]));
        }
        if let Some(ch) = channel {
            tags.push(make_test_tag(&["buzz-channel", ch]));
        }
        if let Some(vis) = visibility {
            tags.push(make_test_tag(&["buzz-visibility", vis]));
        }
        tags
    }

    #[test]
    fn update_omission_preserves_existing_field() {
        let head = make_head_tags(&[make_test_tag(&["name", "Old Name"])]);
        let result = apply_update_tags(head, None, false, None, false, None, false, None, false);
        assert!(result.iter().any(|t| tag_value(t) == Some("Old Name")));
    }

    #[test]
    fn update_setter_replaces_existing_field() {
        let head = make_head_tags(&[make_test_tag(&["name", "Old Name"])]);
        let result = apply_update_tags(
            head,
            Some("New Name"),
            false,
            None,
            false,
            None,
            false,
            None,
            false,
        );
        assert!(result.iter().any(|t| tag_value(t) == Some("New Name")));
        assert!(!result.iter().any(|t| tag_value(t) == Some("Old Name")));
    }

    #[test]
    fn update_clear_drops_existing_field() {
        let head = make_head_tags(&[make_test_tag(&["name", "Old Name"])]);
        let result = apply_update_tags(head, None, true, None, false, None, false, None, false);
        assert!(!result.iter().any(|t| tag_name(t) == Some("name")));
    }

    #[test]
    fn update_clear_visibility_drops_tag() {
        let head = make_head_tags(&[make_test_tag(&["buzz-visibility", "unlisted"])]);
        let result = apply_update_tags(head, None, false, None, false, None, false, None, true);
        assert!(!result
            .iter()
            .any(|t| tag_name(t) == Some("buzz-visibility")));
    }

    #[test]
    fn update_exactly_one_singleton_after_replace() {
        // Start with a buzz-channel; replace with a new one; must have exactly one.
        let uuid1 = "3580ca9b-47b4-4af9-b22a-1068778f26c6";
        let uuid2 = "00000000-0000-0000-0000-000000000000";
        let head = make_head_tags(&[make_test_tag(&["buzz-channel", uuid1])]);
        let result = apply_update_tags(
            head,
            None,
            false,
            None,
            false,
            Some(uuid2),
            false,
            None,
            false,
        );
        let channels: Vec<_> = result
            .iter()
            .filter(|t| tag_name(t) == Some("buzz-channel"))
            .collect();
        assert_eq!(channels.len(), 1);
        assert_eq!(tag_value(channels[0]), Some(uuid2));
    }

    // ── duplicate-member rejection on republish ───────────────────────────────

    #[test]
    fn duplicate_member_in_foreign_head_fails_rebuild() {
        let coord = format!("30617:{OWNER_HEX}:buzz");
        let tags = vec![
            make_test_tag(&["d", "platform"]),
            make_test_tag(&["a", &coord]),
            make_test_tag(&["a", &coord]), // duplicate
        ];
        let ts = Timestamp::from(1_700_000_001u64);
        assert!(rebuild_project("", tags, ts).is_err());
    }

    // ── validate_project_envelope integration ────────────────────────────────

    #[test]
    fn validate_project_envelope_accepts_hinted_member() {
        let coord = format!("30617:{OWNER_HEX}:buzz");
        let tags = vec![
            make_test_tag(&["d", "platform"]),
            Tag::parse(["a", &coord, "wss://relay.example.com"]).unwrap(),
        ];
        assert!(validate_project_envelope(&tags, "").is_ok());
    }

    #[test]
    fn validate_project_envelope_rejects_four_element_member() {
        let coord = format!("30617:{OWNER_HEX}:buzz");
        let tags = vec![
            make_test_tag(&["d", "platform"]),
            Tag::parse(["a", &coord, "wss://relay.example.com", "extra"]).unwrap(),
        ];
        assert!(validate_project_envelope(&tags, "").is_err());
    }

    // ── next_timestamp ordering ───────────────────────────────────────────────

    fn project_head_at(created_at: u64) -> Event {
        let keys = nostr::Keys::generate();
        let tags = vec![
            make_test_tag(&["d", "platform"]),
            make_test_tag(&["a", &format!("30617:{OWNER_HEX}:buzz")]),
        ];
        rebuild_project("", tags, Timestamp::from(created_at))
            .expect("valid head envelope")
            .sign_with_keys(&keys)
            .expect("sign")
    }

    #[test]
    fn next_timestamp_uses_later_of_wall_clock_and_after_head() {
        let cases = [
            ("stale head", 100, 1_000, 1_000),
            ("head equal to now", 1_000, 1_000, 1_001),
            ("future head", 1_500, 1_000, 1_501),
            ("last timestamp inside future boundary", 1_899, 1_000, 1_900),
            (
                "future boundary cannot be dominated inside the window",
                1_900,
                1_000,
                1_901,
            ),
        ];

        for (name, head_ts, now, expected) in cases {
            let head = project_head_at(head_ts);
            let next = next_timestamp(&head, Timestamp::from(now)).expect("no overflow");

            assert_eq!(next.as_secs(), expected, "case: {name}");
        }
    }

    #[test]
    fn next_timestamp_rejects_overflowing_head() {
        let head = project_head_at(u64::MAX);

        let err = next_timestamp(&head, Timestamp::from(1_000u64))
            .expect_err("maximum timestamp cannot be advanced");

        assert!(
            matches!(err, CliError::Other(ref message) if message == "project timestamp cannot be advanced"),
            "unexpected error: {err}"
        );
    }

    // ── empty update guard ────────────────────────────────────────────────────

    /// `cmd_update` with no setters or clearers must return `CliError::Usage`
    /// before making any network call.  The guard is synchronous (before the
    /// first `.await`) so we can drive it with a dummy client whose address
    /// would reject any real connection attempt.
    #[tokio::test]
    async fn empty_update_returns_usage_error_before_any_network_call() {
        let keys = nostr::Keys::generate();
        // Port 9 is the discard protocol — any real connect will be refused
        // immediately, but the guard fires before the first await so this
        // never reaches the network.
        let client = crate::client::BuzzClient::new("http://127.0.0.1:9".into(), keys, None, None)
            .expect("client construction");

        let err = cmd_update(
            &client, "my-slug", None, false, // name / clear_name
            None, false, // description / clear_description
            None, false, // channel / clear_channel
            None, false, // visibility / clear_visibility
        )
        .await
        .expect_err("empty update must fail");

        assert!(
            matches!(err, CliError::Usage(_)),
            "expected CliError::Usage, got {err:?}"
        );
    }

    // ── no-network malformed-input tests ─────────────────────────────────────
    //
    // All three cases use port 9 (discard protocol): any real connection is
    // refused immediately, but local validation fires before the first .await
    // so the network is never touched.

    fn discard_client() -> crate::client::BuzzClient {
        let keys = nostr::Keys::generate();
        crate::client::BuzzClient::new("http://127.0.0.1:9".into(), keys, None, None)
            .expect("client construction")
    }

    /// Creating without --repo or --channel must fail locally; the default
    /// repository needs a home channel to bind as git ACL.
    #[tokio::test]
    async fn create_without_repo_or_channel_returns_usage_before_any_network_call() {
        let client = discard_client();
        let err = cmd_create(&client, "my-slug", &[], None, None, None, None)
            .await
            .expect_err("missing repo and channel must fail");
        assert!(
            matches!(err, CliError::Usage(_)),
            "expected CliError::Usage, got {err:?}"
        );
        assert!(
            format!("{err}").contains("--channel"),
            "Usage message must mention --channel, got {err:?}"
        );
    }

    /// Invalid visibility token must return Usage before touching the relay.
    #[tokio::test]
    async fn create_invalid_visibility_returns_usage_before_any_network_call() {
        let client = discard_client();
        let err = cmd_create(
            &client,
            "my-slug",
            &["buzz".to_string()],
            None,
            None,
            None,
            Some("chartreuse"),
        )
        .await
        .expect_err("invalid visibility must fail");
        assert!(
            matches!(err, CliError::Usage(_)),
            "expected CliError::Usage for invalid visibility, got {err:?}"
        );
    }

    /// A name longer than 256 bytes must return Usage before touching the relay.
    #[tokio::test]
    async fn create_overlong_name_returns_usage_before_any_network_call() {
        let client = discard_client();
        let long_name = "a".repeat(257);
        let err = cmd_create(
            &client,
            "my-slug",
            &["buzz".to_string()],
            Some(&long_name),
            None,
            None,
            None,
        )
        .await
        .expect_err("overlong name must fail");
        assert!(
            matches!(err, CliError::Usage(_)),
            "expected CliError::Usage for overlong name, got {err:?}"
        );
    }

    /// A malformed --repo coordinate must return Usage before touching the relay.
    #[tokio::test]
    async fn create_malformed_repo_returns_usage_before_any_network_call() {
        let client = discard_client();
        let err = cmd_create(
            &client,
            "my-slug",
            &["nope:bad".to_string()],
            None,
            None,
            None,
            None,
        )
        .await
        .expect_err("malformed repo must fail");
        assert!(
            matches!(err, CliError::Usage(_)),
            "expected CliError::Usage for malformed repo, got {err:?}"
        );
    }

    /// A malformed --repo coordinate on add-repo must return Usage before touching the relay.
    #[tokio::test]
    async fn add_repo_malformed_coord_returns_usage_before_any_network_call() {
        let client = discard_client();
        let err = cmd_add_repo(&client, "my-slug", &["nope:bad".to_string()])
            .await
            .expect_err("malformed repo must fail");
        assert!(
            matches!(err, CliError::Usage(_)),
            "expected CliError::Usage for malformed repo on add-repo, got {err:?}"
        );
    }

    /// A malformed --repo coordinate on remove-repo must return Usage before touching the relay.
    #[tokio::test]
    async fn remove_repo_malformed_coord_returns_usage_before_any_network_call() {
        let client = discard_client();
        let err = cmd_remove_repo(&client, "my-slug", &["nope:bad".to_string()])
            .await
            .expect_err("malformed repo must fail");
        assert!(
            matches!(err, CliError::Usage(_)),
            "expected CliError::Usage for malformed repo on remove-repo, got {err:?}"
        );
    }

    // ── duplicate --repo within one invocation ────────────────────────────────

    /// Supplying the same coordinate twice in one create call must return Usage
    /// (names the duplicate) before any network call.
    #[tokio::test]
    async fn create_duplicate_repo_returns_usage_before_any_network_call() {
        let client = discard_client();
        let coord = format!("30617:{OWNER_HEX}:buzz");
        let err = cmd_create(
            &client,
            "my-slug",
            &[coord.clone(), coord.clone()],
            None,
            None,
            None,
            None,
        )
        .await
        .expect_err("duplicate repo must fail");
        assert!(
            matches!(err, CliError::Usage(_)),
            "expected CliError::Usage for duplicate repo, got {err:?}"
        );
        // Error message must name the duplicate coordinate.
        assert!(
            format!("{err}").contains("buzz"),
            "Usage message must name the duplicate coordinate, got {err:?}"
        );
    }

    /// Supplying the same coordinate twice in one add-repo call must return Usage
    /// (names the duplicate) before any network call.
    #[tokio::test]
    async fn add_repo_duplicate_coord_returns_usage_before_any_network_call() {
        let client = discard_client();
        let coord = format!("30617:{OWNER_HEX}:buzz");
        let err = cmd_add_repo(&client, "my-slug", &[coord.clone(), coord.clone()])
            .await
            .expect_err("duplicate repo must fail");
        assert!(
            matches!(err, CliError::Usage(_)),
            "expected CliError::Usage for duplicate repo on add-repo, got {err:?}"
        );
    }

    // ── create collision guard ────────────────────────────────────────────────

    // The create-collision Conflict path is pinned by the live transcript
    // (step: duplicate create → Conflict, exit=5). No relay mock is available
    // for a unit test; the no-network tests above cover all pre-await paths.

    // ── add-repo no-op guard ──────────────────────────────────────────────────

    // The add-repo no-op Conflict path is pinned by the live transcript
    // (step 7: buzz already present → exit=5). No relay mock is available
    // for a unit test; the async no-network tests above cover all pre-await paths.
}
