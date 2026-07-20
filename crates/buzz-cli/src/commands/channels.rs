use std::collections::HashSet;

use buzz_core::kind::{KIND_MANAGED_AGENT, KIND_TEAM};
use serde::Deserialize;
use uuid::Uuid;

use crate::client::{
    extract_d_tag, extract_p_tags, extract_tag_value, normalize_write_response,
    print_create_response, BuzzClient,
};
use crate::commands::channel_templates::{self, ChannelTemplateRecord, TemplateAgentRoster};
use crate::error::CliError;
use crate::validate::{parse_uuid, read_or_stdin, validate_hex64, validate_uuid};

fn extract_channel_metadata(e: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "channel_id": extract_d_tag(e),
        "name": extract_tag_value(e, "name"),
        "description": extract_tag_value(e, "about"),
        "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
    })
}

pub async fn cmd_list_channels(
    client: &BuzzClient,
    visibility: Option<&str>,
    member: Option<bool>,
    limit: Option<u32>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    let effective_limit = limit.unwrap_or(500);
    let events = if member == Some(true) {
        // Step 1: find channel IDs where we're a member (kind:39002)
        let my_pk = client.keys().public_key().to_hex();
        let member_filter = serde_json::json!({
            "kinds": [39002],
            "#p": [my_pk],
        });
        let member_events = client
            .query_paginated(member_filter, effective_limit)
            .await?;
        let channel_ids: Vec<String> = member_events
            .iter()
            .map(extract_d_tag)
            .filter(|id| !id.is_empty())
            .collect();
        if channel_ids.is_empty() {
            println!("[]");
            return Ok(());
        }
        // Step 2: fetch kind:39000 metadata for those channels.
        let metadata_filter = serde_json::json!({
            "kinds": [39000],
            "#d": channel_ids,
        });
        client
            .query_paginated(metadata_filter, effective_limit)
            .await?
    } else {
        let filter = serde_json::json!({
            "kinds": [39000],
        });
        client.query_paginated(filter, effective_limit).await?
    };

    let channels: Vec<serde_json::Value> = events
        .iter()
        .filter(|e| {
            if let Some(vis) = visibility {
                // NIP-29: relay emits ["public"] or ["private"] single-element tags
                let nip29_tag = match vis {
                    "open" => "public",
                    _ => vis,
                };
                e.get("tags")
                    .and_then(|t| t.as_array())
                    .map(|tags| {
                        tags.iter().any(|tag| {
                            tag.as_array()
                                .map(|a| {
                                    a.len() == 1
                                        && a.first().and_then(|v| v.as_str()) == Some(nip29_tag)
                                })
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            } else {
                true
            }
        })
        .map(extract_channel_metadata)
        .collect();
    let output = match format {
        crate::OutputFormat::Compact => {
            let compact: Vec<serde_json::Value> = channels
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "channel_id": c.get("channel_id").cloned().unwrap_or_default(),
                        "name": c.get("name").cloned().unwrap_or_default(),
                    })
                })
                .collect();
            serde_json::to_string(&compact).unwrap_or_default()
        }
        crate::OutputFormat::Json => serde_json::to_string(&channels).unwrap_or_default(),
    };
    println!("{output}");
    Ok(())
}

/// Search channels by human-readable name (kind:39000 group metadata).
///
/// The relay's access control already filters out channels the caller can't see
/// (private channels they're not a member of), so we just post-filter the
/// returned events by name and project them into a stable JSON shape.
pub async fn cmd_search_channels(
    client: &BuzzClient,
    query: &str,
    exact: bool,
    include_archived: bool,
    limit: u32,
) -> Result<(), CliError> {
    if query.trim().is_empty() {
        return Err(CliError::Usage("--query cannot be empty".into()));
    }

    let filter = serde_json::json!({
        "kinds": [39000],
    });
    let arr = client.query_paginated(filter, limit).await?;

    let needle = query.to_ascii_lowercase();
    let mut matches: Vec<ChannelSummary> = arr
        .iter()
        .filter_map(ChannelSummary::from_event)
        .filter(|c| if include_archived { true } else { !c.archived })
        .filter(|c| name_matches(&c.name, &needle, exact))
        .collect();
    matches.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.channel_id.cmp(&b.channel_id))
    });

    let output = serde_json::to_string(&matches).expect("serializing ChannelSummary");
    println!("{output}");
    Ok(())
}

/// Stable, scriptable projection of a kind:39000 channel-metadata event.
#[derive(serde::Serialize)]
struct ChannelSummary {
    channel_id: String,
    name: String,
    channel_type: Option<String>,
    visibility: Option<String>,
    archived: bool,
    about: Option<String>,
    topic: Option<String>,
    purpose: Option<String>,
}

