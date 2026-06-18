use buzz_sdk::{DiffMeta, ThreadRef, VoteDirection};
use nostr::PublicKey;
use uuid::Uuid;

use crate::client::{normalize_events, normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{
    infer_language, parse_event_id, parse_uuid, read_or_stdin, truncate_diff,
    validate_content_size, validate_hex64, validate_uuid, MAX_DIFF_BYTES,
};
use buzz_sdk::mentions::{
    extract_at_mentions_with_known, extract_nostr_uris, merge_mentions, strip_code_regions,
    MENTION_CAP,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the thread root event ID from a Nostr tag array.
///
/// Parses `"e"` tags with NIP-10 markers:
/// - If a `"root"` marker exists, returns that event ID.
/// - Otherwise, if only a `"reply"` marker exists, returns the reply target
///   (a direct reply's parent IS the root, and nested replies need that root
///   to thread correctly).
/// - If no thread markers exist, returns `None` (parent is a top-level message,
///   so it is itself the root).
fn find_root_from_tags(tags: &serde_json::Value) -> Option<String> {
    fn valid_event_id(s: &str) -> bool {
        s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
    }
    let arr = tags.as_array()?;
    let mut root = None;
    let mut reply = None;
    for tag in arr {
        let Some(parts) = tag.as_array() else {
            continue;
        };
        if parts.len() >= 4 && parts[0].as_str() == Some("e") {
            // Defensively ignore malformed marker values so a bad tag on the
            // parent event can't block the reply — fall back to root == parent.
            let id = parts[1].as_str().filter(|s| valid_event_id(s));
            match (parts[3].as_str(), id) {
                (Some("root"), Some(id)) => root = Some(id.to_string()),
                (Some("reply"), Some(id)) => reply = Some(id.to_string()),
                _ => {}
            }
        }
    }
    root.or(reply)
}

/// Build a `ThreadRef` for a reply, given the immediate parent's event ID.
///
/// Fetches the parent event from the relay and inspects its NIP-10 `e` tags to
/// determine the thread root:
/// - Direct reply (parent is top-level): `root == parent`.
/// - Nested reply: `root` is the parent's own root marker; `parent` is unchanged.
///
/// Ensures CLI-sent replies thread correctly using the same NIP-10 logic.
async fn resolve_thread_ref(
    client: &BuzzClient,
    parent_event_id: &str,
) -> Result<ThreadRef, CliError> {
    let parent_eid = parse_event_id(parent_event_id)?;
    let filter = serde_json::json!({ "ids": [parent_event_id], "limit": 1 });
    let raw = client.query(&filter).await?;
    let events: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse query response: {e}")))?;
    let event = events
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| CliError::Other(format!("parent event {parent_event_id} not found")))?;
    let tags = event
        .get("tags")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let root_eid = match find_root_from_tags(&tags) {
        Some(root_hex) if root_hex != parent_event_id => parse_event_id(&root_hex)?,
        _ => parent_eid,
    };

    Ok(ThreadRef {
        root_event_id: root_eid,
        parent_event_id: parent_eid,
    })
}

/// Resolve the channel UUID for an event by querying for it via POST /query.
/// Extracts the `h` tag value from the returned event's tags.
async fn resolve_channel_id(client: &BuzzClient, event_id: &str) -> Result<Uuid, CliError> {
    let filter = serde_json::json!({
        "ids": [event_id]
    });
    let raw = client.query(&filter).await?;
    let events: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse query response: {e}")))?;
    let arr = events
        .as_array()
        .ok_or_else(|| CliError::Other("query response is not an array".into()))?;
    let event = arr
        .first()
        .ok_or_else(|| CliError::Other(format!("event {event_id} not found")))?;
    let tags = event
        .get("tags")
        .and_then(|t| t.as_array())
        .ok_or_else(|| CliError::Other("event missing 'tags' field".into()))?;
    for tag in tags {
        if let Some(arr) = tag.as_array() {
            if arr.first().and_then(|v| v.as_str()) == Some("h") {
                if let Some(uuid_str) = arr.get(1).and_then(|v| v.as_str()) {
                    return Uuid::parse_str(uuid_str).map_err(|_| {
                        CliError::Other(format!("event h-tag is not a valid UUID: {uuid_str}"))
                    });
                }
            }
        }
    }
    Err(CliError::Other(format!(
        "event {event_id} has no h-tag — cannot determine channel"
    )))
}

/// Resolve `@name` mentions in `content` against this channel's members.
///
/// Queries kind 39002 (channel members) then kind 0 (profiles), parses
/// display names once, and feeds them to [`extract_at_mentions_with_known`]
/// for multi-word matching. On any I/O or parse failure, returns an empty
/// vec — auto-tagging is best-effort and must never block a send.
async fn resolve_content_mentions(
    client: &BuzzClient,
    channel_id: &str,
    content: &str,
) -> Vec<String> {
    if !content.contains('@') {
        return vec![];
    }

    // 1. Membership list (kind 39002 is parameterized-replaceable, addressed by `d` tag).
    let members_filter = serde_json::json!({
        "kinds": [39002],
        "#d": [channel_id],
        "limit": 1,
    });
    let member_pubkeys = match fetch_member_pubkeys(client, &members_filter).await {
        Some(pks) if !pks.is_empty() => pks,
        _ => return vec![],
    };

    // 2. Profiles for those members (kind 0).
    let profiles_filter = serde_json::json!({
        "kinds": [0],
        "authors": member_pubkeys,
        "limit": member_pubkeys.len(),
    });
    let profile_events = match fetch_events(client, &profiles_filter).await {
        Some(v) => v,
        None => return vec![],
    };

    // 3. Single parse: extract (pubkey, display_name) pairs from profile JSON.
    let mut name_to_pubkeys: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut display_names: Vec<String> = Vec::new();
    for e in &profile_events {
        let Some(pubkey) = e.get("pubkey").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(content_json) = e.get("content").and_then(|v| v.as_str()) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(content_json) else {
            continue;
        };
        let Some(name) = v
            .get("display_name")
            .or_else(|| v.get("name"))
            .and_then(|n| n.as_str())
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        name_to_pubkeys
            .entry(lower)
            .or_default()
            .push(pubkey.to_string());
        display_names.push(name.to_string());
    }

    // 4. Two-pass extraction: known multi-word names first, single-word fallback.
    let known_refs: Vec<&str> = display_names.iter().map(|s| s.as_str()).collect();
    let names = extract_at_mentions_with_known(content, &known_refs);

    // 5. Look up matched names → pubkeys via the map we already built.
    names
        .iter()
        .flat_map(|n| name_to_pubkeys.get(n).into_iter().flatten())
        .cloned()
        .collect()
}

/// Fetch raw events for `filter` via the relay's `/query` endpoint.
/// Returns `None` on any I/O or parse failure.
async fn fetch_events(
    client: &BuzzClient,
    filter: &serde_json::Value,
) -> Option<Vec<serde_json::Value>> {
    let raw = client.query(filter).await.ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    parsed.as_array().cloned()
}

/// Extract member pubkeys (the `p` tag values) from a single 39002 event.
async fn fetch_member_pubkeys(
    client: &BuzzClient,
    filter: &serde_json::Value,
) -> Option<Vec<String>> {
    let events = fetch_events(client, filter).await?;
    Some(parse_member_pubkeys(events.first()?))
}

/// Parse member pubkeys from a kind 39002 event JSON value.
///
/// Filters and canonicalizes via `nostr::PublicKey::from_hex` — matching
/// MCP's typed-Nostr behavior so both surfaces accept exactly the same
/// pubkeys. Pure helper, split out for testing.
fn parse_member_pubkeys(event: &serde_json::Value) -> Vec<String> {
    let Some(tags) = event.get("tags").and_then(|t| t.as_array()) else {
        return vec![];
    };
    tags.iter()
        .filter_map(|t| {
            let arr = t.as_array()?;
            if arr.first()?.as_str()? != "p" {
                return None;
            }
            let pk = arr.get(1)?.as_str()?;
            PublicKey::from_hex(pk).ok().map(|k| k.to_hex())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Read commands — POST /query
// ---------------------------------------------------------------------------

fn format_events(normalized: &str, format: &crate::OutputFormat) -> String {
    match format {
        crate::OutputFormat::Compact => {
            let events: Vec<serde_json::Value> =
                serde_json::from_str(normalized).unwrap_or_default();
            let compact: Vec<serde_json::Value> = events
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.get("id").cloned().unwrap_or_default(),
                        "content": e.get("content").cloned().unwrap_or_default(),
                        "created_at": e.get("created_at").cloned().unwrap_or_default(),
                    })
                })
                .collect();
            serde_json::to_string(&compact).unwrap_or_default()
        }
        crate::OutputFormat::Json => normalized.to_string(),
    }
}

