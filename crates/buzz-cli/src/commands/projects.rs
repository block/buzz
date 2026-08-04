//! `buzz projects` commands — NIP-MP kind:30621 write path.
//!
//! All mutations follow a read-modify-write pattern:
//!   1. Fetch the caller's own live head via `kinds:[30621] + authors:[self] + #d:[slug]`.
//!   2. Mutate the tag set (strip `auth`, apply change).
//!   3. Re-validate the full envelope through Layer A before submitting.
//!   4. Set `created_at = head.created_at + 1` (never wall-clock) to avoid
//!      overwriting a concurrently advancing head.
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
use nostr::{Event, EventBuilder, Tag, Timestamp};

use crate::client::BuzzClient;
use crate::commands::parse_write_response;
use crate::error::CliError;

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

/// Fetch the caller's own live kind:30621 head for `slug`.
async fn fetch_own_project(client: &BuzzClient, slug: &str) -> Result<Option<Event>, CliError> {
    fetch_project(client, slug, None).await
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

async fn submit_project(client: &BuzzClient, builder: EventBuilder) -> Result<(), CliError> {
    let event = client.sign_event(builder)?;
    let raw = client.submit_event(event).await?;
    println!(
        "{}",
        parse_write_response(&raw, "project changed concurrently; retry")?
    );
    Ok(())
}

// ── Build helpers ─────────────────────────────────────────────────────────────

/// Advance the `created_at` counter off an observed head.
fn next_timestamp(head: &Event) -> Result<Timestamp, CliError> {
    head.created_at
        .as_secs()
        .checked_add(1)
        .map(Timestamp::from)
        .ok_or_else(|| CliError::Other("project timestamp cannot be advanced".into()))
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
    let members: Vec<ProjectMemberCoord> = repos
        .iter()
        .map(|r| expand_repo_coord(r, &caller_pubkey))
        .collect::<Result<Vec<_>, _>>()?;

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

    // ── Build via Layer B (enforces all writer policy) ────────────────────
    let builder = build_project(slug, name, description, &members, channel, visibility)
        .map_err(|e| CliError::Usage(e.to_string()))?;
    submit_project(client, builder).await
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
    validate_project_slug(slug)?;
    let caller_pubkey = client.keys().public_key().to_hex();

    // ── Local validation before any .await ────────────────────────────────
    let new_members: Vec<ProjectMemberCoord> = repos
        .iter()
        .map(|r| expand_repo_coord(r, &caller_pubkey))
        .collect::<Result<Vec<_>, _>>()?;

    // Dedupe within this invocation: first occurrence wins, duplicate → Usage.
    let mut seen = std::collections::HashSet::new();
    for m in &new_members {
        if !seen.insert(m.coord.clone()) {
            return Err(CliError::Usage(format!(
                "duplicate --repo coordinate in this invocation: {:?}",
                m.coord
            )));
        }
    }

    // ── Network: fetch head ───────────────────────────────────────────────
    let head = fetch_own_project(client, slug)
        .await?
        .ok_or_else(|| CliError::NotFound(format!("project {slug:?} not found")))?;
    let next_ts = next_timestamp(&head)?;

    // Build the new tag set: keep existing tags (including hinted members),
    // append new members only if not already present (by coordinate).
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

    // All requested coordinates were already present — no change to publish.
    if added == 0 {
        return Err(CliError::Conflict(format!(
            "all requested repositories are already members of project {slug:?}"
        )));
    }

    let builder = rebuild_project(&head.content, tags, next_ts)?;
    submit_project(client, builder).await
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
    let next_ts = next_timestamp(&head)?;

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
    submit_project(client, builder).await
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
    let next_ts = next_timestamp(&head)?;

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
    submit_project(client, builder).await
}

/// `buzz projects delete`
///
/// Head-based and verified:
///   1. Fetch own live head — `NotFound` if absent.
///   2. Build tombstone at `head.created_at + 1`.
///   3. Submit.
///   4. Re-query the coordinate; if a newer head survived → `Conflict`.
pub async fn cmd_delete(client: &BuzzClient, slug: &str) -> Result<(), CliError> {
    validate_project_slug(slug)?;

    let head = fetch_own_project(client, slug)
        .await?
        .ok_or_else(|| CliError::NotFound(format!("project {slug:?} not found")))?;
    let next_ts = next_timestamp(&head)?;

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

    // ── Coordinate expansion ──────────────────────────────────────────────────

    const OWNER_HEX: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OWNER_B_HEX: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

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

    /// `next_timestamp` must return `head.created_at + 1` regardless of the wall
    /// clock.  NIP-MP Deletion rule: a tombstone older than the live head does
    /// NOT remove it, so we must advance strictly off the observed head — never
    /// use wall-clock time, which could be behind a head that was bumped
    /// multiple times in the same second.
    #[test]
    fn next_timestamp_returns_head_plus_one_when_head_is_ahead_of_wall_clock() {
        // Build a minimal signed event with a created_at far in the future.
        let keys = nostr::Keys::generate();
        let far_future_ts = Timestamp::from(9_999_999_999u64); // year 2286
        let tags = vec![
            make_test_tag(&["d", "platform"]),
            make_test_tag(&["a", &format!("30617:{OWNER_HEX}:buzz")]),
        ];
        let builder = rebuild_project("", tags, far_future_ts).expect("valid head envelope");
        let head = builder.sign_with_keys(&keys).expect("sign");
        // Verify the event actually has our future timestamp.
        assert_eq!(head.created_at, far_future_ts);

        // next_timestamp must return far_future + 1, not now().
        let next = next_timestamp(&head).expect("no overflow");
        assert_eq!(
            next.as_secs(),
            far_future_ts.as_secs() + 1,
            "tombstone must be strictly after head, even when head is far in the future"
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
