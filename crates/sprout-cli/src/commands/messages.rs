use nostr::EventId;
use sprout_sdk::{DiffMeta, ThreadRef, VoteDirection};
use uuid::Uuid;

use crate::client::SproutClient;
use crate::error::CliError;
use crate::validate::{
    extract_at_names, infer_language, merge_mentions, normalize_mention_pubkeys, read_or_stdin,
    truncate_diff, validate_content_size, validate_hex64, validate_uuid, MAX_DIFF_BYTES,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a 64-char hex string into a nostr::EventId.
fn parse_event_id(hex: &str) -> Result<EventId, CliError> {
    EventId::parse(hex).map_err(|e| CliError::Usage(format!("invalid event ID: {e}")))
}

/// Parse a UUID string into uuid::Uuid.
fn parse_uuid(s: &str) -> Result<Uuid, CliError> {
    Uuid::parse_str(s).map_err(|e| CliError::Usage(format!("invalid channel UUID: {e}")))
}

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
/// This matches the behavior of `sprout-mcp`'s `resolve_thread_ref` so that
/// CLI-sent replies thread identically to MCP-sent replies.
async fn resolve_thread_ref(
    client: &SproutClient,
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
async fn resolve_channel_id(client: &SproutClient, event_id: &str) -> Result<Uuid, CliError> {
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

/// Resolve @names in content against channel members (queried from channel metadata).
/// Returns matching pubkeys. On any error, returns empty vec — never blocks a send.
async fn resolve_content_mentions(
    client: &SproutClient,
    channel_id: &str,
    content: &str,
) -> Vec<String> {
    let names = extract_at_names(content);
    if names.is_empty() {
        return vec![];
    }
    // Query channel metadata to get member list from p-tags
    let filter = serde_json::json!({
        "kinds": [39002],
        "#h": [channel_id]
    });
    let raw = client.query(&filter).await.unwrap_or_default();
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
    // Channel metadata is returned as an array of events
    let Some(events) = parsed.as_array() else {
        return vec![];
    };
    let Some(event) = events.first() else {
        return vec![];
    };
    let Some(tags) = event.get("tags").and_then(|t| t.as_array()) else {
        return vec![];
    };
    // p-tags contain member pubkeys; we can't resolve display names without profiles
    // For now, return empty — @mention resolution requires profile lookup
    let _ = (tags, names);
    vec![]
}

// ---------------------------------------------------------------------------
// Read commands — POST /query
// ---------------------------------------------------------------------------

pub async fn cmd_get_messages(
    client: &SproutClient,
    channel_id: &str,
    limit: Option<u32>,
    before: Option<i64>,
    since: Option<i64>,
    kinds: Option<&str>,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let limit = limit.unwrap_or(50).min(200);

    let mut filter = serde_json::json!({
        "kinds": [9, 40002],
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
    println!("{resp}");
    Ok(())
}

pub async fn cmd_get_thread(
    client: &SproutClient,
    channel_id: &str,
    event_id: &str,
    _depth_limit: Option<u32>,
    limit: Option<u32>,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    validate_hex64(event_id)?;
    let limit = limit.unwrap_or(100).min(500);

    // Get the root event and all replies referencing it via e-tag
    let filter = serde_json::json!({
        "kinds": [9, 40002],
        "#h": [channel_id],
        "#e": [event_id],
        "limit": limit
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_search(
    client: &SproutClient,
    query: &str,
    limit: Option<u32>,
) -> Result<(), CliError> {
    let limit = limit.unwrap_or(20).min(100);
    let filter = serde_json::json!({
        "search": query,
        "limit": limit
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Write commands — signed events via POST /events
// ---------------------------------------------------------------------------

pub struct SendMessageParams {
    pub channel_id: String,
    pub content: String,
    #[allow(dead_code)] // reserved for future kind routing
    pub kind: Option<u16>,
    pub reply_to: Option<String>,
    pub broadcast: bool,
    pub mentions: Vec<String>,
    pub files: Vec<String>,
}

pub async fn cmd_send_message(client: &SproutClient, p: SendMessageParams) -> Result<(), CliError> {
    validate_uuid(&p.channel_id)?;
    validate_content_size(&p.content)?;
    if let Some(ref r) = p.reply_to {
        validate_hex64(r)?;
    }
    for m in &p.mentions {
        validate_hex64(m)?;
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

    // Normalize explicit mentions, then merge auto-resolved up to SDK cap of 50.
    let mut merged: Vec<String> = normalize_mention_pubkeys(&p.mentions, "");
    let auto_resolved = resolve_content_mentions(client, &p.channel_id, &final_content).await;
    merge_mentions(&mut merged, &auto_resolved, 50);
    let mention_refs: Vec<&str> = merged.iter().map(|s| s.as_str()).collect();

    let builder = sprout_sdk::build_message(
        channel_uuid,
        &final_content,
        thread_ref.as_ref(),
        &mention_refs,
        p.broadcast,
        &media_tags,
    )
    .map_err(|e| CliError::Other(format!("build_message failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
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

pub async fn cmd_send_diff_message(
    client: &SproutClient,
    p: SendDiffParams,
) -> Result<(), CliError> {
    validate_uuid(&p.channel_id)?;
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
        sprout_sdk::build_diff_message(channel_uuid, &diff, &diff_meta, thread_ref.as_ref())
            .map_err(|e| CliError::Other(format!("build_diff_message failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_delete_message(client: &SproutClient, event_id: &str) -> Result<(), CliError> {
    validate_hex64(event_id)?;

    // Resolve channel_id from the event's h-tag
    let channel_uuid = resolve_channel_id(client, event_id).await?;
    let target_eid = parse_event_id(event_id)?;

    let builder = sprout_sdk::build_delete_message(channel_uuid, target_eid)
        .map_err(|e| CliError::Other(format!("build_delete_message failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

/// Edit a message you previously sent.
pub async fn cmd_edit_message(
    client: &SproutClient,
    event_id: &str,
    content: &str,
) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    validate_content_size(content)?;

    // Resolve channel_id from the event's h-tag
    let channel_uuid = resolve_channel_id(client, event_id).await?;
    let target_eid = parse_event_id(event_id)?;

    let builder = sprout_sdk::build_edit(channel_uuid, target_eid, content)
        .map_err(|e| CliError::Other(format!("build_edit failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

/// Vote on a forum post or comment.
pub async fn cmd_vote_on_post(
    client: &SproutClient,
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

    let builder = sprout_sdk::build_vote(channel_uuid, target_eid, vote_dir)
        .map_err(|e| CliError::Other(format!("build_vote failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::find_root_from_tags;
    use serde_json::json;

    const ID_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ID_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const PUBKEY: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

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
}