pub async fn cmd_get_messages(
    client: &BuzzClient,
    channel_id: &str,
    limit: Option<u32>,
    before: Option<i64>,
    since: Option<i64>,
    kinds: Option<&str>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let limit = limit.unwrap_or(50).min(200);

    let mut filter = serde_json::json!({
        "kinds": [9, 40002, 40008, 45001, 45003],
        "#h": [channel_id],
        "limit": limit
    });

    // If specific kinds requested, override
    if let Some(k) = kinds {
        let kind_list: Vec<u64> = k.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if !kind_list.is_empty() {
            filter["kinds"] = serde_json::json!(kind_list);
        }
    }

    if let Some(b) = before {
        filter["until"] = serde_json::json!(b);
    }
    if let Some(s) = since {
        filter["since"] = serde_json::json!(s);
    }

    let resp = client.query(&filter).await?;
    let mut events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    events.sort_by_key(|e| e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0));
    let normalized = normalize_events(&events);
    println!("{}", format_events(&normalized, format));
    Ok(())
}

pub async fn cmd_get_thread(
    client: &BuzzClient,
    channel_id: &str,
    event_id: &str,
    limit: Option<u32>,
    depth_limit: Option<u32>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    validate_hex64(event_id)?;
    let limit = limit.unwrap_or(100).min(500);

    // Two filters ORed in a single HTTP call:
    // 1. Replies referencing this event via e-tag (no kind restriction)
    // 2. The root event itself by ID
    let mut reply_filter = serde_json::json!({
        "kinds": [9, 40002, 40003, 40008, 45003],
        "#h": [channel_id],
        "#e": [event_id],
        "limit": limit
    });
    if let Some(d) = depth_limit {
        reply_filter["depth_limit"] = serde_json::json!(d);
    }
    let root_filter = serde_json::json!({
        "ids": [event_id],
        "limit": 1
    });
    let resp = client.query_multi(&[reply_filter, root_filter]).await?;
    let mut events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    events.sort_by_key(|e| e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0));
    let normalized = normalize_events(&events);
    println!("{}", format_events(&normalized, format));
    Ok(())
}

