use buzz_sdk::{DeleteMessageOptions, DiffMeta, ThreadRef, VoteDirection};
use nostr::PublicKey;
use uuid::Uuid;

use crate::client::{normalize_events, normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{
    infer_language, parse_event_id, parse_uuid, read_or_stdin, truncate_diff,
    validate_content_size, validate_hex64, validate_uuid, MAX_DIFF_BYTES,
};
use buzz_sdk::mentions::{
    extract_at_mentions_with_known, extract_nostr_uris, strip_code_regions, MENTION_CAP,
};

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

fn resolve_names_to_pubkeys(
    names: &[String],
    name_to_pubkeys: &std::collections::HashMap<String, Vec<String>>,
    has_explicit_mentions: bool,
) -> Result<Vec<String>, CliError> {
    let mut resolved = Vec::new();
    for name in names {
        match name_to_pubkeys
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            [pubkey] => resolved.push(pubkey.clone()),
            [] if has_explicit_mentions => {}
            [] => {
                return Err(CliError::Usage(format!(
                    "mention '@{name}' does not match a current channel member; retry with --mention <pubkey>"
                )))
            }
            _ if has_explicit_mentions => {}
            candidates => {
                return Err(CliError::Usage(format!(
                    "mention '@{name}' is ambiguous; candidates: {}. Retry with --mention <pubkey>",
                    candidates.join(", ")
                )))
            }
        }
    }
    Ok(resolved)
}

/// Resolve mention text against the channel membership snapshot.
///
/// Returns both the current member set and uniquely name-resolved pubkeys.
/// Lookup failures are fatal when mention processing is requested: publishing
/// visible mention text without its intended `p` tag is worse than not sending.
async fn resolve_content_mentions(
    client: &BuzzClient,
    channel_id: &str,
    content: &str,
    has_explicit_mentions: bool,
) -> Result<(Vec<String>, Vec<String>), CliError> {
    let stripped = strip_code_regions(content);
    if !stripped.contains('@') && !has_explicit_mentions {
        return Ok((vec![], vec![]));
    }

    let members_filter = serde_json::json!({
        "kinds": [39002],
        "#d": [channel_id],
        "limit": 1,
    });
    let member_pubkeys = fetch_member_pubkeys(client, &members_filter)
        .await
        .ok_or_else(|| {
            CliError::Other("could not load channel membership for mention preflight".into())
        })?;

    if !stripped.contains('@') {
        return Ok((member_pubkeys, vec![]));
    }

    let profiles_filter = serde_json::json!({
        "kinds": [0],
        "authors": member_pubkeys,
        "limit": member_pubkeys.len(),
    });
    let profile_events = fetch_events(client, &profiles_filter)
        .await
        .ok_or_else(|| {
            CliError::Other("could not load member profiles for mention resolution".into())
        })?;

    let mut name_to_pubkeys: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut display_names = Vec::new();
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
        name_to_pubkeys
            .entry(name.to_ascii_lowercase())
            .or_default()
            .push(pubkey.to_string());
        display_names.push(name.to_string());
    }

    let known_refs: Vec<&str> = display_names.iter().map(String::as_str).collect();
    let names = extract_at_mentions_with_known(&stripped, &known_refs);
    let resolved = resolve_names_to_pubkeys(&names, &name_to_pubkeys, has_explicit_mentions)?;
    Ok((member_pubkeys, resolved))
}

fn normalize_explicit_mentions(values: &[String]) -> Result<Vec<String>, CliError> {
    let mut normalized = Vec::new();
    for value in values {
        let pubkey = PublicKey::parse(value.trim())
            .map_err(|_| CliError::Usage(format!("invalid --mention pubkey: {value}")))?;
        let hex = pubkey.to_hex();
        if !normalized.contains(&hex) {
            normalized.push(hex);
        }
    }
    if normalized.len() > MENTION_CAP {
        return Err(CliError::Usage(format!(
            "too many --mention values (max {MENTION_CAP})"
        )));
    }
    Ok(normalized)
}

fn merge_message_mentions(
    explicit: &[String],
    uri_pubkeys: &[String],
    auto_resolved: &[String],
) -> Result<Vec<String>, CliError> {
    let mut mentions = Vec::new();
    for pubkey in explicit
        .iter()
        .chain(uri_pubkeys.iter())
        .chain(auto_resolved.iter())
    {
        if !mentions.contains(pubkey) {
            mentions.push(pubkey.clone());
        }
    }
    if mentions.len() > MENTION_CAP {
        return Err(CliError::Usage(format!(
            "too many unique message mentions (max {MENTION_CAP})"
        )));
    }
    Ok(mentions)
}