impl ChannelSummary {
    /// Parse a kind:39000 event JSON value into a summary. Returns `None` if the
    /// event lacks the required `d` (channel UUID) or `name` tags.
    fn from_event(event: &serde_json::Value) -> Option<Self> {
        let tags = event.get("tags")?.as_array()?;
        let mut channel_id: Option<String> = None;
        let mut name: Option<String> = None;
        let mut channel_type: Option<String> = None;
        let mut visibility: Option<String> = None;
        let mut archived = false;
        let mut about: Option<String> = None;
        let mut topic: Option<String> = None;
        let mut purpose: Option<String> = None;

        for tag in tags {
            let Some(tag_arr) = tag.as_array() else {
                continue;
            };
            let key = tag_arr.first().and_then(|v| v.as_str()).unwrap_or("");
            let val = tag_arr.get(1).and_then(|v| v.as_str());
            match key {
                "d" => channel_id = val.map(str::to_string),
                "name" => name = val.map(str::to_string),
                "t" => channel_type = val.map(str::to_string),
                // NIP-29 emits both `private` and `public` (Buzz adds the latter).
                // The presence of either tag is the source of truth; tag value is unused.
                "private" => visibility = Some("private".to_string()),
                "public" => visibility = Some("public".to_string()),
                "about" => about = val.map(str::to_string),
                "topic" => topic = val.map(str::to_string),
                "purpose" => purpose = val.map(str::to_string),
                "archived" => archived = val == Some("true"),
                _ => {}
            }
        }

        Some(ChannelSummary {
            channel_id: channel_id?,
            name: name?,
            channel_type,
            visibility,
            archived,
            about,
            topic,
            purpose,
        })
    }
}

fn name_matches(name: &str, needle_lower: &str, exact: bool) -> bool {
    let hay = name.to_ascii_lowercase();
    if exact {
        hay == needle_lower
    } else {
        hay.contains(needle_lower)
    }
}