pub async fn cmd_search(
    client: &BuzzClient,
    query: &str,
    limit: Option<u32>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    let limit = limit.unwrap_or(20).min(100);
    let filter = serde_json::json!({
        "kinds": [9, 40002, 45001, 45003],
        "search": query,
        "limit": limit
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let normalized = normalize_events(&events);
    println!("{}", format_events(&normalized, format));
    Ok(())
}

// ---------------------------------------------------------------------------
// Write commands — signed events via POST /events
// ---------------------------------------------------------------------------

pub struct SendMessageParams {
    pub channel_id: String,
    pub content: String,
    pub kind: Option<u16>,
    pub reply_to: Option<String>,
    pub broadcast: bool,
    pub files: Vec<String>,
}

pub async fn cmd_send_message(
    client: &BuzzClient,
    mut p: SendMessageParams,
) -> Result<(), CliError> {
    // Allow '-' to read content from stdin. This keeps callers from having to
    // jam shell-metacharacter-heavy text (backticks, $vars, etc.) through argv
    // quoting — the source of countless self-inflicted command-substitution
    // bugs for agent and human users alike.
    p.content = read_or_stdin(&p.content)?;
    validate_content_size(&p.content)?;
    if let Some(ref r) = p.reply_to {
        validate_hex64(r)?;
    }
    let channel_uuid = parse_uuid(&p.channel_id)?;

    // Upload files and build imeta tags
    let mut media_tags: Vec<Vec<String>> = Vec::new();
    let mut media_content = String::new();
    for file_path in &p.files {
        let desc = client
            .upload_file(file_path)
            .await
            .map_err(|e| CliError::Other(format!("upload failed for {file_path}: {e}")))?;
        media_tags.push(crate::client::build_imeta_tag(&desc));
        if desc.mime_type.starts_with("video/") {
            media_content.push_str("\n![video](");
        } else {
            media_content.push_str("\n![image](");
        }
        media_content.push_str(&desc.url);
        media_content.push(')');
    }
    let final_content = if media_content.is_empty() {
        p.content.clone()
    } else {
        format!("{}{media_content}", p.content)
    };

    // Build thread ref if replying. `--reply-to` is the immediate parent; the
    // thread root is derived from the parent's NIP-10 tags via the relay.
    let thread_ref = if let Some(ref r) = p.reply_to {
        Some(resolve_thread_ref(client, r).await?)
    } else {
        None
    };

    // Resolve @name mentions in the author-written body only — not the media markdown we
    // append above, which is derived from upload metadata and can't carry `@names`.
    let mut auto_resolved = resolve_content_mentions(client, &p.channel_id, &p.content).await;

    // NIP-27: also extract nostr:npub1… inline references (skipping code regions)
    let stripped = strip_code_regions(&p.content);
    let uri_pubkeys = extract_nostr_uris(&stripped);
    merge_mentions(&mut auto_resolved, &uri_pubkeys, MENTION_CAP);

    let mention_refs: Vec<&str> = auto_resolved.iter().map(|s| s.as_str()).collect();

    let builder = match p.kind {
        Some(45001) => {
            buzz_sdk::build_forum_post(channel_uuid, &final_content, &mention_refs, &media_tags)
                .map_err(|e| CliError::Other(format!("build_forum_post failed: {e}")))?
        }
        Some(45003) => {
            let tr = thread_ref.as_ref().ok_or_else(|| {
                CliError::Usage("--reply-to is required for forum comments (kind 45003)".into())
            })?;
            buzz_sdk::build_forum_comment(
                channel_uuid,
                &final_content,
                tr,
                &mention_refs,
                &media_tags,
            )
            .map_err(|e| CliError::Other(format!("build_forum_comment failed: {e}")))?
        }
        None | Some(9) => buzz_sdk::build_message(
            channel_uuid,
            &final_content,
            thread_ref.as_ref(),
            &mention_refs,
            p.broadcast,
            &media_tags,
        )
        .map_err(|e| CliError::Other(format!("build_message failed: {e}")))?,
        Some(k) => {
            return Err(CliError::Usage(format!(
                "--kind {k} is not supported (use 9, 45001, or 45003)"
            )))
        }
    };

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub struct SendDiffParams {
    pub channel_id: String,
    pub diff: String,
    pub repo_url: String,
    pub commit_sha: String,
    pub file_path: Option<String>,
    pub parent_commit_sha: Option<String>,
    pub source_branch: Option<String>,
    pub target_branch: Option<String>,
    pub pr_number: Option<u32>,
    pub language: Option<String>,
    pub description: Option<String>,
    pub reply_to: Option<String>,
}

pub async fn cmd_send_diff_message(client: &BuzzClient, p: SendDiffParams) -> Result<(), CliError> {
    if let Some(r) = &p.reply_to {
        validate_hex64(r)?;
    }

    // Branch pairing: both or neither
    match (&p.source_branch, &p.target_branch) {
        (Some(_), None) | (None, Some(_)) => {
            return Err(CliError::Usage(
                "--source-branch and --target-branch must both be provided or both omitted".into(),
            ));
        }
        _ => {}
    }

    let channel_uuid = parse_uuid(&p.channel_id)?;

    // Read diff from stdin if "--diff -"
    let diff_content = read_or_stdin(&p.diff)?;

    // Truncate at 60 KiB hunk boundary
    let (diff, truncated) = truncate_diff(&diff_content, MAX_DIFF_BYTES);

    // Language inference: explicit flag wins, then infer from file path
    let language = p
        .language
        .clone()
        .or_else(|| p.file_path.as_deref().and_then(infer_language));

    // NIP-31 alt tag
    let alt = match (&p.file_path, &p.description) {
        (Some(fp), Some(desc)) => format!("Diff: {} — {}", fp, desc),
        (Some(fp), None) => format!("Diff: {}", fp),
        _ => "Diff".to_string(),
    };

    // `--reply-to` is the immediate parent; the thread root is derived from
    // the parent's NIP-10 tags via the relay.
    let thread_ref = if let Some(r) = &p.reply_to {
        Some(resolve_thread_ref(client, r).await?)
    } else {
        None
    };

    let branch = match (&p.source_branch, &p.target_branch) {
        (Some(src), Some(tgt)) => Some((src.clone(), tgt.clone())),
        _ => None,
    };

    let diff_meta = DiffMeta {
        repo_url: p.repo_url.clone(),
        commit_sha: p.commit_sha.clone(),
        file_path: p.file_path.clone(),
        parent_commit: p.parent_commit_sha.clone(),
        branch,
        pr_number: p.pr_number,
        language,
        description: p.description.clone(),
        truncated,
        alt_text: Some(alt),
    };

    let builder =
        buzz_sdk::build_diff_message(channel_uuid, &diff, &diff_meta, thread_ref.as_ref())
            .map_err(|e| CliError::Other(format!("build_diff_message failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_delete_message(client: &BuzzClient, event_id: &str) -> Result<(), CliError> {
    validate_hex64(event_id)?;

    // Resolve channel_id from the event's h-tag
    let channel_uuid = resolve_channel_id(client, event_id).await?;
    let target_eid = parse_event_id(event_id)?;

    let builder = buzz_sdk::build_delete_message(channel_uuid, target_eid)
        .map_err(|e| CliError::Other(format!("build_delete_message failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Edit a message you previously sent.
pub async fn cmd_edit_message(
    client: &BuzzClient,
    event_id: &str,
    content: &str,
) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    validate_content_size(content)?;

    // Resolve channel_id from the event's h-tag
    let channel_uuid = resolve_channel_id(client, event_id).await?;
    let target_eid = parse_event_id(event_id)?;

    let builder = buzz_sdk::build_edit(channel_uuid, target_eid, content)
        .map_err(|e| CliError::Other(format!("build_edit failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Vote on a forum post or comment.
pub async fn cmd_vote_on_post(
    client: &BuzzClient,
    event_id: &str,
    direction: &str,
) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    let vote_dir = match direction {
        "up" => VoteDirection::Up,
        "down" => VoteDirection::Down,
        _ => {
            return Err(CliError::Usage(format!(
                "--direction must be 'up' or 'down' (got: {direction})"
            )))
        }
    };

    // Resolve channel_id from the event's h-tag
    let channel_uuid = resolve_channel_id(client, event_id).await?;
    let target_eid = parse_event_id(event_id)?;

    let builder = buzz_sdk::build_vote(channel_uuid, target_eid, vote_dir)
        .map_err(|e| CliError::Other(format!("build_vote failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub async fn dispatch(
    cmd: crate::MessagesCmd,
    client: &BuzzClient,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    use crate::MessagesCmd;
    match cmd {
        MessagesCmd::Send {
            channel,
            content,
            kind,
            reply_to,
            broadcast,
            files,
        } => {
            cmd_send_message(
                client,
                SendMessageParams {
                    channel_id: channel,
                    content,
                    kind,
                    reply_to,
                    broadcast,
                    files,
                },
            )
            .await
        }
        MessagesCmd::SendDiff {
            channel,
            diff,
            repo,
            commit,
            file,
            parent_commit,
            source_branch,
            target_branch,
            pr,
            lang,
            description,
            reply_to,
        } => {
            cmd_send_diff_message(
                client,
                SendDiffParams {
                    channel_id: channel,
                    diff,
                    repo_url: repo,
                    commit_sha: commit,
                    file_path: file,
                    parent_commit_sha: parent_commit,
                    source_branch,
                    target_branch,
                    pr_number: pr,
                    language: lang,
                    description,
                    reply_to,
                },
            )
            .await
        }
        MessagesCmd::Edit { event, content } => cmd_edit_message(client, &event, &content).await,
        MessagesCmd::Delete { event } => cmd_delete_message(client, &event).await,
        MessagesCmd::Get {
            channel,
            limit,
            before,
            since,
            kinds,
        } => {
            cmd_get_messages(
                client,
                &channel,
                limit,
                before,
                since,
                kinds.as_deref(),
                format,
            )
            .await
        }
        MessagesCmd::Thread {
            channel,
            event,
            limit,
            depth_limit,
        } => cmd_get_thread(client, &channel, &event, limit, depth_limit, format).await,
        MessagesCmd::Search { query, limit } => cmd_search(client, &query, limit, format).await,
        MessagesCmd::Vote { event, direction } => {
            cmd_vote_on_post(client, &event, &direction).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{find_root_from_tags, parse_member_pubkeys};
    use buzz_sdk::mentions::{
        extract_at_mentions_with_known, extract_at_names, match_names_to_profiles, MentionProfile,
    };
    use serde_json::json;

    const ID_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ID_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const PUBKEY: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    // Three real pubkeys (lowercase 64-char hex) used by parse_member_pubkeys tests.
    // See the test's own comment on what `PublicKey::from_hex` actually validates.
    const PK_VALID_A: &str = "35c18ae273fccfaf80d629e20e7f8721b90499379addff533054acc2504c12b4";
    const PK_VALID_B: &str = "c6237ef84fa537c78dcee78efd2d4e59f728859c7f194da42ac51ededfa0be05";
    const PK_VALID_C: &str = "f4a42a97e594b77bdbd8ee35191c8b28a94a4cb871d96f32921558275421fb68";

    #[test]
    fn root_marker_wins_over_reply_marker() {
        let tags = json!([
            ["e", ID_A, "", "root"],
            ["e", ID_B, "", "reply"],
            ["p", PUBKEY],
        ]);
        assert_eq!(find_root_from_tags(&tags).as_deref(), Some(ID_A));
    }

    #[test]
    fn reply_only_falls_back_to_reply_target() {
        // Direct reply to a top-level message — the parent's only e-tag is a
        // "reply" marker pointing at it; treat the reply target as the root.
        let tags = json!([["e", ID_B, "", "reply"], ["p", PUBKEY],]);
        assert_eq!(find_root_from_tags(&tags).as_deref(), Some(ID_B));
    }

    #[test]
    fn no_thread_markers_returns_none() {
        let tags = json!([["p", PUBKEY], ["h", "channel-uuid"],]);
        assert!(find_root_from_tags(&tags).is_none());
    }

    #[test]
    fn unmarked_e_tag_ignored() {
        // NIP-10 deprecated positional markers; ignore e-tags lacking an
        // explicit "root"/"reply" marker rather than guessing.
        let tags = json!([["e", ID_A], ["e", ID_B, ""],]);
        assert!(find_root_from_tags(&tags).is_none());
    }

    #[test]
    fn malformed_tags_are_skipped() {
        let tags = json!([
            "not-an-array",
            ["e"],
            ["e", "short"],
            ["e", ID_A, "", "root"],
        ]);
        assert_eq!(find_root_from_tags(&tags).as_deref(), Some(ID_A));
    }

    #[test]
    fn malformed_marker_id_is_ignored() {
        // Parent event has a "root" marker whose value isn't a valid 64-hex
        // event id (other-client bug, relay-accepted). Treat the marker as
        // absent so the caller falls back to root == parent rather than
        // failing to send the reply.
        let tags = json!([["e", "not-a-valid-id", "", "root"], ["p", PUBKEY],]);
        assert!(find_root_from_tags(&tags).is_none());
    }

    #[test]
    fn malformed_root_does_not_shadow_valid_reply() {
        // If "root" is malformed but "reply" is valid, fall back to "reply".
        let tags = json!([["e", "garbage", "", "root"], ["e", ID_B, "", "reply"],]);
        assert_eq!(find_root_from_tags(&tags).as_deref(), Some(ID_B));
    }

    #[test]
    fn non_array_input_returns_none() {
        assert!(find_root_from_tags(&json!({})).is_none());
        assert!(find_root_from_tags(&json!(null)).is_none());
    }

    // ── @mention resolution pipeline ────────────────────────────────────
    //
    // These tests don't hit the network — they prove that *given* the
    // events the relay returns, the CLI's parse + match wiring produces
    // the right pubkeys. The async I/O wrapper around them is one
    // straight line; the pure stages it composes are exercised here and
    // in buzz-sdk.

    /// End-to-end (sans I/O): body text → extracted names → matched
    /// member pubkeys, using realistic 39002 + kind:0 event JSON.
    /// This is the regression guard for the previous stub that always
    /// returned `vec![]`.
    #[test]
    fn cli_pipeline_resolves_body_at_names_to_member_pubkeys() {
        // kind 39002 channel-members event with three members.
        let members_event = json!({
            "kind": 39002,
            "tags": [
                ["d", "00000000-0000-0000-0000-000000000000"],
                ["p", PK_VALID_A, "", "member"],
                ["p", PK_VALID_B, "", "member"],
                ["p", PK_VALID_C, "", "member"],
            ],
            "content": "",
        });
        assert_eq!(
            parse_member_pubkeys(&members_event),
            vec![PK_VALID_A, PK_VALID_B, PK_VALID_C]
        );

        // Three kind:0 profile events.
        let entries = vec![
            MentionProfile {
                pubkey: PK_VALID_A,
                content_json: r#"{"display_name":"Alice"}"#,
            },
            MentionProfile {
                pubkey: PK_VALID_B,
                content_json: r#"{"display_name":"Bob"}"#,
            },
            MentionProfile {
                pubkey: PK_VALID_C,
                content_json: r#"{"name":"Carol"}"#,
            },
        ];

        // Body mentions Alice and Carol (display_name fallback to `name`).
        let names = extract_at_names("hello @alice and @CAROL");
        let resolved = match_names_to_profiles(&names, &entries);
        assert_eq!(resolved, vec![PK_VALID_A, PK_VALID_C]);
    }

    #[test]
    fn cli_pipeline_resolves_multiword_display_names() {
        let profile_events: Vec<serde_json::Value> = vec![
            json!({
                "pubkey": PK_VALID_A,
                "content": r#"{"display_name":"Will Pfleger"}"#,
            }),
            json!({
                "pubkey": PK_VALID_B,
                "content": r#"{"display_name":"Alice"}"#,
            }),
        ];

        // Simulate the single-parse pipeline from resolve_content_mentions.
        let mut name_to_pubkeys: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        let mut display_names: Vec<String> = Vec::new();
        for e in &profile_events {
            let pubkey = e.get("pubkey").unwrap().as_str().unwrap();
            let content_json = e.get("content").unwrap().as_str().unwrap();
            let v: serde_json::Value = serde_json::from_str(content_json).unwrap();
            let name = v
                .get("display_name")
                .or_else(|| v.get("name"))
                .and_then(|n| n.as_str())
                .filter(|n| !n.is_empty())
                .unwrap();
            let lower = name.to_ascii_lowercase();
            name_to_pubkeys
                .entry(lower)
                .or_default()
                .push(pubkey.to_string());
            display_names.push(name.to_string());
        }

        let known_refs: Vec<&str> = display_names.iter().map(|s| s.as_str()).collect();
        let names = extract_at_mentions_with_known("hey @Will Pfleger and @alice!", &known_refs);
        assert_eq!(names, vec!["will pfleger", "alice"]);

        let resolved: Vec<String> = names
            .iter()
            .flat_map(|n| name_to_pubkeys.get(n).into_iter().flatten())
            .cloned()
            .collect();
        assert_eq!(resolved, vec![PK_VALID_A, PK_VALID_B]);
    }

    #[test]
    fn cli_pipeline_returns_empty_when_no_at_names() {
        // Sanity: no `@names` in body → no profile match attempt needed.
        let names = extract_at_names("plain message, no mentions");
        assert!(names.is_empty());
    }

    #[test]
    fn parse_member_pubkeys_ignores_non_p_tags() {
        let event = json!({
            "tags": [
                ["d", "channel-id"],
                ["p", PK_VALID_A],
                ["h", "channel-id"],
                ["e", "some-event"],
                ["p", PK_VALID_B, "wss://relay", "member"],
            ],
        });
        assert_eq!(parse_member_pubkeys(&event), vec![PK_VALID_A, PK_VALID_B]);
    }

    #[test]
    fn parse_member_pubkeys_handles_malformed_event() {
        assert!(parse_member_pubkeys(&json!({})).is_empty());
        assert!(parse_member_pubkeys(&json!({"tags": "not an array"})).is_empty());
        assert!(parse_member_pubkeys(&json!({"tags": [["p"]]})).is_empty());
    }

    #[test]
    fn parse_member_pubkeys_filters_invalid_hex() {
        // `PublicKey::from_hex` rejects non-hex and wrong-length inputs and
        // canonicalizes hex case. (Note: it accepts any 64-char x-only hex
        // whose integer value is in field; it does not verify the point is
        // actually on the curve — same as MCP's behavior.)
        let pk_uppercase: String = PK_VALID_A.to_ascii_uppercase();
        let event = json!({
            "tags": [
                ["p", PK_VALID_A],       // valid, lowercase
                ["p", pk_uppercase],     // valid hex, canonicalized to lowercase
                ["p", "too-short"],      // length fail
                ["p", "z".repeat(64)],   // non-hex chars
                ["p", "a".repeat(63)],   // off-by-one length
            ],
        });
        assert_eq!(parse_member_pubkeys(&event), vec![PK_VALID_A, PK_VALID_A]);
    }
}