fn missing_members(mentions: &[String], members: &[String]) -> Vec<String> {
    let members: std::collections::HashSet<&str> = members.iter().map(String::as_str).collect();
    mentions
        .iter()
        .filter(|pk| !members.contains(pk.as_str()))
        .cloned()
        .collect()
}

fn event_mention_pubkeys(event: &nostr::Event) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some("p"))
                .then(|| parts.get(1).cloned())
                .flatten()
        })
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

/// Composite cursor grammar: the event-id half is meaningless without the
/// timestamp half. The relay rejects it outright (`before_id requires until to
/// be set`, `bridge.rs`), and a half cursor silently demoted to a head request
/// is the dup/loss bug the composite form exists to kill — so refuse locally
/// rather than spend a round trip on a 400.
fn validate_cursor_pair(
    ts: Option<i64>,
    id: Option<&str>,
    id_flag: &str,
    ts_flag: &str,
) -> Result<(), CliError> {
    if let Some(id) = id {
        if ts.is_none() {
            return Err(CliError::Usage(format!("{id_flag} requires {ts_flag}")));
        }
        validate_hex64(id)?;
    }
    Ok(())
}

/// Kinds returned by `messages get` when `--kinds` is omitted.
///
/// Quoted by file:line in field notes, because the CLI has no way to report
/// the kind universe of a channel — a pull can only ever state which kinds it
/// asked for.
const DEFAULT_MESSAGE_KINDS: [u64; 5] = [9, 40002, 40008, 45001, 45003];

/// Parse a `--kinds` list, refusing anything that is not an event kind.
///
/// The previous form was `filter_map(|s| s.trim().parse().ok())`, which
/// discarded unparseable tokens silently: `--kinds '*'` and `--kinds all`
/// produced an empty list, left the default kinds in place, and exited 0 — so
/// a caller measuring a widened pull was handed the narrow default while being
/// told it succeeded. A typo is now a usage error naming the token.
fn parse_kinds(kinds: &str) -> Result<Vec<u64>, CliError> {
    kinds
        .split(',')
        .map(|token| {
            let token = token.trim();
            token.parse::<u64>().map_err(|_| {
                CliError::Usage(format!(
                    "--kinds: `{token}` is not an event kind (expected comma-separated integers, e.g. 9,1984)"
                ))
            })
        })
        .collect()
}

/// Build the filter for a channel message query.
///
/// Split out from [`cmd_get_messages`] so the cursor grammar is testable
/// without a live relay.
fn build_messages_filter(
    channel_id: &str,
    limit: u32,
    before: Option<i64>,
    before_id: Option<&str>,
    since: Option<i64>,
    kinds: Option<&str>,
) -> Result<serde_json::Value, CliError> {
    let mut filter = serde_json::json!({
        "kinds": DEFAULT_MESSAGE_KINDS,
        "#h": [channel_id],
        "limit": limit
    });

    // If specific kinds requested, override. Parsing happens here rather than
    // at the caller so no code path can reach the wire with a partially
    // discarded kind list.
    if let Some(k) = kinds {
        filter["kinds"] = serde_json::json!(parse_kinds(k)?);
    }

    if let Some(b) = before {
        filter["until"] = serde_json::json!(b);
        // Both or neither: the id half only rides along with the timestamp.
        if let Some(bid) = before_id {
            filter["before_id"] = serde_json::json!(bid);
        }
    }
    if let Some(s) = since {
        filter["since"] = serde_json::json!(s);
    }

    Ok(filter)
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_get_messages(
    client: &BuzzClient,
    channel_id: &str,
    limit: Option<u32>,
    before: Option<i64>,
    before_id: Option<&str>,
    since: Option<i64>,
    kinds: Option<&str>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    validate_cursor_pair(before, before_id, "--before-id", "--before")?;
    let limit = limit.unwrap_or(50).min(200);

    let filter = build_messages_filter(channel_id, limit, before, before_id, since, kinds)?;

    let resp = client.query(&filter).await?;
    let mut events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    events.sort_by_key(|e| e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0));
    let normalized = normalize_events(&events);
    println!("{}", format_events(&normalized, format));
    Ok(())
}

/// Depth bound sent when `--thread-cursor` is used without `--depth-limit`.
///
/// The relay only reads the thread cursor on the depth-limited code path
/// (`bridge.rs`: filters without `depth_limit` fall through to the generic
/// catch-all query, which has no cursor), so reaching the cursor at all
/// requires sending *some* depth bound. This value expresses "no effective
/// bound": it is `i32::MAX`, and thread nesting cannot approach it.
///
/// It is deliberately `i32::MAX` rather than `u32::MAX` — the relay binds the
/// depth as `i32`, so any value above `i32::MAX` wraps negative and matches
/// zero rows.
const THREAD_CURSOR_DEPTH_SENTINEL: u32 = i32::MAX as u32;