pub async fn cmd_get_channel(client: &BuzzClient, channel_id: &str) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let filter = serde_json::json!({
        "kinds": [39000],
        "#d": [channel_id],
        "limit": 1
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    if let Some(e) = events.first() {
        let mut normalized = extract_channel_metadata(e);
        normalized["pubkey"] =
            serde_json::json!(e.get("pubkey").and_then(|v| v.as_str()).unwrap_or(""));
        println!("{normalized}");
    } else {
        println!("null");
    }
    Ok(())
}

pub async fn cmd_list_channel_members(
    client: &BuzzClient,
    channel_id: &str,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let filter = serde_json::json!({
        "kinds": [39002],
        "#d": [channel_id],
        "limit": 1
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let members = events.first().map(extract_p_tags).unwrap_or_default();
    let output = serde_json::to_string(&members).unwrap_or_default();
    println!("{output}");
    Ok(())
}

pub async fn cmd_get_canvas(client: &BuzzClient, channel_id: &str) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let filter = serde_json::json!({
        "kinds": [40100],
        "#h": [channel_id]
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    if let Some(content) = events
        .first()
        .and_then(|e| e.get("content"))
        .and_then(|c| c.as_str())
    {
        println!("{content}");
    } else {
        println!("null");
    }
    Ok(())
}

pub async fn cmd_create_channel(
    client: &BuzzClient,
    name: &str,
    channel_type: &str,
    visibility: &str,
    description: Option<&str>,
    ttl: Option<i64>,
) -> Result<(), CliError> {
    match channel_type {
        "stream" | "forum" => {}
        _ => {
            return Err(CliError::Usage(format!(
                "--type must be 'stream' or 'forum' (got: {channel_type})"
            )))
        }
    }
    match visibility {
        "open" | "private" => {}
        _ => {
            return Err(CliError::Usage(format!(
                "--visibility must be 'open' or 'private' (got: {visibility})"
            )))
        }
    }

    let ttl = ttl.map(validate_ttl_seconds).transpose()?;

    let channel_uuid = Uuid::new_v4();

    let vis = match visibility {
        "open" => buzz_sdk::Visibility::Open,
        "private" => buzz_sdk::Visibility::Private,
        _ => unreachable!(),
    };
    let ct = match channel_type {
        "stream" => buzz_sdk::ChannelKind::Stream,
        "forum" => buzz_sdk::ChannelKind::Forum,
        _ => unreachable!(),
    };
    let builder =
        buzz_sdk::build_create_channel(channel_uuid, name, Some(vis), Some(ct), description, ttl)
            .map_err(|e| CliError::Other(format!("build_create_channel failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    print_create_response(&resp, "channel_id", &channel_uuid.to_string());
    Ok(())
}

/// A resolved live managed-agent instance backing a template persona slug.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedAgent {
    persona_id: String,
    pubkey: String,
}

/// Minimal projection of a kind:30177 event's content needed for roster
/// resolution. Other fields (system_prompt, model, ...) are irrelevant here.
#[derive(Debug, Deserialize)]
struct ManagedAgentContent {
    #[serde(default)]
    persona_id: Option<String>,
}

/// Outcome of resolving a template's roster against the relay, before any
/// channel-creation side effects. `skipped` (zero live instances) and
/// cardinality errors are both known at this point — resolution happens
/// entirely before channel creation so ambiguity aborts with zero side effects.
#[derive(Debug)]
struct ResolvedRoster {
    /// Exactly one live instance per persona slug — safe to add.
    agents: Vec<ResolvedAgent>,
    /// Persona slugs with no live kind:30177 instance for the effective
    /// owner (cold-start provisioning is desktop-only, out of scope here).
    skipped: Vec<String>,
}

/// Fetch kind:30176 (team) events authored by `owner` with `#d = [team_id]`
/// and return the team's persona slugs. Absent `persona_ids` (publisher
/// predates always-publish, or no matching event) resolves to an empty slug
/// set — the CLI reads a single relay snapshot, not a local reconciled
/// merge, so "unknown" here is indistinguishable from "empty."
async fn fetch_team_persona_slugs(
    client: &BuzzClient,
    owner: &str,
    team_id: &str,
) -> Result<Vec<String>, CliError> {
    let filter = serde_json::json!({
        "kinds": [KIND_TEAM],
        "authors": [owner],
        "#d": [team_id],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse team query response: {e}")))?;
    let Some(event) = events.first() else {
        return Err(CliError::NotFound(format!(
            "team '{team_id}' not found for effective owner {owner}"
        )));
    };
    let content: serde_json::Value = event
        .get("content")
        .and_then(|c| c.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);
    let slugs = content
        .get("persona_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(slugs)
}

/// Scan all kind:30177 (managed-agent) events authored by `owner`, keyset-
/// paginated (`until` + `before_id`, never `page`/offset — 30177 is
/// parameterized-replaceable and offset drift can silently skip a live
/// instance across requests). Returns every event whose `content.persona_id`
/// is in `slugs`, keyed by the event's `d` tag (the agent pubkey).
async fn scan_managed_agents_by_owner(
    client: &BuzzClient,
    owner: &str,
    slugs: &HashSet<&str>,
) -> Result<Vec<ResolvedAgent>, CliError> {
    let filter = serde_json::json!({
        "kinds": [KIND_MANAGED_AGENT],
        "authors": [owner],
    });
    let events = client.query_all(filter).await?;
    let mut found: Vec<ResolvedAgent> = Vec::new();

    for event in &events {
        let pubkey = extract_d_tag(event);
        if pubkey.is_empty() {
            continue;
        }
        let content: ManagedAgentContent = event
            .get("content")
            .and_then(|c| c.as_str())
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(ManagedAgentContent { persona_id: None });
        let Some(persona_id) = content.persona_id else {
            continue;
        };
        if slugs.contains(persona_id.as_str()) {
            found.push(ResolvedAgent { persona_id, pubkey });
        }
    }

    Ok(found)
}

/// Apply the F4 cardinality rule per persona slug: zero live instances is a
/// known skip (cold-start provisioning is desktop-only, out of scope), one is
/// added, more than one is a hard error listing candidate pubkeys — matching
/// all instances silently would risk adding a stale or wrong instance. Pure
/// and independent of the relay so it's directly unit-testable.
fn apply_cardinality_rule(
    slugs: &[String],
    found: &[ResolvedAgent],
) -> Result<ResolvedRoster, CliError> {
    let mut agents = Vec::new();
    let mut skipped = Vec::new();
    for slug in slugs {
        let matches: Vec<&ResolvedAgent> = found.iter().filter(|a| &a.persona_id == slug).collect();
        match matches.as_slice() {
            [] => skipped.push(slug.clone()),
            [one] => agents.push((*one).clone()),
            many => {
                let candidates: Vec<&str> = many.iter().map(|a| a.pubkey.as_str()).collect();
                return Err(CliError::Usage(format!(
                    "persona '{slug}' has {} live instances for this owner ({}); \
                     pass a template with a single instance per persona, or resolve \
                     the duplicate in Buzz Desktop before creating the channel",
                    many.len(),
                    candidates.join(", ")
                )));
            }
        }
    }
    Ok(ResolvedRoster { agents, skipped })
}

/// Resolve a template's roster against the relay: expand team entries into
/// persona slugs (via kind:30176), scan for live kind:30177 instances scoped
/// to the effective owner, and apply the cardinality rule per slug. Runs
/// entirely before any channel-creation side effect — a cardinality error
/// aborts with nothing created.
async fn resolve_template_roster(
    client: &BuzzClient,
    owner: &str,
    roster: &TemplateAgentRoster,
) -> Result<ResolvedRoster, CliError> {
    let mut slugs: Vec<String> = Vec::new();
    for entry in &roster.personas {
        if !slugs.contains(&entry.persona_id) {
            slugs.push(entry.persona_id.clone());
        }
    }
    for team in &roster.teams {
        let team_slugs = fetch_team_persona_slugs(client, owner, &team.team_id).await?;
        for slug in team_slugs {
            if !slugs.contains(&slug) {
                slugs.push(slug);
            }
        }
    }

    if slugs.is_empty() {
        return Ok(ResolvedRoster {
            agents: Vec::new(),
            skipped: Vec::new(),
        });
    }

    let slug_set: HashSet<&str> = slugs.iter().map(String::as_str).collect();
    let found = scan_managed_agents_by_owner(client, owner, &slug_set).await?;
    apply_cardinality_rule(&slugs, &found)
}

/// `buzz channels create --template <name>`: load a desktop-local channel
/// template, resolve its agent roster against the relay, create the
/// channel, apply the canvas template, and add resolved agents as members.
///
/// Roster resolution happens entirely before channel creation (see
/// `resolve_template_roster`) so an ambiguous roster aborts with zero side
/// effects. Channel creation, canvas, and member-add are best-effort from
/// that point: canvas failures and per-member add failures are reported,
/// not fatal.
#[allow(clippy::too_many_arguments)]
pub async fn cmd_create_channel_from_template(
    client: &BuzzClient,
    name: &str,
    template_name: &str,
    templates_file: Option<&str>,
    channel_type_override: Option<&str>,
    visibility_override: Option<&str>,
    description: Option<&str>,
    ttl: Option<i64>,
) -> Result<(), CliError> {
    let templates_path = channel_templates::resolve_templates_path(templates_file)?;
    let template: ChannelTemplateRecord =
        channel_templates::find_template(&templates_path, template_name)?;

    let channel_type = channel_type_override.unwrap_or(&template.channel_type);
    let visibility = visibility_override.unwrap_or(&template.visibility);
    match channel_type {
        "stream" | "forum" => {}
        _ => {
            return Err(CliError::Usage(format!(
                "template channel_type must be 'stream' or 'forum' (got: {channel_type})"
            )))
        }
    }
    match visibility {
        "open" | "private" => {}
        _ => {
            return Err(CliError::Usage(format!(
                "template visibility must be 'open' or 'private' (got: {visibility})"
            )))
        }
    }
    let ttl = ttl.map(validate_ttl_seconds).transpose()?;

    // Owner invariant (F1): the auth-tag owner (already verified against the
    // signer at startup) if present, else the signing pubkey. No sole-author
    // fallback — a same-slug 30176/30177 from another principal must never
    // be selected.
    let owner = client
        .auth_tag_owner_hex()
        .unwrap_or_else(|| client.keys().public_key().to_hex());

    let resolved = resolve_template_roster(client, &owner, &template.agents).await?;

    let channel_uuid = Uuid::new_v4();
    let vis = match visibility {
        "open" => buzz_sdk::Visibility::Open,
        "private" => buzz_sdk::Visibility::Private,
        _ => unreachable!(),
    };
    let ct = match channel_type {
        "stream" => buzz_sdk::ChannelKind::Stream,
        "forum" => buzz_sdk::ChannelKind::Forum,
        _ => unreachable!(),
    };
    let effective_description = description.or(template.description.as_deref());
    let builder = buzz_sdk::build_create_channel(
        channel_uuid,
        name,
        Some(vis),
        Some(ct),
        effective_description,
        ttl,
    )
    .map_err(|e| CliError::Other(format!("build_create_channel failed: {e}")))?;
    let event = client.sign_event(builder)?;
    client.submit_event(event).await?;

    let mut canvas_applied = false;
    if let Some(canvas_template) = template.canvas_template.as_deref() {
        let content = canvas_template
            .replace("{channel.name}", name)
            .replace("{template.name}", &template.name);
        let canvas_result: Result<(), CliError> = async {
            let builder = buzz_sdk::build_set_canvas(channel_uuid, &content)
                .map_err(|e| CliError::Other(format!("build_set_canvas failed: {e}")))?;
            let event = client.sign_event(builder)?;
            client.submit_event(event).await?;
            Ok(())
        }
        .await;
        // Canvas is best-effort — matches desktop's useApplyTemplate.ts behavior.
        canvas_applied = canvas_result.is_ok();
    }

    // Members are added sequentially: concurrent kind:9000 writes are
    // last-write-wins on the relay (see channelAgents.ts), so parallel adds
    // here would race each other for no benefit.
    let mut members_added: Vec<serde_json::Value> = Vec::new();
    let mut member_failures: Vec<serde_json::Value> = Vec::new();
    for agent in &resolved.agents {
        let outcome: Result<(), CliError> = async {
            let builder = buzz_sdk::build_add_member(
                channel_uuid,
                &agent.pubkey,
                Some(buzz_sdk::MemberRole::Bot),
            )
            .map_err(|e| CliError::Other(format!("build_add_member failed: {e}")))?;
            let event = client.sign_event(builder)?;
            client.submit_event(event).await?;
            Ok(())
        }
        .await;
        match outcome {
            Ok(()) => members_added.push(serde_json::json!({
                "persona_id": agent.persona_id,
                "pubkey": agent.pubkey,
            })),
            Err(e) => member_failures.push(serde_json::json!({
                "persona_id": agent.persona_id,
                "pubkey": agent.pubkey,
                "error": e.to_string(),
            })),
        }
    }

    let status = if member_failures.is_empty() {
        "ok"
    } else {
        "partial"
    };
    let report = serde_json::json!({
        "status": status,
        "channel_id": channel_uuid.to_string(),
        "template": template.name,
        "canvas_applied": canvas_applied,
        "members_added": members_added,
        "skipped": resolved.skipped,
        "member_failures": member_failures,
    });
    println!("{report}");
    Ok(())
}

/// Validate a user-supplied TTL (in seconds): must be a positive value that
/// fits in the relay's `i32` column.
fn validate_ttl_seconds(secs: i64) -> Result<i32, CliError> {
    if secs <= 0 {
        return Err(CliError::Usage(format!(
            "--ttl must be a positive number of seconds (got: {secs})"
        )));
    }
    i32::try_from(secs)
        .map_err(|_| CliError::Usage(format!("--ttl is too large (max {} seconds)", i32::MAX)))
}

pub async fn cmd_update_channel(
    client: &BuzzClient,
    channel_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    ttl: Option<i64>,
    no_ttl: bool,
) -> Result<(), CliError> {
    // Outer Option: None leaves TTL unchanged. Inner: Some(secs) sets it,
    // None (from --no-ttl) clears it, making the channel permanent.
    let ttl_change: Option<Option<i32>> = match (ttl, no_ttl) {
        (Some(secs), _) => Some(Some(validate_ttl_seconds(secs)?)),
        (None, true) => Some(None),
        (None, false) => None,
    };

    if name.is_none() && description.is_none() && ttl_change.is_none() {
        return Err(CliError::Usage(
            "at least one field required (--name, --description, --ttl, --no-ttl)".into(),
        ));
    }
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = buzz_sdk::build_update_channel(channel_uuid, name, description, None, ttl_change)
        .map_err(|e| CliError::Other(format!("build_update_channel failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_set_channel_topic(
    client: &BuzzClient,
    channel_id: &str,
    topic: &str,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = buzz_sdk::build_set_topic(channel_uuid, topic)
        .map_err(|e| CliError::Other(format!("build_set_topic failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_set_channel_purpose(
    client: &BuzzClient,
    channel_id: &str,
    purpose: &str,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = buzz_sdk::build_set_purpose(channel_uuid, purpose)
        .map_err(|e| CliError::Other(format!("build_set_purpose failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_join_channel(client: &BuzzClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = buzz_sdk::build_join(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_join failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_leave_channel(client: &BuzzClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = buzz_sdk::build_leave(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_leave failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_archive_channel(client: &BuzzClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = buzz_sdk::build_archive(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_archive failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_unarchive_channel(client: &BuzzClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = buzz_sdk::build_unarchive(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_unarchive failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_delete_channel(client: &BuzzClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = buzz_sdk::build_delete_channel(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_delete_channel failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_add_channel_member(
    client: &BuzzClient,
    channel_id: &str,
    pubkey: &str,
    role: Option<&str>,
) -> Result<(), CliError> {
    validate_hex64(pubkey)?;
    let channel_uuid = parse_uuid(channel_id)?;

    let typed_role = match role {
        None => None,
        Some("owner") => Some(buzz_sdk::MemberRole::Owner),
        Some("admin") => Some(buzz_sdk::MemberRole::Admin),
        Some("member") => Some(buzz_sdk::MemberRole::Member),
        Some("guest") => Some(buzz_sdk::MemberRole::Guest),
        Some("bot") => Some(buzz_sdk::MemberRole::Bot),
        Some(other) => {
            return Err(CliError::Usage(format!(
                "--role must be owner/admin/member/guest/bot (got: {other})"
            )))
        }
    };
    let builder = buzz_sdk::build_add_member(channel_uuid, pubkey, typed_role)
        .map_err(|e| CliError::Other(format!("build_add_member failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_remove_channel_member(
    client: &BuzzClient,
    channel_id: &str,
    pubkey: &str,
) -> Result<(), CliError> {
    validate_hex64(pubkey)?;
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = buzz_sdk::build_remove_member(channel_uuid, pubkey)
        .map_err(|e| CliError::Other(format!("build_remove_member failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Set the channel addition policy — sign and submit a kind:10100 (agent profile) event.
pub async fn cmd_set_add_policy(client: &BuzzClient, policy: &str) -> Result<(), CliError> {
    match policy {
        "anyone" | "owner_only" | "nobody" => {}
        _ => {
            return Err(CliError::Usage(format!(
                "--policy must be 'anyone', 'owner_only', or 'nobody' (got: {policy})"
            )))
        }
    }

    // Check if this policy is allowed by the deployment.
    // NOTE: This gate covers only the `buzz channels set-add-policy` CLI path.
    // A client that submits a kind:10100 event directly to the relay bypasses
    // this check. Full enforcement requires relay-side validation, which is
    // intentionally out of scope for this change (see team decision: no
    // relay-side enforcement of client behavior).
    if let Ok(allowed_raw) = std::env::var("BUZZ_ACP_ALLOWED_CHANNEL_ADD_POLICIES") {
        let allowed: Vec<&str> = allowed_raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if !allowed.is_empty() && !allowed.contains(&policy) {
            return Err(CliError::Usage(format!(
                "channel_add_policy '{policy}' is not permitted on this deployment \
                 (BUZZ_ACP_ALLOWED_CHANNEL_ADD_POLICIES={allowed_raw})"
            )));
        }
    }

    let content = serde_json::json!({ "channel_add_policy": policy }).to_string();
    use nostr::{EventBuilder, Kind};
    let builder = EventBuilder::new(
        Kind::Custom(buzz_sdk::kind::KIND_AGENT_PROFILE as u16),
        &content,
    )
    .tags([]);
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_set_canvas(
    client: &BuzzClient,
    channel_id: &str,
    content: &str,
) -> Result<(), CliError> {
    let content = read_or_stdin(content)?;
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = buzz_sdk::build_set_canvas(channel_uuid, &content)
        .map_err(|e| CliError::Other(format!("build_set_canvas failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn dispatch(
    cmd: crate::ChannelsCmd,
    client: &BuzzClient,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    use crate::ChannelsCmd;
    match cmd {
        ChannelsCmd::List {
            visibility,
            member,
            limit,
        } => {
            let vis_str = visibility.as_ref().map(|v| v.to_string());
            cmd_list_channels(client, vis_str.as_deref(), Some(member), limit, format).await
        }
        ChannelsCmd::Get { channel } => cmd_get_channel(client, &channel).await,
        ChannelsCmd::Search {
            query,
            exact,
            include_archived,
            limit,
        } => cmd_search_channels(client, &query, exact, include_archived, limit).await,
        ChannelsCmd::Create {
            name,
            channel_type,
            visibility,
            description,
            ttl,
            template,
            templates_file,
        } => {
            if let Some(template_name) = template {
                cmd_create_channel_from_template(
                    client,
                    &name,
                    &template_name,
                    templates_file.as_deref(),
                    channel_type.as_ref().map(|t| t.to_string()).as_deref(),
                    visibility.as_ref().map(|v| v.to_string()).as_deref(),
                    description.as_deref(),
                    ttl,
                )
                .await
            } else {
                // required_unless_present = "template" guarantees these are
                // Some when template is None.
                let channel_type =
                    channel_type.ok_or_else(|| CliError::Usage("--type is required".into()))?;
                let visibility =
                    visibility.ok_or_else(|| CliError::Usage("--visibility is required".into()))?;
                cmd_create_channel(
                    client,
                    &name,
                    &channel_type.to_string(),
                    &visibility.to_string(),
                    description.as_deref(),
                    ttl,
                )
                .await
            }
        }
        ChannelsCmd::Update {
            channel,
            name,
            description,
            ttl,
            no_ttl,
        } => {
            cmd_update_channel(
                client,
                &channel,
                name.as_deref(),
                description.as_deref(),
                ttl,
                no_ttl,
            )
            .await
        }
        ChannelsCmd::Topic { channel, topic } => {
            cmd_set_channel_topic(client, &channel, &topic).await
        }
        ChannelsCmd::Purpose { channel, purpose } => {
            cmd_set_channel_purpose(client, &channel, &purpose).await
        }
        ChannelsCmd::Join { channel } => cmd_join_channel(client, &channel).await,
        ChannelsCmd::Leave { channel } => cmd_leave_channel(client, &channel).await,
        ChannelsCmd::Archive { channel } => cmd_archive_channel(client, &channel).await,
        ChannelsCmd::Unarchive { channel } => cmd_unarchive_channel(client, &channel).await,
        ChannelsCmd::Delete { channel } => cmd_delete_channel(client, &channel).await,
        ChannelsCmd::Members { channel } => cmd_list_channel_members(client, &channel).await,
        ChannelsCmd::AddMember {
            channel,
            pubkey,
            role,
        } => cmd_add_channel_member(client, &channel, &pubkey, role.as_deref()).await,
        ChannelsCmd::RemoveMember { channel, pubkey } => {
            cmd_remove_channel_member(client, &channel, &pubkey).await
        }
        ChannelsCmd::SetAddPolicy { policy } => cmd_set_add_policy(client, &policy).await,
    }
}

pub async fn dispatch_canvas(cmd: crate::CanvasCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::CanvasCmd;
    match cmd {
        CanvasCmd::Get { channel } => cmd_get_canvas(client, &channel).await,
        CanvasCmd::Set { channel, content } => cmd_set_canvas(client, &channel, &content).await,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_cardinality_rule, cmd_set_add_policy, name_matches, validate_ttl_seconds,
        ChannelSummary, ResolvedAgent,
    };
    use crate::client::BuzzClient;
    use crate::CliError;
    use serde_json::json;

    fn event(tags: serde_json::Value) -> serde_json::Value {
        json!({ "tags": tags })
    }

    #[test]
    fn from_event_extracts_known_tags() {
        let ev = event(json!([
            ["d", "11111111-1111-1111-1111-111111111111"],
            ["name", "buzz-chat-composer"],
            ["t", "stream"],
            ["public"],
            ["about", "About text"],
            ["topic", "Composer work"],
            ["purpose", "Track UI for the composer"],
        ]));
        let s = ChannelSummary::from_event(&ev).expect("parse");
        assert_eq!(s.channel_id, "11111111-1111-1111-1111-111111111111");
        assert_eq!(s.name, "buzz-chat-composer");
        assert_eq!(s.channel_type.as_deref(), Some("stream"));
        assert_eq!(s.visibility.as_deref(), Some("public"));
        assert!(!s.archived);
        assert_eq!(s.about.as_deref(), Some("About text"));
        assert_eq!(s.topic.as_deref(), Some("Composer work"));
        assert_eq!(s.purpose.as_deref(), Some("Track UI for the composer"));
    }

    #[test]
    fn from_event_marks_archived() {
        let ev = event(json!([
            ["d", "11111111-1111-1111-1111-111111111111"],
            ["name", "old-channel"],
            ["archived", "true"],
        ]));
        let s = ChannelSummary::from_event(&ev).expect("parse");
        assert!(s.archived);
    }

    #[test]
    fn from_event_marks_private() {
        let ev = event(json!([
            ["d", "11111111-1111-1111-1111-111111111111"],
            ["name", "secret"],
            ["private"],
        ]));
        let s = ChannelSummary::from_event(&ev).expect("parse");
        assert_eq!(s.visibility.as_deref(), Some("private"));
    }

    #[test]
    fn from_event_returns_none_without_required_tags() {
        // missing `name`
        let ev = event(json!([["d", "11111111-1111-1111-1111-111111111111"]]));
        assert!(ChannelSummary::from_event(&ev).is_none());
        // missing `d`
        let ev = event(json!([["name", "no-id"]]));
        assert!(ChannelSummary::from_event(&ev).is_none());
    }

    #[test]
    fn from_event_tolerates_malformed_tags() {
        // Non-array tag entry, empty tag, single-element tag — all must be skipped, not panic.
        let ev = event(json!([
            "not-an-array",
            [],
            ["name"],
            ["d", "11111111-1111-1111-1111-111111111111"],
            ["name", "fine"],
        ]));
        let s = ChannelSummary::from_event(&ev).expect("parse");
        assert_eq!(s.name, "fine");
    }

    // `name_matches` takes a pre-lowercased needle (caller responsibility, set in
    // cmd_search_channels). Tests follow the same contract.

    #[test]
    fn name_matches_substring_case_insensitive() {
        assert!(name_matches("Buzz-Chat-Composer", "composer", false));
        assert!(name_matches("Buzz-Chat-Composer", "buzz", false));
        assert!(!name_matches("design", "composer", false));
    }

    #[test]
    fn name_matches_exact_case_insensitive() {
        assert!(name_matches("Buzz", "buzz", true));
        assert!(!name_matches("Buzz-Chat", "buzz", true));
    }

    #[test]
    fn validate_ttl_accepts_positive() {
        assert_eq!(validate_ttl_seconds(3600).unwrap(), 3600);
        assert_eq!(validate_ttl_seconds(1).unwrap(), 1);
        assert_eq!(validate_ttl_seconds(i32::MAX as i64).unwrap(), i32::MAX);
    }

    #[test]
    fn validate_ttl_rejects_zero_and_negative() {
        assert!(validate_ttl_seconds(0).is_err());
        assert!(validate_ttl_seconds(-1).is_err());
    }

    #[test]
    fn validate_ttl_rejects_overflow() {
        assert!(validate_ttl_seconds(i32::MAX as i64 + 1).is_err());
    }

    // --- BUZZ_ACP_ALLOWED_CHANNEL_ADD_POLICIES gate ---

    fn check_allowed_channel_add_policy(allowed_raw: &str, policy: &str) -> Result<(), CliError> {
        let allowed: Vec<&str> = allowed_raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if !allowed.is_empty() && !allowed.contains(&policy) {
            return Err(CliError::Usage(format!(
                "channel_add_policy '{policy}' is not permitted on this deployment \
                 (BUZZ_ACP_ALLOWED_CHANNEL_ADD_POLICIES={allowed_raw})"
            )));
        }
        Ok(())
    }

    #[test]
    fn set_add_policy_rejects_disallowed_policy() {
        let result = check_allowed_channel_add_policy("owner_only,nobody", "anyone");
        assert!(
            result.is_err(),
            "anyone should be rejected when not in allowed set"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not permitted"),
            "error should mention 'not permitted': {msg}"
        );
        assert!(
            msg.contains("anyone"),
            "error should name the disallowed policy: {msg}"
        );
    }

    #[test]
    fn set_add_policy_accepts_allowed_policy() {
        let result = check_allowed_channel_add_policy("owner_only,nobody", "owner_only");
        assert!(result.is_ok(), "owner_only should be accepted: {result:?}");
    }

    #[test]
    fn set_add_policy_no_restriction_allows_all() {
        // Empty allowed list means no restriction.
        let result = check_allowed_channel_add_policy("", "anyone");
        assert!(
            result.is_ok(),
            "empty allowed list should permit any policy: {result:?}"
        );
    }

    // --- Integration test: full env-var → cmd_set_add_policy() path ---
    //
    // This test calls cmd_set_add_policy directly with the env var set. The function
    // returns early with an error before any network call, so no relay is needed.
    // If the BUZZ_ACP_ALLOWED_CHANNEL_ADD_POLICIES check were removed from cmd_set_add_policy,
    // this test would fail (it would proceed to sign_event and return a different error).

    fn make_test_client() -> BuzzClient {
        // Scalar = 1 is the smallest valid secp256k1 private key.
        let keys =
            nostr::Keys::parse("0000000000000000000000000000000000000000000000000000000000000001")
                .expect("valid test key");
        BuzzClient::new("ws://localhost:3000".to_string(), keys, None, None)
            .expect("client construction should not fail")
    }

    #[tokio::test]
    async fn set_add_policy_env_gate_rejects_disallowed_via_full_path() {
        std::env::set_var("BUZZ_ACP_ALLOWED_CHANNEL_ADD_POLICIES", "owner_only,nobody");
        let client = make_test_client();
        let result = cmd_set_add_policy(&client, "anyone").await;
        std::env::remove_var("BUZZ_ACP_ALLOWED_CHANNEL_ADD_POLICIES");

        assert!(
            result.is_err(),
            "cmd_set_add_policy should reject 'anyone' when not in allowed set"
        );
        match result.unwrap_err() {
            crate::CliError::Usage(msg) => {
                assert!(
                    msg.contains("not permitted"),
                    "error should mention 'not permitted': {msg}"
                );
            }
            other => panic!("expected CliError::Usage, got {other:?}"),
        }
    }

    // --- Template roster cardinality (F4) ---

    fn agent(persona_id: &str, pubkey: &str) -> ResolvedAgent {
        ResolvedAgent {
            persona_id: persona_id.to_string(),
            pubkey: pubkey.to_string(),
        }
    }

    #[test]
    fn cardinality_zero_instances_is_skipped_not_error() {
        let slugs = vec!["builtin:fizz".to_string()];
        let resolved = apply_cardinality_rule(&slugs, &[]).expect("zero instances is not fatal");
        assert!(resolved.agents.is_empty());
        assert_eq!(resolved.skipped, vec!["builtin:fizz".to_string()]);
    }

    #[test]
    fn cardinality_one_instance_is_added() {
        let slugs = vec!["builtin:fizz".to_string()];
        let found = vec![agent("builtin:fizz", "a".repeat(64).as_str())];
        let resolved = apply_cardinality_rule(&slugs, &found).expect("single instance resolves");
        assert_eq!(resolved.agents.len(), 1);
        assert_eq!(resolved.agents[0].persona_id, "builtin:fizz");
        assert!(resolved.skipped.is_empty());
    }

    #[test]
    fn cardinality_multiple_instances_is_hard_error_listing_candidates() {
        let slugs = vec!["builtin:fizz".to_string()];
        let found = vec![
            agent("builtin:fizz", &"a".repeat(64)),
            agent("builtin:fizz", &"b".repeat(64)),
        ];
        let err = apply_cardinality_rule(&slugs, &found).unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
        let msg = err.to_string();
        assert!(msg.contains("builtin:fizz"));
        assert!(msg.contains(&"a".repeat(64)));
        assert!(msg.contains(&"b".repeat(64)));
    }

    #[test]
    fn cardinality_mixed_slugs_zero_one_many_reports_first_ambiguity() {
        // Zero and one resolve fine on their own, but a hard error on any
        // slug must abort the whole roster (no partial channel-creation
        // side effects from this stage) — the error must name the
        // ambiguous slug, not a co-resolved one.
        let slugs = vec![
            "builtin:no-instance".to_string(),
            "builtin:fizz".to_string(),
            "builtin:duplicated".to_string(),
        ];
        let found = vec![
            agent("builtin:fizz", &"a".repeat(64)),
            agent("builtin:duplicated", &"b".repeat(64)),
            agent("builtin:duplicated", &"c".repeat(64)),
        ];
        let err = apply_cardinality_rule(&slugs, &found).unwrap_err();
        assert!(err.to_string().contains("builtin:duplicated"));
    }

    #[test]
    fn cardinality_empty_roster_resolves_to_empty_lists() {
        let resolved = apply_cardinality_rule(&[], &[]).expect("empty roster is not fatal");
        assert!(resolved.agents.is_empty());
        assert!(resolved.skipped.is_empty());
    }

    #[test]
    fn cardinality_ignores_instances_for_unrelated_slugs() {
        // A found agent for a slug that isn't in this roster must not leak
        // into the resolved set or affect another slug's cardinality.
        let slugs = vec!["builtin:fizz".to_string()];
        let found = vec![
            agent("builtin:fizz", &"a".repeat(64)),
            agent("builtin:unrelated", &"z".repeat(64)),
        ];
        let resolved = apply_cardinality_rule(&slugs, &found).expect("resolves");
        assert_eq!(resolved.agents.len(), 1);
        assert_eq!(resolved.agents[0].persona_id, "builtin:fizz");
    }
}