/// Build the reply filter for a thread query.
///
/// Split out from [`cmd_get_thread`] so the cursor/depth interaction is
/// testable without a live relay.
fn build_thread_reply_filter(
    channel_id: &str,
    event_id: &str,
    limit: u32,
    depth_limit: Option<u32>,
    thread_cursor: Option<i64>,
    thread_cursor_id: Option<&str>,
) -> serde_json::Value {
    let mut reply_filter = serde_json::json!({
        "kinds": [9, 40002, 40003, 40008, 45003],
        "#h": [channel_id],
        "#e": [event_id],
        "limit": limit
    });
    // A cursor is only honoured on the depth-limited path, so an explicit
    // cursor implies a depth bound even when the caller gave none.
    match (depth_limit, thread_cursor) {
        (Some(d), _) => reply_filter["depth_limit"] = serde_json::json!(d),
        (None, Some(_)) => {
            reply_filter["depth_limit"] = serde_json::json!(THREAD_CURSOR_DEPTH_SENTINEL)
        }
        (None, None) => {}
    }
    if let Some(c) = thread_cursor {
        reply_filter["thread_cursor"] = serde_json::json!(c);
        if let Some(id) = thread_cursor_id {
            reply_filter["thread_cursor_id"] = serde_json::json!(id);
        }
    }
    reply_filter
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_get_thread(
    client: &BuzzClient,
    channel_id: &str,
    event_id: &str,
    limit: Option<u32>,
    depth_limit: Option<u32>,
    thread_cursor: Option<i64>,
    thread_cursor_id: Option<&str>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    validate_hex64(event_id)?;
    validate_cursor_pair(thread_cursor, thread_cursor_id, "--after-id", "--after")?;
    let limit = limit.unwrap_or(100).min(500);

    // Two filters ORed in a single HTTP call:
    // 1. Replies referencing this event via e-tag (no kind restriction)
    // 2. The root event itself by ID
    let reply_filter = build_thread_reply_filter(
        channel_id,
        event_id,
        limit,
        depth_limit,
        thread_cursor,
        thread_cursor_id,
    );
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
    query: Option<&str>,
    author: Option<&str>,
    since: Option<i64>,
    limit: Option<u32>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    if query.is_none() && author.is_none() {
        return Err(CliError::Usage(
            "at least one of --query or --author is required".into(),
        ));
    }
    let limit = limit.unwrap_or(20).min(100);

    let author_hex = match author {
        Some(a) => Some(resolve_author(client, a).await?),
        None => None,
    };

    let mut filter = serde_json::json!({
        "kinds": [9, 40002, 45001, 45003],
        "limit": limit
    });
    if let Some(q) = query {
        filter["search"] = serde_json::json!(q);
    }
    if let Some(ref pk) = author_hex {
        filter["authors"] = serde_json::json!([pk]);
    }
    if let Some(s) = since {
        filter["since"] = serde_json::json!(s);
    }
    let resp = client.query(&filter).await?;
    let mut events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    // The full-text path returns relevance order; a pure author/time query has
    // no relevance, so present newest-first like `messages get`.
    if query.is_none() {
        events.sort_by_key(|e| {
            std::cmp::Reverse(e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0))
        });
    }
    let normalized = normalize_events(&events);
    println!("{}", format_events(&normalized, format));
    Ok(())
}

/// Resolve an `--author` value to a 64-char hex pubkey.
///
/// Accepts, in order of precedence: 64-char hex (validated), an `npub1…`
/// bech32 key, or a display name resolved via NIP-50 profile search. A name
/// must match exactly one user (case-insensitive, on `display_name` or
/// `name`) — ambiguity is an error listing the candidates rather than a
/// silent mix of authors.
async fn resolve_author(client: &BuzzClient, author: &str) -> Result<String, CliError> {
    let author = author.trim();
    if author.len() == 64 && author.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(author.to_ascii_lowercase());
    }
    if author.starts_with("npub1") {
        return nostr::PublicKey::parse(author)
            .map(|pk| pk.to_hex())
            .map_err(|_| CliError::Usage(format!("invalid npub: {author}")));
    }

    // Display name → NIP-50 search on kind:0, exact case-insensitive match.
    let filter = serde_json::json!({
        "kinds": [0],
        "search": author,
        "limit": 100
    });
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap_or_default();
    let mut matches = match_profiles_by_name(&events, author);
    match matches.len() {
        0 => Err(CliError::Usage(format!(
            "no user found with name '{author}' — pass a hex pubkey or npub instead"
        ))),
        1 => Ok(matches.remove(0).0),
        _ => {
            // Cap the candidate listing — some names are shared by dozens of
            // users, and an unbounded list turns the error into a wall of text.
            let shown = 5.min(matches.len());
            let mut listing: Vec<String> = matches[..shown]
                .iter()
                .map(|(pk, name)| format!("{name} ({pk})"))
                .collect();
            if matches.len() > shown {
                listing.push(format!("… and {} more", matches.len() - shown));
            }
            Err(CliError::Usage(format!(
                "name '{author}' is ambiguous — matches: {}. Pass a pubkey instead",
                listing.join(", ")
            )))
        }
    }
}

/// Exact case-insensitive profile match on `display_name` or `name` across
/// kind:0 events. Returns deduped `(pubkey, shown name)` pairs. Pure so the
/// name-resolution semantics are unit-testable without a relay.
fn match_profiles_by_name(events: &[serde_json::Value], name: &str) -> Vec<(String, String)> {
    let lower = name.to_ascii_lowercase();
    let mut matches: Vec<(String, String)> = Vec::new();
    for e in events {
        let Some(pubkey) = e.get("pubkey").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(content) = e
            .get("content")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        else {
            continue;
        };
        let display_name = content
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let plain_name = content.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if display_name.to_ascii_lowercase() == lower || plain_name.to_ascii_lowercase() == lower {
            let shown = if display_name.is_empty() {
                plain_name
            } else {
                display_name
            };
            matches.push((pubkey.to_string(), shown.to_string()));
        }
    }
    matches.sort();
    matches.dedup();
    matches
}

pub struct SendMessageParams {
    pub channel_id: String,
    pub content: String,
    pub kind: Option<u16>,
    pub reply_to: Option<String>,
    pub broadcast: bool,
    pub files: Vec<String>,
    pub mentions: Vec<String>,
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

    let explicit_mentions = normalize_explicit_mentions(&p.mentions)?;
    let stripped = strip_code_regions(&p.content);
    let uri_pubkeys = extract_nostr_uris(&stripped);
    // Supplying any identity explicitly authorizes unresolved or ambiguous @Name text
    // as presentation-only, matching Desktop's separate visible-label and p-tag model.
    // Uniquely resolvable member names still add their own p-tags; callers must supply
    // every intended identity whose visible label cannot be resolved uniquely.
    let has_explicit_mentions = !explicit_mentions.is_empty() || !uri_pubkeys.is_empty();
    let (member_pubkeys, auto_resolved) =
        resolve_content_mentions(client, &p.channel_id, &p.content, has_explicit_mentions).await?;
    let mention_pubkeys = merge_message_mentions(&explicit_mentions, &uri_pubkeys, &auto_resolved)?;

    let missing = missing_members(&mention_pubkeys, &member_pubkeys);
    if !missing.is_empty() {
        return Err(CliError::Usage(
            serde_json::json!({
                "message": "mentioned pubkeys are not channel members; add them explicitly before retrying",
                "missing_member_pubkeys": missing,
                "add_member_command": format!("buzz channels add-member --channel {} --pubkey <pubkey> --role <member|bot>", p.channel_id),
            })
            .to_string(),
        ));
    }

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

    let mention_refs: Vec<&str> = mention_pubkeys.iter().map(String::as_str).collect();

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
    let emitted_mentions = event_mention_pubkeys(&event);
    let resp = client.submit_event(event).await?;
    let mut output: serde_json::Value = serde_json::from_str(&normalize_write_response(&resp))
        .unwrap_or_else(|_| serde_json::json!({ "response": resp }));
    if let Some(object) = output.as_object_mut() {
        object.insert(
            "mention_pubkeys".into(),
            serde_json::json!(emitted_mentions),
        );
    }
    println!("{output}");
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

pub async fn cmd_delete_message(
    client: &BuzzClient,
    event_id: &str,
    action_id: Option<Uuid>,
    reason_code: Option<&str>,
    public_reason: Option<&str>,
) -> Result<(), CliError> {
    validate_hex64(event_id)?;

    // Resolve channel_id from the event's h-tag
    let channel_uuid = resolve_channel_id(client, event_id).await?;
    let target_eid = parse_event_id(event_id)?;

    let builder = buzz_sdk::build_delete_message_with_options(
        channel_uuid,
        target_eid,
        DeleteMessageOptions {
            action_id,
            reason_code,
            public_reason,
        },
    )
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
            mentions,
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
                    mentions,
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
        MessagesCmd::Delete {
            event,
            action_id,
            reason_code,
            public_reason,
        } => {
            cmd_delete_message(
                client,
                &event,
                action_id,
                reason_code.as_deref(),
                public_reason.as_deref(),
            )
            .await
        }
        MessagesCmd::Get {
            channel,
            limit,
            before,
            before_id,
            since,
            kinds,
        } => {
            cmd_get_messages(
                client,
                &channel,
                limit,
                before,
                before_id.as_deref(),
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
            after,
            after_id,
        } => {
            cmd_get_thread(
                client,
                &channel,
                &event,
                limit,
                depth_limit,
                after,
                after_id.as_deref(),
                format,
            )
            .await
        }
        MessagesCmd::Search {
            query,
            author,
            since,
            limit,
        } => {
            cmd_search(
                client,
                query.as_deref(),
                author.as_deref(),
                since,
                limit,
                format,
            )
            .await
        }
        MessagesCmd::Vote { event, direction } => {
            cmd_vote_on_post(client, &event, &direction).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        event_mention_pubkeys, find_root_from_tags, match_profiles_by_name, merge_message_mentions,
        missing_members, normalize_explicit_mentions, parse_member_pubkeys,
        resolve_names_to_pubkeys,
    };
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

    #[test]
    fn explicit_mentions_accept_hex_and_npub_and_deduplicate() {
        use nostr::ToBech32;
        let npub = nostr::PublicKey::from_hex(PK_VALID_A)
            .unwrap()
            .to_bech32()
            .unwrap();
        assert_eq!(
            normalize_explicit_mentions(&[PK_VALID_A.into(), npub]).unwrap(),
            vec![PK_VALID_A]
        );
        assert!(normalize_explicit_mentions(&["not-a-key".into()]).is_err());
    }

    #[test]
    fn explicit_mentions_authorize_presentation_text_without_name_resolution() {
        let names = vec!["renamed user".into()];
        let profiles = std::collections::HashMap::new();
        assert_eq!(
            resolve_names_to_pubkeys(&names, &profiles, true).unwrap(),
            Vec::<String>::new()
        );
        assert!(resolve_names_to_pubkeys(&names, &profiles, false).is_err());
    }

    #[test]
    fn explicit_mentions_authorize_ambiguous_presentation_text() {
        let names = vec!["alice".into()];
        let profiles = std::collections::HashMap::from([(
            "alice".into(),
            vec![PK_VALID_A.into(), PK_VALID_B.into()],
        )]);
        assert_eq!(
            resolve_names_to_pubkeys(&names, &profiles, true).unwrap(),
            Vec::<String>::new()
        );
        let error = resolve_names_to_pubkeys(&names, &profiles, false).unwrap_err();
        assert!(error.to_string().contains(PK_VALID_A));
        assert!(error.to_string().contains(PK_VALID_B));
    }

    #[test]
    fn explicit_mentions_make_all_at_names_presentation_only() {
        let names = vec!["alice".into(), "bob".into()];
        let profiles = std::collections::HashMap::from([("alice".into(), vec![PK_VALID_A.into()])]);
        assert_eq!(
            resolve_names_to_pubkeys(&names, &profiles, true).unwrap(),
            vec![PK_VALID_A]
        );
        assert!(resolve_names_to_pubkeys(&names, &profiles, false).is_err());
    }

    #[test]
    fn combined_mention_union_errors_instead_of_truncating() {
        let explicit: Vec<String> = (0..50).map(|i| format!("explicit-{i}")).collect();
        assert!(merge_message_mentions(&explicit, &[], &["resolved-bob".into()]).is_err());

        let mut with_duplicate = explicit.clone();
        with_duplicate.push(explicit[0].clone());
        assert_eq!(
            merge_message_mentions(&with_duplicate, &[explicit[1].clone()], &[])
                .unwrap()
                .len(),
            50
        );
    }

    #[test]
    fn membership_preflight_lists_only_missing_mentions() {
        assert_eq!(
            missing_members(
                &[PK_VALID_A.into(), PK_VALID_B.into()],
                &[PK_VALID_A.into()]
            ),
            vec![PK_VALID_B]
        );
    }

    #[test]
    fn mention_evidence_comes_from_signed_event_tags() {
        use nostr::{EventBuilder, Keys, Tag};
        let event = EventBuilder::text_note("hello")
            .tags(vec![Tag::parse(["p", PK_VALID_A]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert_eq!(event_mention_pubkeys(&event), vec![PK_VALID_A]);
    }

    // ---- match_profiles_by_name (author resolution for `messages search --author`) ----

    fn profile_event(
        pubkey: &str,
        display_name: Option<&str>,
        name: Option<&str>,
    ) -> serde_json::Value {
        let mut content = serde_json::Map::new();
        if let Some(d) = display_name {
            content.insert("display_name".into(), json!(d));
        }
        if let Some(n) = name {
            content.insert("name".into(), json!(n));
        }
        json!({
            "pubkey": pubkey,
            "content": serde_json::Value::Object(content).to_string(),
        })
    }

    #[test]
    fn author_name_match_is_exact_case_insensitive() {
        let events = vec![
            profile_event(PK_VALID_A, Some("Aaron"), Some("aaron")),
            // Substring only — NIP-50 may return it, but it must not match.
            profile_event(PK_VALID_B, Some("Aaronson"), None),
        ];
        let matches = match_profiles_by_name(&events, "aArOn");
        assert_eq!(matches, vec![(PK_VALID_A.to_string(), "Aaron".to_string())]);
    }

    #[test]
    fn author_name_ambiguity_returns_all_candidates() {
        let events = vec![
            profile_event(PK_VALID_A, Some("Sam"), None),
            profile_event(PK_VALID_B, None, Some("sam")),
        ];
        let matches = match_profiles_by_name(&events, "sam");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn author_name_no_match_and_malformed_content() {
        let events = vec![
            profile_event(PK_VALID_A, Some("Aaron"), None),
            json!({"pubkey": PK_VALID_B, "content": "not-json"}),
            json!({"content": "{}"}), // missing pubkey
        ];
        assert!(match_profiles_by_name(&events, "Zoe").is_empty());
    }

    #[test]
    fn author_name_dedups_replaceable_event_copies() {
        // Same (pubkey, name) appearing twice (e.g. duplicate kind:0 rows)
        // must resolve unambiguously.
        let events = vec![
            profile_event(PK_VALID_A, Some("Aaron"), None),
            profile_event(PK_VALID_A, Some("Aaron"), None),
        ];
        assert_eq!(match_profiles_by_name(&events, "Aaron").len(), 1);
    }
}

#[cfg(test)]
mod thread_cursor_tests {
    use super::{build_thread_reply_filter, THREAD_CURSOR_DEPTH_SENTINEL};

    const CH: &str = "3928fe05-df61-4b5d-b9c7-d623b9b10ea1";
    const ROOT: &str = "f6f7a5212b1a6451f1906406e224c01834dc950826c337046b74b18ecc5785ce";
    const CURSOR_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn no_cursor_and_no_depth_sends_neither_field() {
        // The default pull must keep its existing shape: absent `depth_limit`
        // is what routes the filter to the catch-all (newest-anchored) path,
        // so adding the cursor flags must not perturb it.
        let f = build_thread_reply_filter(CH, ROOT, 100, None, None, None);
        assert!(f.get("depth_limit").is_none());
        assert!(f.get("thread_cursor").is_none());
        assert!(f.get("thread_cursor_id").is_none());
    }

    #[test]
    fn cursor_without_depth_limit_supplies_the_sentinel() {
        // The relay only reads the cursor on the depth-limited path, so a
        // cursor with no explicit depth must still carry a depth bound or the
        // cursor is silently ignored and the caller re-reads page one forever.
        let f = build_thread_reply_filter(CH, ROOT, 500, None, Some(1_786_800_000), None);
        assert_eq!(
            f["depth_limit"],
            serde_json::json!(THREAD_CURSOR_DEPTH_SENTINEL)
        );
        assert_eq!(f["thread_cursor"], serde_json::json!(1_786_800_000_i64));
    }

    #[test]
    fn sentinel_is_representable_as_i32() {
        // buzz-db binds the depth as i32 (`dl as i32`); any value above
        // i32::MAX wraps negative and `depth <= -N` matches zero replies.
        // Measured live at bff3110a0: --depth-limit 2147483648 returns n=1
        // (root only) while 2147483647 returns the full thread.
        assert!(i32::try_from(THREAD_CURSOR_DEPTH_SENTINEL).is_ok());
        assert_eq!(THREAD_CURSOR_DEPTH_SENTINEL, i32::MAX as u32);
    }

    #[test]
    fn explicit_depth_limit_wins_over_the_sentinel() {
        // A caller who asks for depth 2 while paging must get depth 2, not the
        // sentinel — otherwise the cursor plumbing would silently widen an
        // explicit depth bound.
        let f = build_thread_reply_filter(CH, ROOT, 500, Some(2), Some(1_786_800_000), None);
        assert_eq!(f["depth_limit"], serde_json::json!(2));
    }

    #[test]
    fn composite_cursor_carries_the_tiebreak_id() {
        let f =
            build_thread_reply_filter(CH, ROOT, 500, None, Some(1_786_800_000), Some(CURSOR_ID));
        assert_eq!(f["thread_cursor"], serde_json::json!(1_786_800_000_i64));
        assert_eq!(f["thread_cursor_id"], serde_json::json!(CURSOR_ID));
    }

    #[test]
    fn cursor_id_is_dropped_without_a_cursor_timestamp() {
        // Defense in depth: cmd_get_thread rejects this combination up front,
        // but the builder must not emit a lone `thread_cursor_id` either —
        // the relay's cursor grammar requires both or neither.
        let f = build_thread_reply_filter(CH, ROOT, 500, None, None, Some(CURSOR_ID));
        assert!(f.get("thread_cursor_id").is_none());
        assert!(f.get("thread_cursor").is_none());
    }

    #[test]
    fn limit_and_targeting_fields_are_unchanged_by_paging() {
        let f = build_thread_reply_filter(CH, ROOT, 500, None, Some(1), None);
        assert_eq!(f["limit"], serde_json::json!(500));
        assert_eq!(f["#h"], serde_json::json!([CH]));
        assert_eq!(f["#e"], serde_json::json!([ROOT]));
    }

    #[test]
    fn a_zero_cursor_is_a_real_cursor_and_seeds_the_forward_walk() {
        // `--after 0` is how a caller opts into the oldest-anchored walk
        // without also constraining depth. `Some(0)` must therefore be treated
        // as present, not folded into `None` by a falsy check — otherwise the
        // filter routes to the newest-anchored catch-all and the walk
        // terminates after one page.
        let f = build_thread_reply_filter(CH, ROOT, 500, None, Some(0), None);
        assert_eq!(f["thread_cursor"], serde_json::json!(0_i64));
        assert_eq!(
            f["depth_limit"],
            serde_json::json!(THREAD_CURSOR_DEPTH_SENTINEL)
        );
    }

    #[test]
    fn the_tiebreak_id_is_sent_whenever_it_is_supplied() {
        // Polarity asymmetry, measured at bff3110a0: the forward (thread)
        // legacy cursor is STRICT `>` (buzz-db/src/thread.rs:443) while the
        // backward (messages get) one is INCLUSIVE `<=` (event.rs:505). So on
        // this path a timestamp-only cursor whose page boundary lands inside a
        // shared second skips the remainder of that second silently — there is
        // no inclusive re-return to recover it, and no timestamp-only step rule
        // is safe at every boundary alignment. Live: thread 7f2cea28, tie
        // second 1785163671 (multiplicity 2 by pinned probe), ts-only drops at
        // caps {1,2,11,22} while the composite cursor recovers 2/2 at all of
        // 1..24. The tiebreak must therefore ride along on every paged call.
        let f =
            build_thread_reply_filter(CH, ROOT, 500, None, Some(1_785_163_671), Some(CURSOR_ID));
        assert_eq!(f["thread_cursor_id"], serde_json::json!(CURSOR_ID));
    }
}

#[cfg(test)]
mod messages_cursor_tests {
    use super::{build_messages_filter, validate_cursor_pair};

    const CH: &str = "3928fe05-df61-4b5d-b9c7-d623b9b10ea1";
    const BEFORE_ID: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn head_request_sends_no_cursor_fields() {
        // The default pull must keep its existing wire shape.
        let f = build_messages_filter(CH, 50, None, None, None, None).expect("no kinds to parse");
        assert!(f.get("until").is_none());
        assert!(f.get("before_id").is_none());
        assert_eq!(f["limit"], serde_json::json!(50));
        assert_eq!(f["#h"], serde_json::json!([CH]));
    }

    #[test]
    fn timestamp_only_cursor_still_works() {
        // Back-compat: `--before` alone is the existing (inclusive) cursor and
        // must keep sending a bare `until`, never a half composite.
        let f = build_messages_filter(CH, 200, Some(1_786_800_000), None, None, None)
            .expect("no kinds to parse");
        assert_eq!(f["until"], serde_json::json!(1_786_800_000_i64));
        assert!(f.get("before_id").is_none());
    }

    #[test]
    fn composite_cursor_sends_both_halves() {
        // Wire-verified discriminator (buzz-security corpus, `bff3110a0`):
        // window B=1785282528, cap=7, tie second T=1785163671 with
        // multiplicity 2. A decrement-always backward walk recovers 1/2 there
        // (drops `fa81da0b`); the same walk with `--before-id` recovers 2/2.
        // So the composite cursor removes a correctness dependency on the
        // caller's loop shape, not merely a truncation at a large tie.
        let f = build_messages_filter(CH, 200, Some(1_786_800_000), Some(BEFORE_ID), None, None)
            .expect("no kinds to parse");
        assert_eq!(f["until"], serde_json::json!(1_786_800_000_i64));
        assert_eq!(f["before_id"], serde_json::json!(BEFORE_ID));
    }

    #[test]
    fn cursor_id_is_dropped_without_a_timestamp() {
        // The relay 400s on `before_id` without `until`; the builder must not
        // emit a half cursor even if the caller-level guard is bypassed.
        let f = build_messages_filter(CH, 200, None, Some(BEFORE_ID), None, None)
            .expect("no kinds to parse");
        assert!(f.get("before_id").is_none());
        assert!(f.get("until").is_none());
    }

    #[test]
    fn since_and_kinds_survive_a_composite_cursor() {
        let f = build_messages_filter(
            CH,
            200,
            Some(1_786_800_000),
            Some(BEFORE_ID),
            Some(1_786_000_000),
            Some("9,1984"),
        )
        .expect("a well-formed kind list parses");
        assert_eq!(f["since"], serde_json::json!(1_786_000_000_i64));
        assert_eq!(f["kinds"], serde_json::json!([9, 1984]));
        assert_eq!(f["before_id"], serde_json::json!(BEFORE_ID));
    }

    #[test]
    fn lone_cursor_id_is_a_usage_error() {
        let err = validate_cursor_pair(None, Some(BEFORE_ID), "--before-id", "--before")
            .expect_err("lone --before-id must be refused");
        assert!(
            err.to_string().contains("--before-id requires --before"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn malformed_cursor_id_is_rejected_locally() {
        // The relay rejects a non-64-hex `before_id` with a 400; refusing it
        // here means the caller never spends a round trip to learn that.
        assert!(validate_cursor_pair(
            Some(1_786_800_000),
            Some("not-a-hex-id"),
            "--before-id",
            "--before"
        )
        .is_err());
    }

    #[test]
    fn a_full_composite_cursor_validates() {
        assert!(validate_cursor_pair(
            Some(1_786_800_000),
            Some(BEFORE_ID),
            "--before-id",
            "--before"
        )
        .is_ok());
        // And a bare timestamp cursor is legal on both surfaces.
        assert!(validate_cursor_pair(Some(1_786_800_000), None, "--before-id", "--before").is_ok());
    }

    // ── `--kinds` must not substitute the default for what you asked for ──
    //
    // Measured at bff3110a0, before this change: `--kinds ''`, `--kinds '*'`
    // and `--kinds all` all exited 0 having sent the DEFAULT kind list, so a
    // caller trying to widen a pull was handed the narrow default and told it
    // worked. Each shape below is one of those commands.

    #[test]
    fn a_wildcard_kind_list_is_refused_instead_of_silently_defaulting() {
        for garbage in ["*", "all", "", "9,*", "9, ,1984", "-1", "1984abc"] {
            let err = build_messages_filter(CH, 50, None, None, None, Some(garbage))
                .expect_err("unparseable --kinds must be a usage error");
            assert!(
                err.to_string().contains("is not an event kind"),
                "unexpected error for {garbage:?}: {err}"
            );
        }
    }

    #[test]
    fn a_refused_kind_list_never_reaches_the_wire_as_the_default() {
        // The specific failure this closes: the error must not be recoverable
        // into a filter at all, so there is no shape where `*` measures [9].
        assert!(build_messages_filter(CH, 50, None, None, None, Some("*")).is_err());
        let ok = build_messages_filter(CH, 50, None, None, None, Some("7"))
            .expect("a real token still works");
        assert_eq!(ok["kinds"], serde_json::json!([7]));
    }

    #[test]
    fn whitespace_around_real_tokens_is_still_tolerated() {
        // Positive control for the stricter parser: it must reject typos
        // without also rejecting the documented ` 9, 1984 ` spelling.
        let f = build_messages_filter(CH, 50, None, None, None, Some(" 9 , 1984 "))
            .expect("padded integers parse");
        assert_eq!(f["kinds"], serde_json::json!([9, 1984]));
    }

    #[test]
    fn omitting_kinds_sends_the_documented_default_list() {
        // The default is what `--help` and any field note must quote; pin it so
        // a change to the list is a deliberate edit here.
        let f = build_messages_filter(CH, 50, None, None, None, None).expect("no kinds to parse");
        assert_eq!(
            f["kinds"],
            serde_json::json!([9, 40002, 40008, 45001, 45003])
        );
    }
}
