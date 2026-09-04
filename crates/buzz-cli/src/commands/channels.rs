use std::collections::{HashMap, HashSet};

use buzz_core::kind::{
    KIND_MANAGED_AGENT, KIND_PRESENCE_SNAPSHOT, KIND_PRESENCE_UPDATE, KIND_TEAM,
};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::client::{
    extract_d_tag, extract_p_tags, extract_tag_value, normalize_write_response,
    print_create_response, BuzzClient,
};
use crate::commands::agents::fetch_archived_snapshot;
use crate::commands::channel_templates::{self, ChannelTemplateRecord, TemplateAgentRoster};
use crate::commands::users::presence_subject;
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
    ttl_seconds: Option<i64>,
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
        let mut ttl_seconds: Option<i64> = None;

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
                "ttl" => ttl_seconds = val.and_then(|value| value.parse().ok()),
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
            ttl_seconds,
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

/// Fetch the live canvas head (kind 40100) for a channel.
///
/// The relay orders results `created_at DESC, id ASC`, so a `limit: 1` query
/// returns exactly the head every surface agrees on — no full-stream scan, no
/// silent page clamp.
///
/// `writer_pinned` opts into read-your-writes: when the head read gates a write
/// (the restore precondition, or `set`'s writer-discipline stamping) it must
/// observe the caller's own recent writes, so it sets `consistency: strong` to
/// pin the relay read to the writer pool. The display path (`get`) leaves it
/// `false` and stays replica-eligible.
async fn fetch_canvas_head(
    client: &BuzzClient,
    channel_id: &str,
    writer_pinned: bool,
) -> Result<Option<serde_json::Value>, CliError> {
    let mut filter = serde_json::json!({
        "kinds": [40100],
        "#h": [channel_id],
        "limit": 1,
    });
    if writer_pinned {
        filter["consistency"] = serde_json::json!("strong");
    }
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).map_err(|e| {
        CliError::Other(format!(
            "malformed relay response querying canvas head: {e}"
        ))
    })?;
    Ok(events.into_iter().next())
}

/// Fetch a single canvas revision by event ID, scoped to the channel and kind.
///
/// An ID-scoped query resolves revisions of any age; scanning a capped stream
/// would report a retained-but-old revision as absent.
async fn fetch_canvas_revision(
    client: &BuzzClient,
    channel_id: &str,
    revision: &str,
) -> Result<Option<serde_json::Value>, CliError> {
    let filter = serde_json::json!({
        "ids": [revision],
        "kinds": [40100],
        "#h": [channel_id],
        "limit": 1,
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).map_err(|e| {
        CliError::Other(format!(
            "malformed relay response querying canvas revision: {e}"
        ))
    })?;
    Ok(events.into_iter().next())
}

pub async fn cmd_get_canvas(
    client: &BuzzClient,
    channel_id: &str,
    revision: Option<&str>,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let selected = match revision {
        Some(revision) => {
            validate_hex64(revision)?;
            fetch_canvas_revision(client, channel_id, revision).await?
        }
        None => fetch_canvas_head(client, channel_id, false).await?,
    };
    if let Some(content) = selected
        .as_ref()
        .and_then(|e| e.get("content"))
        .and_then(|c| c.as_str())
    {
        println!("{content}");
    } else {
        println!("null");
    }
    Ok(())
}

/// List canvas revisions newest-first as JSON `{event_id, author, created_at}`.
///
/// Uses composite `(until, before_id)` pagination so a `limit` above one relay
/// page returns the full requested window instead of a silently truncated one.
/// The Clap layer bounds `limit` to 1–10000, so the request is never unbounded.
///
/// Ordering is `created_at DESC, id ASC`: revisions sharing a second break the
/// tie by smallest event id, not by which write arrived last.
pub async fn cmd_canvas_history(
    client: &BuzzClient,
    channel_id: &str,
    limit: u32,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let filter = serde_json::json!({
        "kinds": [40100],
        "#h": [channel_id],
    });
    let events = client.query_paginated(filter, limit).await?;
    let revisions: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            serde_json::json!({
                "event_id": e.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                "author": e.get("pubkey").and_then(|v| v.as_str()).unwrap_or(""),
                "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
            })
        })
        .collect();
    println!("{}", serde_json::to_string(&revisions).unwrap_or_default());
    Ok(())
}

/// Restore the canvas to a previous revision by re-publishing its content.
///
/// The target revision is fetched by an ID-scoped query (resolves any age) and
/// the head by a separate `limit: 1` query — neither scans the full stream.
/// Fetching the head immediately before the write is an optimistic
/// precondition check: a missing target revision or absent head fails here, and
/// restoring the current head is short-circuited so history never grows a
/// redundant revision. The republished event is built via
/// [`buzz_sdk::build_set_canvas_after_head`], which applies writer discipline
/// (`created_at = max(now, head.created_at + 1)`) so the restore sorts strictly
/// ahead of the head it read; the `expected-revision` tag it carries is
/// enforced by the relay as a CAS (compare-and-swap): the relay reads the
/// canonical live head under an advisory lock and rejects writes whose
/// precondition no longer matches.
///
/// After publishing, a single post-write re-read classifies whether the restore
/// still holds the visible head (or a later write legitimately built on it). If
/// a concurrent write has become current, this returns [`CliError::Conflict`]
/// (exit 5) naming the persisted revision — the restore is not lost, it survives
/// in history. If the verification read itself fails, the accepted restore is
/// reported as success (exit 0) with a stderr warning that verification was
/// unavailable — a failed read must never masquerade as a failed restore.
pub async fn cmd_restore_canvas(
    client: &BuzzClient,
    channel_id: &str,
    revision: &str,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;
    validate_hex64(revision)?;

    let content = fetch_canvas_revision(client, channel_id, revision)
        .await?
        .as_ref()
        .and_then(|e| e.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| {
            CliError::Usage(format!(
                "revision {revision} not found for channel {channel_id}"
            ))
        })?
        .to_string();

    let head = fetch_canvas_head(client, channel_id, true)
        .await?
        .ok_or_else(|| CliError::Other(format!("no canvas head found for channel {channel_id}")))?;
    let head_id = head
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CliError::Other(format!("no canvas head found for channel {channel_id}")))?;
    let head_created_at = head.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);

    // Restoring the revision that is already the head would publish a new event
    // with identical content, growing history with a redundant revision. Match
    // Desktop, which hides Restore on the current revision, by short-circuiting.
    if head_id.eq_ignore_ascii_case(revision) {
        eprintln!("revision {revision} is already the current revision");
        let _ = writeln!(
            out,
            "{}",
            serde_json::json!({
                "event_id": revision,
                "accepted": true,
                "message": "already-current",
            })
        );
        return Ok(());
    }

    let builder =
        buzz_sdk::build_set_canvas_after_head(channel_uuid, &content, head_id, head_created_at)
            .map_err(|e| CliError::Other(format!("build_set_canvas failed: {e}")))?;
    let event = client.sign_event(builder)?;
    let our_id = event.id.to_hex();
    let submit_result = client.submit_event(event).await;

    // If the relay returned a conflict-shaped error the submit may still have
    // persisted: a prior attempt stored A(expected=H), the response was lost,
    // and the retry of identical A bytes sees RevisionMismatch — a persisted
    // restore presented as a rejected write. Two concurrent-write shapes arise:
    //
    // - A tagged write B(expected=A) becomes head: `canvas_write_survived`
    //   returns true; A committed and is an accepted ancestor → success.
    // - An unconditional (legacy) write B(no expected-revision) becomes head:
    //   `canvas_write_survived` returns false because B carries no ancestry link
    //   to A, yet A may still be durably in the stream. A bounded ancestry walk
    //   alone cannot distinguish "A is absent" from "A is present but not an
    //   ancestor of head."
    //
    // Fix: establish A's persistence via a separate writer-pinned IDs lookup,
    // then feed the four-way classification:
    //   - survived (ancestor of head)         → accepted JSON, exit 0
    //   - persisted but head unrelated to A   → supersession naming A
    //   - genuinely absent                    → original 409 rejection
    //   - either reconciliation read fails    → DeliveryUnknown naming A
    let resp = match submit_result {
        Ok(resp) => resp,
        Err(e @ CliError::Relay { status: 409, .. }) if matches!(&e, CliError::Relay { body, .. } if body.contains("canvas changed")) =>
        {
            // Both reads use strong consistency so read-replica lag cannot
            // report a just-stored event as absent.
            let stream = match fetch_canvas_ancestry(client, channel_id).await {
                Ok(s) => s,
                Err(_) => {
                    return Err(CliError::DeliveryUnknown(format!(
                        "canvas restore {our_id}: submit returned a conflict and the post-submit ancestry read failed; outcome unknown — check `buzz canvas history` before retrying"
                    )));
                }
            };
            if buzz_sdk::canvas_write_survived(&our_id, &stream) {
                // A is an ancestor of the live head: it committed and a later
                // write built on it. This is the same committed state the
                // normal accepted-submit path treats as success — return
                // accepted JSON so the caller is not prompted to re-restore.
                // The relay's 409 response body is not an accepted response;
                // synthesize one from the event ID we know committed.
                let accepted_json = serde_json::json!({
                    "event_id": our_id,
                    "accepted": true,
                    "message": "",
                })
                .to_string();
                let _ = writeln!(out, "{accepted_json}");
                return Ok(());
            }
            // A is not a reachable ancestor. Distinguish absence from presence
            // with a separate writer-pinned IDs lookup (an unconditional legacy
            // write can become head without linking back to A, so the bounded
            // ancestry stream alone cannot confirm A exists).
            match fetch_canvas_event_exists(client, channel_id, &our_id).await {
                Ok(true) => {
                    // A committed but is no longer an ancestor of the current
                    // head: an unconditional write superseded it. Surface as a
                    // post-write supersession so the caller knows A is durable.
                    return Err(CliError::Conflict(format!(
                        "canvas restore {our_id} was superseded by a concurrent write; it is preserved in history — re-run restore against the current head if you still want it"
                    )));
                }
                Ok(false) => {
                    // A is genuinely absent: return the original relay error unchanged.
                    return Err(e);
                }
                Err(_) => {
                    // Cannot confirm or deny whether A was stored.
                    return Err(CliError::DeliveryUnknown(format!(
                        "canvas restore {our_id}: submit returned a conflict and the post-submit existence check failed; outcome unknown — check `buzz canvas history` before retrying"
                    )));
                }
            }
        }
        Err(e) => return Err(e),
    };

    // Post-write supersession check. Re-read a recent slice of the revision
    // stream and classify via the SDK's shared predicate: our event is the
    // head, or reachable through the head's `expected-revision` ancestry chain
    // (a later write legitimately built on ours, possibly transitively) → the
    // restore is live; anything else (including no head) → a concurrent write
    // won the visible head. The restored revision is preserved in history, so
    // surface a conflict naming it rather than reporting a hollow success. This
    // only catches a competitor visible by now; the residual race past this
    // read needs relay linearization (phase 2).
    //
    // The submit above was accepted, so the restore is durable. Only this
    // verification read can still fail; when it does we must not present an
    // accepted publish as a failed restore. Print the normal success response
    // naming `our_id`, warn on stderr that verification was unavailable, and
    // exit 0 — a failed read is not a conflict.
    let live_stream = match fetch_canvas_ancestry(client, channel_id).await {
        Ok(stream) => stream,
        Err(_) => {
            eprintln!(
                "warning: canvas restore {our_id} was published but its post-write verification read failed; the restore is preserved in history — check `buzz canvas history` if a concurrent edit may have landed"
            );
            let _ = writeln!(out, "{}", normalize_write_response(&resp));
            return Ok(());
        }
    };
    if !buzz_sdk::canvas_write_survived(&our_id, &live_stream) {
        return Err(CliError::Conflict(format!(
            "canvas restore {our_id} was superseded by a concurrent write; it is preserved in history — re-run restore against the current head if you still want it"
        )));
    }

    let _ = writeln!(out, "{}", normalize_write_response(&resp));
    Ok(())
}

/// Read a recent slice of the canvas revision stream as `(event_id,
/// expected-revision tag)` pairs, newest first, for the post-write supersession
/// check. The head is the first element; the rest let
/// [`buzz_sdk::canvas_write_survived`] walk `expected-revision` links back
/// through a descendant chain (A→B→C) so a legitimate later write layered on
/// ours is not misread as a supersession. An empty vec means no canvas exists.
async fn fetch_canvas_ancestry(
    client: &BuzzClient,
    channel_id: &str,
) -> Result<Vec<(String, Option<String>)>, CliError> {
    let filter = serde_json::json!({
        "kinds": [40100],
        "#h": [channel_id],
        "limit": buzz_sdk::CANVAS_ANCESTRY_WALK_MAX,
        // Read-your-writes: this post-write verification read must observe the
        // restore we just published, so it pins to the writer pool.
        "consistency": "strong",
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).map_err(|e| {
        CliError::Other(format!(
            "malformed relay response querying canvas head: {e}"
        ))
    })?;
    Ok(events
        .iter()
        .filter_map(|event| {
            let id = event.get("id").and_then(|v| v.as_str())?.to_string();
            Some((id, extract_head_expected_revision(event)))
        })
        .collect())
}

/// Whether a canvas event with `event_id` exists in this channel's history,
/// using a writer-pinned (strong-consistency) read so the caller is not
/// reporting a truly-persisted event as absent due to read-replica lag.
///
/// Used by the 409-reconciliation branch to establish persistence independently
/// of ancestry position: an unconditional ("legacy") write can become head
/// without building an `expected-revision` link to the event being checked,
/// so a bounded ancestry walk alone cannot confirm presence.
async fn fetch_canvas_event_exists(
    client: &BuzzClient,
    channel_id: &str,
    event_id: &str,
) -> Result<bool, CliError> {
    let filter = serde_json::json!({
        "ids": [event_id],
        "kinds": [40100],
        "#h": [channel_id],
        "limit": 1,
        "consistency": "strong",
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).map_err(|e| {
        CliError::Other(format!(
            "malformed relay response checking canvas event existence: {e}"
        ))
    })?;
    Ok(!events.is_empty())
}

/// The head's own `["expected-revision", …]` tag value (the id it built on), or
/// `None` when absent — used by the post-write supersession check to recognize
/// a later write that legitimately layered on top of ours.
fn extract_head_expected_revision(head: &serde_json::Value) -> Option<String> {
    head.get("tags")
        .and_then(|t| t.as_array())
        .and_then(|tags| {
            tags.iter().find(|t| {
                t.as_array()
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    == Some("expected-revision")
            })
        })
        .and_then(|t| t.as_array())
        .and_then(|a| a.get(1))
        .and_then(|v| v.as_str())
        .map(str::to_string)
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

/// Outcome of the pure F4 cardinality rule alone (see
/// [`apply_cardinality_rule`]) — zero live instances is a plain "skipped"
/// slug here, with no notion of *why* (archive filtering happens one layer
/// up, in [`build_roster_resolution`]).
#[derive(Debug)]
struct ResolvedRoster {
    /// Exactly one live instance per persona slug — safe to add.
    agents: Vec<ResolvedAgent>,
    /// Persona slugs with no live kind:30177 instance for the effective
    /// owner (cold-start provisioning is desktop-only, out of scope here).
    skipped: Vec<String>,
}

/// One live instance dropped from the roster because it's archived
/// (NIP-IA) — reported so an operator can see exactly what was excluded,
/// not just that a slug ended up short.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ArchivedExclusion {
    persona_id: String,
    pubkey: String,
}

/// A persona slug with no agent added to the roster, and why. Distinguishes
/// "no live instances ever existed" from "instances existed but all are
/// archived" — both look the same as a bare skip otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SkippedSlug {
    persona_id: String,
    reason: String,
}

/// Archive-aware roster resolution: the [`apply_cardinality_rule`] outcome
/// after archived instances are filtered out first, plus what the archive
/// filter itself excluded and whether its snapshot could be trusted.
#[derive(Debug)]
struct RosterResolution {
    /// Exactly one live, non-archived instance per persona slug.
    agents: Vec<ResolvedAgent>,
    skipped: Vec<SkippedSlug>,
    /// The complete archive-exclusion list, never just the first.
    archived_excluded: Vec<ArchivedExclusion>,
    /// Set when the archived-identities snapshot failed its trust check
    /// (NIP-IA state 3) and the filter fell back to "no known archived
    /// identities" rather than fabricating a resolution. `None` means the
    /// snapshot was trusted (states 1/2) or there was nothing to filter.
    archive_state_warning: Option<String>,
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

/// Best-effort hints for a candidate agent pubkey, used to annotate the
/// duplicate-instance error. Gathered from relay presence and kind:0 lookups
/// before cardinality runs — both are optional so a lookup failure never
/// becomes a new failure mode.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CandidateHint {
    /// Latest presence status from kind:40902 (`"online"`, `"offline"`, or
    /// whatever string the relay holds). `None` if the lookup failed or
    /// returned no event.
    presence: Option<String>,
    /// `created_at` timestamp from the agent's kind:0 profile event — the
    /// time of the last profile update (kind:0 is replaceable; desktop
    /// republishes it on rename and profile reconciliation). `None` if the
    /// lookup failed or returned nothing.
    profile_updated_at: Option<u64>,
}

/// Fetch best-effort presence (kind:40902) and kind:0 metadata for each
/// pubkey in `pubkeys`. Each query is bounded *independently* by `timeout` and
/// the two outcomes are joined, so a lookup that completes survives a sibling
/// that hangs (see [`join_bounded_queries`]). Returns a map from pubkey to
/// hints; pubkeys with failed or absent lookups are absent from the map rather
/// than causing an error — callers must handle the missing-hint case. On
/// timeout or relay error, returns whatever partial hints were collected
/// (possibly an empty map) so the caller can still print bare pubkeys promptly.
///
/// Only called when duplicate candidates have been detected: happy-path
/// resolutions perform zero hint queries.
async fn fetch_candidate_hints(
    client: &BuzzClient,
    pubkeys: &[String],
    timeout: std::time::Duration,
) -> HashMap<String, CandidateHint> {
    if pubkeys.is_empty() {
        return HashMap::new();
    }

    // Presence: kind:40902, relay-synthesized on demand.
    let presence_filter = serde_json::json!({
        "kinds": [KIND_PRESENCE_SNAPSHOT],
        "authors": pubkeys,
        "limit": pubkeys.len(),
    });
    // Profile: kind:0 replaceable head per author.
    let profile_filter = serde_json::json!({
        "kinds": [0],
        "authors": pubkeys,
        "limit": pubkeys.len(),
    });

    let (presence_result, profile_result) = join_bounded_queries(
        timeout,
        client.query(&presence_filter),
        client.query(&profile_filter),
    )
    .await;

    hints_from_results(pubkeys, presence_result, profile_result)
}

/// Run two relay queries concurrently, bounding *each* independently by
/// `timeout` and joining the outcomes. A per-query timeout maps to `Err`, so a
/// completed lookup is never discarded because its sibling hung — the fail-soft
/// contract requires partial enrichment to survive. The whole call still
/// returns within `timeout` because neither branch can outlast it.
async fn join_bounded_queries<P, Q>(
    timeout: std::time::Duration,
    presence: P,
    profile: Q,
) -> (Result<String, CliError>, Result<String, CliError>)
where
    P: std::future::Future<Output = Result<String, CliError>>,
    Q: std::future::Future<Output = Result<String, CliError>>,
{
    tokio::join!(
        async {
            tokio::time::timeout(timeout, presence)
                .await
                .unwrap_or_else(|_| Err(CliError::Other("presence hint timeout".to_string())))
        },
        async {
            tokio::time::timeout(timeout, profile)
                .await
                .unwrap_or_else(|_| Err(CliError::Other("profile hint timeout".to_string())))
        },
    )
}

/// Convert the raw presence and profile query outcomes into a hint map.
///
/// A presence response is trusted as a **complete snapshot** for the requested
/// `pubkeys` only when it parses as a JSON array in which *every* element is a
/// relay-synthesized presence event — a complete signed [`nostr::Event`] of
/// kind [`KIND_PRESENCE_UPDATE`] carrying exactly one `p` tag whose subject is
/// one of the requested `pubkeys` (see [`trusted_presence_snapshot`]). The
/// relay drops the Redis
/// presence key when an identity goes offline, so a trusted snapshot that omits
/// a requested pubkey means that pubkey is offline — exactly the stale
/// duplicate an operator needs flagged. Omitted pubkeys are therefore seeded as
/// `offline`, then returned statuses overlay the seed.
///
/// Anything less than a fully trusted array — a failed/timed-out query, invalid
/// top-level JSON, or an array containing any element that is not such an event
/// (a vacuous object, `[{}]`, `[null]`, an event of the wrong kind, or one for
/// an unrequested subject) — makes presence enrichment untrusted: no offline
/// seeding and no presence labels at all. A completed profile sibling still
/// contributes its hints in that case. This refuses to invent an `offline`
/// label from a response we cannot trust (a relay-side fake-empty success or a
/// partially malformed body). Kept separate from IO so the trust boundary is
/// directly unit-testable without a relay.
fn hints_from_results(
    pubkeys: &[String],
    presence_result: Result<String, CliError>,
    profile_result: Result<String, CliError>,
) -> HashMap<String, CandidateHint> {
    let (offline_seed, presence_events): (&[String], Vec<serde_json::Value>) =
        match trusted_presence_snapshot(pubkeys, presence_result) {
            Some(events) => (pubkeys, events),
            None => (&[], Vec::new()),
        };
    let profile_events: Vec<serde_json::Value> = profile_result
        .ok()
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default();

    build_hint_map(offline_seed, &presence_events, &profile_events)
}

/// Validate a presence query outcome as a trustworthy complete snapshot.
///
/// Returns the parsed events only when the body parses as a JSON array and
/// *every* element is a relay-synthesized presence snapshot for the requested
/// set: a complete, well-formed [`nostr::Event`] of kind
/// [`KIND_PRESENCE_UPDATE`] carrying exactly one `p` tag whose subject is one of
/// `pubkeys`. A failed query, non-array JSON, or any element that is not such
/// an event yields `None` — the caller must then treat presence as untrusted
/// and never infer `offline`.
///
/// Parsing each element as a full event (not just checking two fields) is what
/// stops a vacuous object like `{"pubkey":"…","content":"online"}` — which
/// lacks `id`/`sig`/`kind`/`created_at` — from masquerading as a snapshot; the
/// kind check rejects a fully-shaped event of the wrong kind; and validating the
/// *sole* `p`-tag subject (the exact value the consumer reads) rejects an event
/// for an unrequested subject as well as a mixed-tag event that would pass a
/// weaker "any `p` tag is requested" check yet overlay a different subject
/// downstream. Any of these would otherwise re-enable false `offline` seeding
/// from an untrustworthy body.
fn trusted_presence_snapshot(
    pubkeys: &[String],
    presence_result: Result<String, CliError>,
) -> Option<Vec<serde_json::Value>> {
    let events: Vec<serde_json::Value> = presence_result
        .ok()
        .and_then(|r| serde_json::from_str(&r).ok())?;
    let requested: HashSet<&str> = pubkeys.iter().map(String::as_str).collect();
    let all_trusted = events.iter().all(|value| {
        // Must parse as a complete signed event of the presence-update kind.
        let Ok(event) = serde_json::from_value::<nostr::Event>(value.clone()) else {
            return false;
        };
        event.kind == nostr::Kind::Custom(KIND_PRESENCE_UPDATE as u16)
            // Require the *sole* `p`-tag subject — the one `build_hint_map`
            // consumes via `presence_subject` — to be requested. Reading the
            // same single subject the consumer reads is what prevents a
            // mixed-tag event (`[["p","<unrequested>"],["p","<requested>"]]`)
            // from passing here yet overlaying a different subject downstream.
            && sole_p_tag_subject(value).is_some_and(|s| requested.contains(s))
    });
    all_trusted.then_some(events)
}

/// The subject of the event's single `p` tag, or `None` unless there is exactly
/// one `p` tag carrying a string subject. The relay synthesizes presence
/// snapshots with exactly one `p` tag (the subject); requiring exactly one keeps
/// this validator reading the same subject that `presence_subject` (which takes
/// the first `p` tag) consumes in `build_hint_map`, so a mixed- or
/// malformed-tag event cannot pass validation and then overlay a different
/// subject.
fn sole_p_tag_subject(event: &serde_json::Value) -> Option<&str> {
    let tags = event.get("tags")?.as_array()?;
    let mut p_subjects = tags
        .iter()
        .filter_map(|tag| match tag.as_array()?.as_slice() {
            [name, subject, ..] if name == "p" => Some(subject.as_str()),
            _ => None,
        });
    let first = p_subjects.next()?;
    if p_subjects.next().is_some() {
        return None; // more than one `p` tag → outside the single-subject contract
    }
    first // the sole `p` tag's subject, or `None` if it was not a string
}

/// Pure response-to-map conversion: takes the raw presence (kind:40902) and
/// profile (kind:0) event slices returned by the relay and builds the
/// per-pubkey hint map. Extracted as a sync function so it is directly
/// unit-testable without a relay.
///
/// `offline_seed` names the pubkeys whose presence was requested via a
/// response the caller trusts as a complete snapshot; each is pre-labeled
/// `offline` before overlaying returned statuses, so a duplicate the relay
/// omitted (its Redis key was dropped on going offline) is still flagged
/// `offline` rather than left blank. Pass an empty slice when the presence
/// response failed, timed out, or was malformed — never infer offline then.
///
/// Presence subject is the `p`-tag value when present (relay signs the event
/// and embeds the agent pubkey there), otherwise the event author.
fn build_hint_map(
    offline_seed: &[String],
    presence_events: &[serde_json::Value],
    profile_events: &[serde_json::Value],
) -> HashMap<String, CandidateHint> {
    let mut hints: HashMap<String, CandidateHint> = HashMap::new();

    // Seed requested pubkeys as offline: a trusted snapshot that omits a
    // requested pubkey means that identity is offline.
    for pubkey in offline_seed {
        hints
            .entry(pubkey.clone())
            .or_insert(CandidateHint {
                presence: None,
                profile_updated_at: None,
            })
            .presence = Some("offline".to_string());
    }

    for event in presence_events {
        let subject = presence_subject(event).to_string();
        if subject.is_empty() {
            continue;
        }
        let status = event
            .get("content")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        // Only overlay a real status string; a returned event with no readable
        // content must not erase an offline seed for the same pubkey.
        if let Some(status) = status {
            hints
                .entry(subject)
                .or_insert(CandidateHint {
                    presence: None,
                    profile_updated_at: None,
                })
                .presence = Some(status);
        }
    }

    for event in profile_events {
        let Some(pubkey) = event
            .get("pubkey")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        else {
            continue;
        };
        let profile_updated_at = event.get("created_at").and_then(|v| v.as_u64());
        hints
            .entry(pubkey)
            .or_insert(CandidateHint {
                presence: None,
                profile_updated_at: None,
            })
            .profile_updated_at = profile_updated_at;
    }

    hints
}

/// Format a single candidate pubkey for the duplicate-instance error,
/// appending available hint fields in brackets. Pure and testable.
///
/// Examples:
/// - `"aaa…bbb [online, profile updated 2024-01-15]"`
/// - `"aaa…bbb [offline]"`
/// - `"aaa…bbb [profile updated 2024-01-15]"`
/// - `"aaa…bbb"` (no hint at all)
fn format_candidate(pubkey: &str, hint: Option<&CandidateHint>) -> String {
    let Some(h) = hint else {
        return pubkey.to_string();
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(status) = &h.presence {
        parts.push(status.clone());
    }
    if let Some(ts) = h.profile_updated_at {
        // Use chrono for safe conversion; omit the date if the timestamp is
        // out of range rather than panicking or printing garbage.
        if let Some(dt) = DateTime::from_timestamp(ts as i64, 0) {
            parts.push(format!("profile updated {}", dt.format("%Y-%m-%d")));
        }
    }
    if parts.is_empty() {
        pubkey.to_string()
    } else {
        format!("{pubkey} [{}]", parts.join(", "))
    }
}

/// Apply the F4 cardinality rule per persona slug: zero live instances is a
/// known skip (cold-start provisioning is desktop-only, out of scope), one is
/// added, more than one is a hard error listing candidate pubkeys — matching
/// all instances silently would risk adding a stale or wrong instance. Pure
/// and independent of the relay so it's directly unit-testable.
///
/// `hints` is best-effort decoration gathered by the async caller before this
/// function runs: absent entries are silently omitted from the error, never a
/// new failure mode.
fn apply_cardinality_rule(
    slugs: &[String],
    found: &[ResolvedAgent],
    hints: &HashMap<String, CandidateHint>,
) -> Result<ResolvedRoster, CliError> {
    let mut agents = Vec::new();
    let mut skipped = Vec::new();
    for slug in slugs {
        let matches: Vec<&ResolvedAgent> = found.iter().filter(|a| &a.persona_id == slug).collect();
        match matches.as_slice() {
            [] => skipped.push(slug.clone()),
            [one] => agents.push((*one).clone()),
            many => {
                let candidates: Vec<String> = many
                    .iter()
                    .map(|a| format_candidate(&a.pubkey, hints.get(&a.pubkey)))
                    .collect();
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

/// The stderr warning + embedded-in-report/error-detail message for an
/// untrusted archived-identities snapshot (NIP-IA state 3) — one wording,
/// shared by [`build_roster_resolution`]'s immediate stderr print and
/// [`resolve_roster_with_archive_filter`]'s `archive_state_warning`/error-
/// detail paths, so the two never drift.
fn archive_snapshot_warning(e: &CliError) -> String {
    format!("archived-identities snapshot untrusted, proceeding without archive filtering: {e}")
}

/// Pure archive-filter + cardinality core of [`build_roster_resolution`],
/// taking the already-fetched `found` set and the already-attempted archive
/// snapshot fetch (`archived_result`) so it's directly unit-testable without
/// any relay I/O — same separation as [`apply_cardinality_rule`] vs
/// [`scan_managed_agents_by_owner`].
///
/// Archive filtering deliberately **fails open** on `archived_result: Err`
/// (NIP-IA snapshot state 3): filtering with an empty archived set rather
/// than fabricating a resolution, since filtering can only *remove*
/// ambiguity, never resolve it. The trust failure is always surfaced: on
/// stderr immediately by the caller (network-adjacent, not this function's
/// job), and in whichever of `RosterResolution::archive_state_warning` or
/// the returned error the caller ends up on here.
fn resolve_roster_with_archive_filter(
    slugs: &[String],
    found: Vec<ResolvedAgent>,
    archived_result: Result<Vec<String>, CliError>,
    hints: &HashMap<String, CandidateHint>,
) -> Result<RosterResolution, CliError> {
    let (archived, archive_state_warning) = match archived_result {
        Ok(pubkeys) => (pubkeys.into_iter().collect::<HashSet<String>>(), None),
        Err(e) => (HashSet::new(), Some(archive_snapshot_warning(&e))),
    };

    let mut archived_excluded = Vec::new();
    let mut live_found = Vec::new();
    for agent in found {
        if archived.contains(&agent.pubkey) {
            archived_excluded.push(ArchivedExclusion {
                persona_id: agent.persona_id.clone(),
                pubkey: agent.pubkey.clone(),
            });
        } else {
            live_found.push(agent);
        }
    }

    let resolved = apply_cardinality_rule(slugs, &live_found, hints).map_err(|e| {
        match (e, &archive_state_warning) {
            (CliError::Usage(msg), Some(warning)) => {
                CliError::Usage(format!("{msg} (warning: {warning})"))
            }
            (other, _) => other,
        }
    })?;

    let archived_slugs: HashSet<&str> = archived_excluded
        .iter()
        .map(|a| a.persona_id.as_str())
        .collect();
    let skipped = resolved
        .skipped
        .into_iter()
        .map(|persona_id| {
            let reason = if archived_slugs.contains(persona_id.as_str()) {
                "all instances archived".to_string()
            } else {
                "no live instances".to_string()
            };
            SkippedSlug { persona_id, reason }
        })
        .collect();

    Ok(RosterResolution {
        agents: resolved.agents,
        skipped,
        archived_excluded,
        archive_state_warning,
    })
}

/// Sync tail of [`build_roster_resolution`]: emits the state-3 trust-failure
/// warning to `warn_sink` (production: stderr) and then runs
/// [`resolve_roster_with_archive_filter`]. Split out from the async relay
/// I/O so the emission and the resolution it gates are exercised together
/// through an injected writer — the only way to prove the emission is still
/// wired to the same trust check the resolution result carries.
fn finalize_roster_resolution(
    slugs: &[String],
    found: Vec<ResolvedAgent>,
    archived_result: Result<Vec<String>, CliError>,
    hints: &HashMap<String, CandidateHint>,
    warn_sink: &mut dyn std::io::Write,
) -> Result<RosterResolution, CliError> {
    if let Err(e) = &archived_result {
        let warning = archive_snapshot_warning(e);
        let _ = writeln!(warn_sink, "{}", serde_json::json!({"warning": warning}));
    }
    resolve_roster_with_archive_filter(slugs, found, archived_result, hints)
}

/// Post-fetch stage of [`build_roster_resolution`]: given the already-fetched
/// `found` and `archived_result`, identifies duplicate live instances, calls
/// `fetch_hints` only for their pubkeys, then delegates to
/// [`finalize_roster_resolution`].
///
/// Accepting `fetch_hints` as a generic async closure makes this function
/// directly testable without a relay: tests pass a recording closure that
/// asserts the exact pubkey set and returns a controlled hint map.
///
/// - **Happy path** (no duplicates): `fetch_hints` is never called.
/// - **Trusted archive archives one of a pair**: only the surviving live pair
///   triggers `fetch_hints`; archived instances are not fetched for.
/// - **Untrusted archive** (`archived_result: Err`): all found instances are
///   conservatively treated as live for duplicate detection.
async fn assemble_roster_resolution<F, Fut>(
    slugs: &[String],
    found: Vec<ResolvedAgent>,
    archived_result: Result<Vec<String>, CliError>,
    fetch_hints: F,
    warn_sink: &mut dyn std::io::Write,
) -> Result<RosterResolution, CliError>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = HashMap<String, CandidateHint>>,
{
    // Determine which pubkeys belong to duplicate live instances after archive
    // filtering. Untrusted archive (Err) → empty archived set → conservative.
    let duplicate_pubkeys: Vec<String> = {
        let archived_set: HashSet<&str> = match &archived_result {
            Ok(keys) => keys.iter().map(String::as_str).collect(),
            Err(_) => HashSet::new(),
        };
        let live: Vec<&ResolvedAgent> = found
            .iter()
            .filter(|a| !archived_set.contains(a.pubkey.as_str()))
            .collect();
        let mut slug_count: HashMap<&str, Vec<&str>> = HashMap::new();
        for a in &live {
            slug_count
                .entry(a.persona_id.as_str())
                .or_default()
                .push(a.pubkey.as_str());
        }
        slug_count
            .into_values()
            .filter(|pks| pks.len() > 1)
            .flatten()
            .map(str::to_string)
            .collect()
    };

    let hints = if duplicate_pubkeys.is_empty() {
        HashMap::new()
    } else {
        fetch_hints(duplicate_pubkeys).await
    };

    finalize_roster_resolution(slugs, found, archived_result, &hints, warn_sink)
}

/// Resolve a template's roster against the relay: expand team entries into
/// persona slugs (via kind:30176), scan for live kind:30177 instances scoped
/// to the effective owner, filter out archived (NIP-IA) instances, and apply
/// the cardinality rule per slug (see [`resolve_roster_with_archive_filter`]
/// for the pure filter+cardinality core and the fail-open contract). Runs
/// entirely before any channel-creation side effect — a cardinality error
/// aborts with nothing created.
///
/// Hint fetching is zero-cost on the happy path: [`assemble_roster_resolution`]
/// only invokes the hint fetcher when duplicate live instances are detected
/// after archive filtering. Queries run concurrently and are bounded by a
/// 3-second timeout; on expiry the error prints with bare pubkeys.
async fn build_roster_resolution(
    client: &BuzzClient,
    owner: &str,
    roster: &TemplateAgentRoster,
) -> Result<RosterResolution, CliError> {
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
        return Ok(RosterResolution {
            agents: Vec::new(),
            skipped: Vec::new(),
            archived_excluded: Vec::new(),
            archive_state_warning: None,
        });
    }

    let slug_set: HashSet<&str> = slugs.iter().map(String::as_str).collect();
    let (found, archived_result) = tokio::join!(
        scan_managed_agents_by_owner(client, owner, &slug_set),
        fetch_archived_snapshot(client),
    );
    let found = found?;

    assemble_roster_resolution(
        &slugs,
        found,
        archived_result,
        |pks| async move {
            fetch_candidate_hints(client, &pks, std::time::Duration::from_secs(3)).await
        },
        &mut std::io::stderr(),
    )
    .await
}

/// `buzz channels create --template <name>`: load a desktop-local channel
/// template, resolve its agent roster against the relay, create the
/// channel, apply the canvas template, and add resolved agents as members.
///
/// Roster resolution happens entirely before channel creation (see
/// `build_roster_resolution`) so an ambiguous roster aborts with zero side
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

    let resolved = build_roster_resolution(client, &owner, &template.agents).await?;

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
            // Fresh channel: no head can exist yet, so assert the create with
            // `Some("none")` to match the create-assertion convention.
            let builder = buzz_sdk::build_set_canvas(channel_uuid, &content, Some("none"))
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
    let report = build_template_report(
        &channel_uuid.to_string(),
        &template.name,
        status,
        canvas_applied,
        members_added,
        member_failures,
        &resolved,
    );
    println!("{report}");
    Ok(())
}

/// Pure construction of `channels create --template`'s final stdout report.
/// Isolated from `cmd_create_channel_from_template` so the `archive_state_warning`
/// insertion is directly testable against a `RosterResolution` without any
/// relay I/O or channel-creation side effects — production prints exactly
/// what this returns.
#[allow(clippy::too_many_arguments)]
fn build_template_report(
    channel_id: &str,
    template_name: &str,
    status: &str,
    canvas_applied: bool,
    members_added: Vec<serde_json::Value>,
    member_failures: Vec<serde_json::Value>,
    resolved: &RosterResolution,
) -> serde_json::Value {
    let mut report = serde_json::json!({
        "status": status,
        "channel_id": channel_id,
        "template": template_name,
        "canvas_applied": canvas_applied,
        "members_added": members_added,
        "skipped": resolved.skipped,
        "archived_excluded": resolved.archived_excluded,
        "member_failures": member_failures,
    });
    if let Some(warning) = &resolved.archive_state_warning {
        report["archive_state_warning"] = serde_json::Value::String(warning.clone());
    }
    report
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

fn validate_update_channel_fields(
    name: Option<&str>,
    description: Option<&str>,
    visibility: Option<&str>,
    ttl_change: Option<Option<i32>>,
) -> Result<(), CliError> {
    if name.is_none() && description.is_none() && visibility.is_none() && ttl_change.is_none() {
        return Err(CliError::Usage(
            "at least one field required (--name, --description, --visibility, --ttl, --no-ttl)"
                .into(),
        ));
    }
    Ok(())
}

pub async fn cmd_update_channel(
    client: &BuzzClient,
    channel_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    visibility: Option<&str>,
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

    validate_update_channel_fields(name, description, visibility, ttl_change)?;
    let channel_uuid = parse_uuid(channel_id)?;

    let builder =
        buzz_sdk::build_update_channel(channel_uuid, name, description, visibility, ttl_change)
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

    // `set` is an unconditional replace — no `expected-revision` tag, so the
    // relay accepts the write regardless of the current head. A moved head is
    // not an error; there is no post-write supersession check.
    //
    // Writer discipline still applies: read the head and stamp
    // `created_at = max(now, head + 1)` so the write sorts strictly ahead of a
    // newer or future-dated head under `created_at DESC, id ASC`. This keeps
    // an accepted append from landing behind the current head in read order,
    // which would "succeed" without changing the visible canvas.
    //
    // The skew guard applies: a head timestamped more than CANVAS_MAX_FUTURE_SKEW_SECS
    // in the future is refused with a clear error rather than extending a poisoned
    // timeline. With no head yet, `build_set_canvas` with no tag stamps at `now`.
    let builder = match fetch_canvas_head(client, channel_id, true).await? {
        Some(head) => {
            let head_created_at = head.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);
            buzz_sdk::build_set_canvas_unconditional_after_head(
                channel_uuid,
                &content,
                head_created_at,
            )
        }
        None => buzz_sdk::build_set_canvas(channel_uuid, &content, None),
    }
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
            visibility,
            ttl,
            no_ttl,
        } => {
            let visibility = visibility.as_ref().map(|v| v.to_string());
            cmd_update_channel(
                client,
                &channel,
                name.as_deref(),
                description.as_deref(),
                visibility.as_deref(),
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
        CanvasCmd::Get { channel, revision } => {
            cmd_get_canvas(client, &channel, revision.as_deref()).await
        }
        CanvasCmd::Set { channel, content } => cmd_set_canvas(client, &channel, &content).await,
        CanvasCmd::History { channel, limit } => cmd_canvas_history(client, &channel, limit).await,
        CanvasCmd::Restore { channel, revision } => {
            cmd_restore_canvas(client, &channel, &revision, &mut std::io::stdout()).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_cardinality_rule, assemble_roster_resolution, build_hint_map, build_template_report,
        cmd_set_add_policy, fetch_candidate_hints, finalize_roster_resolution, format_candidate,
        hints_from_results, join_bounded_queries, name_matches, resolve_roster_with_archive_filter,
        validate_ttl_seconds, validate_update_channel_fields, ArchivedExclusion, CandidateHint,
        ChannelSummary, ResolvedAgent, RosterResolution, SkippedSlug,
    };
    use crate::client::BuzzClient;
    use crate::CliError;
    use serde_json::json;
    use std::collections::HashMap;

    fn event(tags: serde_json::Value) -> serde_json::Value {
        json!({ "tags": tags })
    }

    fn no_hints() -> HashMap<String, CandidateHint> {
        HashMap::new()
    }

    fn hint(presence: Option<&str>, profile_updated_at: Option<u64>) -> CandidateHint {
        CandidateHint {
            presence: presence.map(str::to_string),
            profile_updated_at,
        }
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
            ["ttl", "3600"],
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
        assert_eq!(s.ttl_seconds, Some(3600));
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

    #[test]
    fn update_channel_fields_rejects_empty_update() {
        let result = validate_update_channel_fields(None, None, None, None);
        assert!(matches!(result, Err(CliError::Usage(_))));
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("at least one field required"));
        assert!(msg.contains("--visibility"));
    }

    #[test]
    fn update_channel_fields_accepts_visibility_only_update() {
        let result = validate_update_channel_fields(None, None, Some("open"), None);
        assert!(result.is_ok(), "visibility-only update should be accepted");
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
        let resolved =
            apply_cardinality_rule(&slugs, &[], &no_hints()).expect("zero instances is not fatal");
        assert!(resolved.agents.is_empty());
        assert_eq!(resolved.skipped, vec!["builtin:fizz".to_string()]);
    }

    #[test]
    fn cardinality_one_instance_is_added() {
        let slugs = vec!["builtin:fizz".to_string()];
        let found = vec![agent("builtin:fizz", "a".repeat(64).as_str())];
        let resolved =
            apply_cardinality_rule(&slugs, &found, &no_hints()).expect("single instance resolves");
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
        let err = apply_cardinality_rule(&slugs, &found, &no_hints()).unwrap_err();
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
        let err = apply_cardinality_rule(&slugs, &found, &no_hints()).unwrap_err();
        assert!(err.to_string().contains("builtin:duplicated"));
    }

    #[test]
    fn cardinality_empty_roster_resolves_to_empty_lists() {
        let resolved =
            apply_cardinality_rule(&[], &[], &no_hints()).expect("empty roster is not fatal");
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
        let resolved = apply_cardinality_rule(&slugs, &found, &no_hints()).expect("resolves");
        assert_eq!(resolved.agents.len(), 1);
        assert_eq!(resolved.agents[0].persona_id, "builtin:fizz");
    }

    // --- PR B: archive-aware roster resolution (resolve_roster_with_archive_filter) ---

    #[test]
    fn archive_filter_drops_archived_instance_and_resolves_to_live_one() {
        // Two instances of the same persona, one archived: the archived one
        // is dropped before cardinality runs, so the slug resolves cleanly
        // to the live instance instead of hitting the multi-instance error.
        let slugs = vec!["builtin:fizz".to_string()];
        let live_pk = "a".repeat(64);
        let archived_pk = "b".repeat(64);
        let found = vec![
            agent("builtin:fizz", &live_pk),
            agent("builtin:fizz", &archived_pk),
        ];
        let resolution = resolve_roster_with_archive_filter(
            &slugs,
            found,
            Ok(vec![archived_pk.clone()]),
            &no_hints(),
        )
        .expect("resolves to the single live instance");
        assert_eq!(resolution.agents.len(), 1);
        assert_eq!(resolution.agents[0].pubkey, live_pk);
        assert!(resolution.skipped.is_empty());
        assert_eq!(
            resolution.archived_excluded,
            vec![ArchivedExclusion {
                persona_id: "builtin:fizz".to_string(),
                pubkey: archived_pk,
            }]
        );
        assert!(resolution.archive_state_warning.is_none());
    }

    #[test]
    fn archive_filter_all_instances_archived_is_skipped_with_explicit_reason() {
        // Every live instance for the slug is archived: distinguishable
        // from "no instances ever existed" via the skip reason, and both
        // exclusions appear in the complete archived_excluded list.
        let slugs = vec!["builtin:fizz".to_string()];
        let pk1 = "a".repeat(64);
        let pk2 = "b".repeat(64);
        let found = vec![agent("builtin:fizz", &pk1), agent("builtin:fizz", &pk2)];
        let resolution = resolve_roster_with_archive_filter(
            &slugs,
            found,
            Ok(vec![pk1.clone(), pk2.clone()]),
            &no_hints(),
        )
        .expect("all-archived is a skip, not an error");
        assert!(resolution.agents.is_empty());
        assert_eq!(
            resolution.skipped,
            vec![SkippedSlug {
                persona_id: "builtin:fizz".to_string(),
                reason: "all instances archived".to_string(),
            }]
        );
        assert_eq!(resolution.archived_excluded.len(), 2);
    }

    #[test]
    fn archive_filter_no_instances_ever_existed_is_skipped_with_different_reason() {
        // Zero live instances (nothing to archive) must not be confused
        // with "all instances archived" — no exclusions were made.
        let slugs = vec!["builtin:fizz".to_string()];
        let resolution =
            resolve_roster_with_archive_filter(&slugs, vec![], Ok(vec![]), &no_hints())
                .expect("zero instances is not fatal");
        assert!(resolution.agents.is_empty());
        assert_eq!(
            resolution.skipped,
            vec![SkippedSlug {
                persona_id: "builtin:fizz".to_string(),
                reason: "no live instances".to_string(),
            }]
        );
        assert!(resolution.archived_excluded.is_empty());
    }

    #[test]
    fn archive_filter_state3_on_success_path_proceeds_with_warning() {
        // Snapshot trust failure (state 3) fails open: resolution proceeds
        // exactly as if no archive filtering had happened, but the warning
        // is threaded into the report via archive_state_warning.
        let slugs = vec!["builtin:fizz".to_string()];
        let pk = "a".repeat(64);
        let found = vec![agent("builtin:fizz", &pk)];
        let archived_err = CliError::Other("relay info document missing 'self' field".into());
        let resolution =
            resolve_roster_with_archive_filter(&slugs, found, Err(archived_err), &no_hints())
                .expect("fails open — resolution still succeeds");
        assert_eq!(resolution.agents.len(), 1);
        assert_eq!(resolution.agents[0].pubkey, pk);
        assert!(resolution.archived_excluded.is_empty());
        let warning = resolution
            .archive_state_warning
            .expect("state-3 warning must be present on the success path");
        assert!(warning.contains("untrusted"));
    }

    #[test]
    fn archive_filter_state3_on_ambiguity_path_keeps_hard_error_with_warning() {
        // Snapshot trust failure alongside an unrelated multi-instance
        // ambiguity: the cardinality hard-error is unchanged (fail-open
        // never removes an error the non-filtered path would have hit),
        // but the trust warning rides along in the error detail — never a
        // fake success report on stdout.
        let slugs = vec!["builtin:fizz".to_string()];
        let found = vec![
            agent("builtin:fizz", &"a".repeat(64)),
            agent("builtin:fizz", &"b".repeat(64)),
        ];
        let archived_err = CliError::Other("query failure".into());
        let err = resolve_roster_with_archive_filter(&slugs, found, Err(archived_err), &no_hints())
            .expect_err("ambiguity error must still propagate");
        assert!(matches!(err, CliError::Usage(_)));
        let msg = err.to_string();
        assert!(
            msg.contains("builtin:fizz"),
            "error should name the slug: {msg}"
        );
        assert!(
            msg.contains("untrusted"),
            "error detail should carry the trust warning: {msg}"
        );
    }

    #[test]
    fn archive_filter_report_shape_includes_archived_excluded() {
        // A slug with no archiving at all still gets an (empty) complete
        // archived_excluded list in the resolution — the field is always
        // present, not conditionally omitted.
        let slugs = vec!["builtin:fizz".to_string()];
        let pk = "a".repeat(64);
        let found = vec![agent("builtin:fizz", &pk)];
        let resolution = resolve_roster_with_archive_filter(&slugs, found, Ok(vec![]), &no_hints())
            .expect("resolves with nothing archived");
        assert!(resolution.archived_excluded.is_empty());
        let serialized = serde_json::to_value(&resolution.archived_excluded).unwrap();
        assert_eq!(serialized, serde_json::json!([]));
    }

    // --- Observable-boundary regressions for the state-3 warning wiring ---
    //
    // The tests above exercise `resolve_roster_with_archive_filter` (the pure
    // core), which never touches stderr or the stdout report — deleting the
    // stderr emission at the `finalize_roster_resolution` call site, or the
    // `archive_state_warning` insertion in `build_template_report`, left all
    // of them green. These three go through the two production seams
    // directly so a regression at either wiring point fails a test.

    fn empty_report_inputs() -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
        (Vec::new(), Vec::new())
    }

    #[test]
    fn archive_filter_state3_success_warns_on_sink_and_in_report() {
        // State-3 fails open on the success path: the trust warning must
        // reach BOTH observable boundaries — the stderr sink at the point
        // of detection, and the top-level report field the caller prints.
        let slugs = vec!["builtin:fizz".to_string()];
        let pk = "a".repeat(64);
        let found = vec![agent("builtin:fizz", &pk)];
        let archived_err = CliError::Other("relay info document missing 'self' field".into());
        let mut sink: Vec<u8> = Vec::new();
        let resolution =
            finalize_roster_resolution(&slugs, found, Err(archived_err), &no_hints(), &mut sink)
                .expect("fails open — resolution still succeeds");

        let sink_text = String::from_utf8(sink).expect("sink is UTF-8");
        let lines: Vec<&str> = sink_text.lines().collect();
        assert_eq!(lines.len(), 1, "exactly one warning line: {sink_text:?}");
        let parsed: serde_json::Value =
            serde_json::from_str(lines[0]).expect("warning line is parseable JSON");
        let warning = parsed["warning"]
            .as_str()
            .expect("warning line has a string 'warning' field");
        assert!(warning.contains("untrusted"), "got: {warning}");

        let (members_added, member_failures) = empty_report_inputs();
        let report = build_template_report(
            "channel-1",
            "template-1",
            "ok",
            false,
            members_added,
            member_failures,
            &resolution,
        );
        assert_eq!(
            report["archive_state_warning"].as_str(),
            Some(warning),
            "report must carry the same warning text as the stderr sink: {report}"
        );
    }

    #[test]
    fn archive_filter_state3_ambiguity_warns_on_sink_and_in_error_detail() {
        // State-3 alongside an unrelated cardinality ambiguity: the warning
        // still reaches the stderr sink at detection time, and the hard
        // error's own detail carries it too. No report is constructible on
        // this path by construction — the function returns `Err` before
        // `cmd_create_channel_from_template` ever reaches `build_template_report`.
        let slugs = vec!["builtin:fizz".to_string()];
        let found = vec![
            agent("builtin:fizz", &"a".repeat(64)),
            agent("builtin:fizz", &"b".repeat(64)),
        ];
        let archived_err = CliError::Other("query failure".into());
        let mut sink: Vec<u8> = Vec::new();
        let err =
            finalize_roster_resolution(&slugs, found, Err(archived_err), &no_hints(), &mut sink)
                .expect_err("ambiguity error must still propagate");

        let sink_text = String::from_utf8(sink).expect("sink is UTF-8");
        assert_eq!(
            sink_text.lines().count(),
            1,
            "exactly one warning line: {sink_text:?}"
        );
        assert!(
            sink_text.contains("untrusted"),
            "stderr sink should carry the trust warning: {sink_text}"
        );
        assert!(matches!(err, CliError::Usage(_)));
        assert!(
            err.to_string().contains("untrusted"),
            "error detail should carry the trust warning: {err}"
        );
    }

    #[test]
    fn build_template_report_omits_warning_key_when_none() {
        // The insertion must not be unconditional — a trusted (states 1/2)
        // resolution's report has NO `archive_state_warning` key at all,
        // not merely a null one, so this can't be satisfied by always
        // inserting the field.
        let resolution = RosterResolution {
            agents: Vec::new(),
            skipped: Vec::new(),
            archived_excluded: Vec::new(),
            archive_state_warning: None,
        };
        let (members_added, member_failures) = empty_report_inputs();
        let report = build_template_report(
            "channel-1",
            "template-1",
            "ok",
            false,
            members_added,
            member_failures,
            &resolution,
        );
        assert!(
            report.get("archive_state_warning").is_none(),
            "no warning key expected: {report}"
        );
    }

    // --- Candidate hint formatting ---

    #[test]
    fn format_candidate_no_hint_returns_bare_pubkey() {
        let pk = "a".repeat(64);
        assert_eq!(format_candidate(&pk, None), pk);
    }

    #[test]
    fn format_candidate_presence_only_appends_status() {
        let pk = "a".repeat(64);
        let h = hint(Some("offline"), None);
        let formatted = format_candidate(&pk, Some(&h));
        assert!(formatted.contains(&pk), "pubkey must appear: {formatted}");
        assert!(
            formatted.contains("[offline]"),
            "presence status must appear: {formatted}"
        );
    }

    #[test]
    fn format_candidate_provisioned_at_only_appends_date() {
        let pk = "b".repeat(64);
        // 2024-01-15 = 1705276800 seconds since epoch
        let h = hint(None, Some(1_705_276_800));
        let formatted = format_candidate(&pk, Some(&h));
        assert!(formatted.contains(&pk), "pubkey must appear: {formatted}");
        assert!(
            formatted.contains("profile updated 2024-01-15"),
            "date must appear: {formatted}"
        );
    }

    #[test]
    fn format_candidate_both_hints_appends_both() {
        let pk = "c".repeat(64);
        let h = hint(Some("online"), Some(1_705_276_800));
        let formatted = format_candidate(&pk, Some(&h));
        assert!(formatted.contains(&pk), "pubkey must appear: {formatted}");
        assert!(
            formatted.contains("online"),
            "presence must appear: {formatted}"
        );
        assert!(
            formatted.contains("profile updated 2024-01-15"),
            "date must appear: {formatted}"
        );
    }

    #[test]
    fn format_candidate_empty_hint_fields_returns_bare_pubkey() {
        // Both hint fields None — same output as no hint at all.
        let pk = "d".repeat(64);
        let h = hint(None, None);
        assert_eq!(format_candidate(&pk, Some(&h)), pk);
    }

    #[test]
    fn cardinality_error_includes_hint_when_provided() {
        // When hints are present, the duplicate-instance error must include
        // the presence and provisioned-at decoration in its candidate list.
        let pk_a = "a".repeat(64);
        let pk_b = "b".repeat(64);
        let slugs = vec!["builtin:fizz".to_string()];
        let found = vec![agent("builtin:fizz", &pk_a), agent("builtin:fizz", &pk_b)];
        let mut hints = HashMap::new();
        hints.insert(pk_a.clone(), hint(Some("offline"), Some(1_705_276_800)));
        hints.insert(pk_b.clone(), hint(Some("online"), None));

        let err = apply_cardinality_rule(&slugs, &found, &hints).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(&pk_a), "pk_a must appear: {msg}");
        assert!(msg.contains(&pk_b), "pk_b must appear: {msg}");
        assert!(msg.contains("offline"), "offline status must appear: {msg}");
        assert!(
            msg.contains("profile updated 2024-01-15"),
            "provisioned date must appear: {msg}"
        );
        assert!(msg.contains("online"), "online status must appear: {msg}");
    }

    #[test]
    fn cardinality_error_falls_back_to_bare_pubkey_when_hint_missing() {
        // A missing hint entry in the map must not cause a panic or omit
        // the pubkey from the error — it must print as a bare pubkey.
        let pk_a = "a".repeat(64);
        let pk_b = "b".repeat(64);
        let slugs = vec!["builtin:fizz".to_string()];
        let found = vec![agent("builtin:fizz", &pk_a), agent("builtin:fizz", &pk_b)];
        // Only pk_a has a hint; pk_b is absent from the map.
        let mut hints = HashMap::new();
        hints.insert(pk_a.clone(), hint(Some("offline"), None));

        let err = apply_cardinality_rule(&slugs, &found, &hints).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(&pk_a), "pk_a must appear: {msg}");
        assert!(
            msg.contains(&pk_b),
            "pk_b must appear as bare pubkey: {msg}"
        );
        // pk_b has no hint — it must not appear as "[online]" or "[offline]"
        // but must still appear in the candidate list.
        assert!(
            !msg.contains(&format!("{pk_b} [")),
            "pk_b must not have hint brackets: {msg}"
        );
    }

    // --- build_hint_map boundary tests ---

    #[test]
    fn build_hint_map_uses_p_tag_over_author_for_presence() {
        // Relay signs presence events with its own key; the agent pubkey is in
        // the `p` tag. The relay author must NOT be used as the map key.
        let relay_pk = "r".repeat(64);
        let agent_pk = "a".repeat(64);
        let presence = vec![json!({
            "pubkey": relay_pk,
            "content": "online",
            "tags": [["p", agent_pk]],
        })];
        let map = build_hint_map(&[], &presence, &[]);
        assert!(
            !map.contains_key(&relay_pk),
            "relay author must not be the key: {map:?}"
        );
        assert!(
            map.contains_key(&agent_pk),
            "agent p-tag must be key: {map:?}"
        );
        assert_eq!(
            map[&agent_pk].presence.as_deref(),
            Some("online"),
            "presence status preserved"
        );
    }

    #[test]
    fn build_hint_map_presence_failure_profile_survives() {
        // If presence lookup fails (empty slice), profile hints must still be
        // populated from the profile events alone.
        let pk = "b".repeat(64);
        let profile = vec![json!({
            "pubkey": pk,
            "created_at": 1_705_276_800_u64,
        })];
        let map = build_hint_map(&[], &[], &profile);
        assert!(map.contains_key(&pk), "pubkey must be in map: {map:?}");
        assert_eq!(
            map[&pk].profile_updated_at,
            Some(1_705_276_800),
            "profile timestamp preserved"
        );
        assert!(
            map[&pk].presence.is_none(),
            "presence must be absent when lookup failed"
        );
    }

    #[test]
    fn build_hint_map_profile_failure_presence_survives() {
        // If profile lookup fails (empty slice), presence hints must still be
        // populated from the presence events alone.
        let pk = "c".repeat(64);
        let presence = vec![json!({
            "pubkey": pk,
            "content": "offline",
            "tags": [],
        })];
        let map = build_hint_map(&[], &presence, &[]);
        assert!(map.contains_key(&pk), "pubkey must be in map: {map:?}");
        assert_eq!(
            map[&pk].presence.as_deref(),
            Some("offline"),
            "presence status preserved"
        );
        assert!(
            map[&pk].profile_updated_at.is_none(),
            "profile_updated_at must be absent when lookup failed"
        );
    }

    #[test]
    fn build_hint_map_malformed_entries_are_skipped() {
        // Presence events missing both pubkey and p-tag are skipped without
        // panicking; profile events missing pubkey are skipped too.
        let malformed_presence = vec![
            json!({"content": "online"}), // no pubkey, no p-tag
            json!({"pubkey": null, "content": "online", "tags": []}),
        ];
        let malformed_profile = vec![
            json!({"created_at": 1_705_276_800_u64}), // no pubkey
            json!({"pubkey": null, "created_at": 1_705_276_800_u64}),
        ];
        let map = build_hint_map(&[], &malformed_presence, &malformed_profile);
        assert!(
            map.is_empty(),
            "malformed entries must yield empty map: {map:?}"
        );
    }

    #[test]
    fn build_hint_map_both_failures_yield_empty_map() {
        // Both slices empty simulates a total timeout / relay error.
        let map = build_hint_map(&[], &[], &[]);
        assert!(map.is_empty(), "empty inputs must yield empty map");
    }

    // --- assemble_roster_resolution wiring tests ---
    // These tests exercise the conditional-fetch logic directly, proving:
    // (a) the fetcher is called only when duplicate live instances exist, and
    // (b) the exact pubkey set passed to the fetcher matches the live duplicates.
    // Using a recording closure instead of a real relay means these run
    // synchronously fast and catch the wiring even without a relay.

    /// Helper: make a `ResolvedAgent` with the given persona and pubkey.
    fn owned_agent(persona_id: &str, pubkey: &str) -> ResolvedAgent {
        ResolvedAgent {
            persona_id: persona_id.to_string(),
            pubkey: pubkey.to_string(),
        }
    }

    #[tokio::test]
    async fn assemble_roster_resolution_duplicate_pair_invokes_fetcher_with_their_pubkeys() {
        // Two live instances for the same slug — fetcher must be called with
        // exactly those two pubkeys.
        let pk_a = "a".repeat(64);
        let pk_b = "b".repeat(64);
        let slugs = vec!["sietch:agent".to_string()];
        let found = vec![
            owned_agent("sietch:agent", &pk_a),
            owned_agent("sietch:agent", &pk_b),
        ];

        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let fetcher_invoked = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&fetcher_invoked);
        let result = assemble_roster_resolution(
            &slugs,
            found,
            Ok(vec![]), // trusted empty archive: both are live
            |pks| async move {
                flag.store(true, Ordering::Relaxed);
                // Verify the fetcher receives exactly the duplicate pubkeys.
                let mut sorted = pks.clone();
                sorted.sort();
                assert_eq!(sorted.len(), 2, "exactly 2 duplicate pubkeys expected");
                HashMap::new()
            },
            &mut std::io::sink(),
        )
        .await;

        assert!(
            fetcher_invoked.load(Ordering::Relaxed),
            "fetcher must be called for a duplicate pair"
        );
        // Both pubkeys appear in the cardinality error (bare, since the fetcher returned empty).
        let err = result.unwrap_err().to_string();
        assert!(err.contains(&pk_a), "pk_a must appear in error: {err}");
        assert!(err.contains(&pk_b), "pk_b must appear in error: {err}");
    }

    #[tokio::test]
    async fn assemble_roster_resolution_single_instance_never_invokes_fetcher() {
        // All slugs have exactly one live instance — fetcher must NOT be called.
        // If it is called, the `panic!` fires.
        let pk = "c".repeat(64);
        let slugs = vec!["sietch:agent".to_string()];
        let found = vec![owned_agent("sietch:agent", &pk)];

        let result = assemble_roster_resolution(
            &slugs,
            found,
            Ok(vec![]),
            |_pks| async move {
                panic!("fetcher must not be called on a single-instance roster");
                #[allow(unreachable_code)]
                HashMap::<String, CandidateHint>::new()
            },
            &mut std::io::sink(),
        )
        .await;

        assert!(
            result.is_ok(),
            "single instance resolves cleanly: {result:?}"
        );
    }

    #[tokio::test]
    async fn assemble_roster_resolution_trusted_archive_removes_duplicate_suppresses_fetcher() {
        // pk_a is archived. Only pk_b remains live — no duplicate, so the
        // fetcher must NOT be called.
        let pk_a = "d".repeat(64);
        let pk_b = "e".repeat(64);
        let slugs = vec!["sietch:agent".to_string()];
        let found = vec![
            owned_agent("sietch:agent", &pk_a),
            owned_agent("sietch:agent", &pk_b),
        ];

        let result = assemble_roster_resolution(
            &slugs,
            found,
            Ok(vec![pk_a.clone()]), // pk_a archived
            |_pks| async move {
                panic!("fetcher must not be called when archive resolves the duplicate");
                #[allow(unreachable_code)]
                HashMap::<String, CandidateHint>::new()
            },
            &mut std::io::sink(),
        )
        .await;

        assert!(
            result.is_ok(),
            "archive resolves duplicate cleanly: {result:?}"
        );
    }

    #[tokio::test]
    async fn assemble_roster_resolution_untrusted_archive_invokes_fetcher_conservatively() {
        // Archive snapshot is Err (untrusted). Both instances are treated as
        // live conservatively → fetcher must be called.
        let pk_a = "f".repeat(64);
        let pk_b = "g".repeat(64);
        let slugs = vec!["sietch:agent".to_string()];
        let found = vec![
            owned_agent("sietch:agent", &pk_a),
            owned_agent("sietch:agent", &pk_b),
        ];

        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let fetcher_invoked = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&fetcher_invoked);
        let result = assemble_roster_resolution(
            &slugs,
            found,
            Err(CliError::Other("snapshot unavailable".to_string())),
            |pks| async move {
                flag.store(true, Ordering::Relaxed);
                let _ = pks;
                HashMap::<String, CandidateHint>::new()
            },
            &mut std::io::sink(),
        )
        .await;

        assert!(
            fetcher_invoked.load(Ordering::Relaxed),
            "fetcher must be called under untrusted archive"
        );
        // Error still surfaces (bare pubkeys, plus the archive warning embedded).
        assert!(
            result.is_err(),
            "untrusted archive + duplicates is still an error"
        );
    }

    // --- hints_from_results offline-seeding boundary ---
    // A successful presence snapshot is complete: the relay drops the Redis
    // presence key on offline, so a requested pubkey the snapshot omits is
    // offline. A failed/malformed presence response must NOT infer offline.

    /// Serialize presence/profile events the way the relay returns them.
    fn events_json(events: &[serde_json::Value]) -> String {
        serde_json::to_string(events).unwrap()
    }

    /// Build a relay-shaped presence snapshot event: a real signed
    /// `nostr::Event` of kind `KIND_PRESENCE_UPDATE` whose `p` tag names
    /// `subject`, matching exactly what `synthesize_presence` produces. Signed
    /// by an arbitrary "relay" key so its author differs from the subject.
    fn presence_event(subject: &str, status: &str) -> serde_json::Value {
        let relay_keys =
            nostr::Keys::parse("0000000000000000000000000000000000000000000000000000000000000002")
                .expect("valid relay test key");
        let event = nostr::EventBuilder::new(
            nostr::Kind::Custom(buzz_core::kind::KIND_PRESENCE_UPDATE as u16),
            status,
        )
        .tags([nostr::Tag::parse(["p", subject]).expect("valid p tag")])
        .sign_with_keys(&relay_keys)
        .expect("signing presence event");
        serde_json::to_value(&event).expect("event to json")
    }

    /// Build a presence event carrying the given `p`-tag subjects in order,
    /// signed by a relay key. Used to construct off-contract multi-`p`-tag
    /// events the relay never emits but a hostile responder could.
    fn presence_event_with_p_tags(subjects: &[&str], status: &str) -> serde_json::Value {
        let relay_keys =
            nostr::Keys::parse("0000000000000000000000000000000000000000000000000000000000000002")
                .expect("valid relay test key");
        let tags: Vec<nostr::Tag> = subjects
            .iter()
            .map(|s| nostr::Tag::parse(["p", s]).expect("valid p tag"))
            .collect();
        let event = nostr::EventBuilder::new(
            nostr::Kind::Custom(buzz_core::kind::KIND_PRESENCE_UPDATE as u16),
            status,
        )
        .tags(tags)
        .sign_with_keys(&relay_keys)
        .expect("signing presence event");
        serde_json::to_value(&event).expect("event to json")
    }

    #[test]
    fn hints_from_results_successful_partial_snapshot_seeds_absent_as_offline() {
        let online_pk = "a".repeat(64);
        let absent_pk = "b".repeat(64);
        let pubkeys = vec![online_pk.clone(), absent_pk.clone()];
        // Snapshot returns only the online instance; absent_pk is omitted.
        let presence = events_json(&[presence_event(&online_pk, "online")]);

        let map = hints_from_results(&pubkeys, Ok(presence), Ok("[]".to_string()));

        assert_eq!(
            map[&online_pk].presence.as_deref(),
            Some("online"),
            "returned status must overlay the seed"
        );
        assert_eq!(
            map[&absent_pk].presence.as_deref(),
            Some("offline"),
            "a requested pubkey omitted from a successful snapshot is offline"
        );
    }

    #[test]
    fn hints_from_results_successful_empty_snapshot_seeds_all_offline() {
        let pk_a = "c".repeat(64);
        let pk_b = "d".repeat(64);
        let pubkeys = vec![pk_a.clone(), pk_b.clone()];

        // Empty-but-successful snapshot: every requested pubkey is offline.
        let map = hints_from_results(&pubkeys, Ok("[]".to_string()), Ok("[]".to_string()));

        assert_eq!(map[&pk_a].presence.as_deref(), Some("offline"));
        assert_eq!(map[&pk_b].presence.as_deref(), Some("offline"));
    }

    #[test]
    fn hints_from_results_failed_presence_yields_no_offline_label() {
        let pk = "e".repeat(64);
        let pubkeys = vec![pk.clone()];
        let profile =
            events_json(&[json!({ "pubkey": pk.clone(), "created_at": 1_705_276_800_u64 })]);

        let map = hints_from_results(
            &pubkeys,
            Err(CliError::Other("presence hint timeout".to_string())),
            Ok(profile),
        );

        assert!(
            map[&pk].presence.is_none(),
            "a failed presence lookup must never be inferred as offline"
        );
        assert_eq!(
            map[&pk].profile_updated_at,
            Some(1_705_276_800),
            "the completed profile lookup must survive the failed presence sibling"
        );
    }

    #[test]
    fn hints_from_results_malformed_presence_yields_no_offline_label() {
        let pk = "f".repeat(64);
        let pubkeys = vec![pk.clone()];

        // Unparseable presence body → not a trusted snapshot → no seeding.
        let map = hints_from_results(
            &pubkeys,
            Ok("not json".to_string()),
            Err(CliError::Other("profile hint timeout".to_string())),
        );

        assert!(
            map.get(&pk).is_none_or(|h| h.presence.is_none()),
            "malformed presence must not infer offline: {map:?}"
        );
    }

    #[test]
    fn hints_from_results_malformed_element_makes_snapshot_untrusted() {
        // A body that parses as an array but contains a malformed element
        // (`{}`, `null`, or a contentless event) is NOT an authoritative
        // snapshot: it must seed nothing, while the profile sibling survives.
        let requested = "a".repeat(64);
        let subject = "b".repeat(64);
        let pubkeys = vec![requested.clone(), subject.clone()];
        let profile =
            events_json(&[json!({ "pubkey": requested.clone(), "created_at": 1_705_276_800_u64 })]);

        for bad_body in [
            "[{}]".to_string(),
            "[null]".to_string(),
            // A well-formed subject but non-string (unreadable) content.
            events_json(&[json!({
                "pubkey": "r".repeat(64),
                "content": 42,
                "tags": [["p", subject.clone()]],
            })]),
        ] {
            let map = hints_from_results(&pubkeys, Ok(bad_body.clone()), Ok(profile.clone()));

            assert!(
                map.values().all(|h| h.presence.is_none()),
                "malformed element {bad_body} must yield no presence labels: {map:?}"
            );
            assert_eq!(
                map[&requested].profile_updated_at,
                Some(1_705_276_800),
                "the completed profile sibling must still contribute hints: {map:?}"
            );
        }
    }

    #[test]
    fn hints_from_results_relay_error_response_seeds_nothing() {
        // The relay surfaces a Redis-outage presence lookup as a non-2xx error,
        // which the CLI query returns as `Err`. That must seed nothing — a
        // backend failure is not an authoritative all-offline snapshot.
        let pk = "c".repeat(64);
        let pubkeys = vec![pk.clone()];

        let map = hints_from_results(
            &pubkeys,
            Err(CliError::Other("presence lookup: redis down".to_string())),
            Ok("[]".to_string()),
        );

        assert!(
            map.get(&pk).is_none_or(|h| h.presence.is_none()),
            "a relay-side presence failure must never be inferred as offline: {map:?}"
        );
    }

    #[test]
    fn hints_from_results_vacuous_object_makes_snapshot_untrusted() {
        // A syntactically-valid array whose element carries a plausible subject
        // and string content but is NOT a complete signed event (no id/sig/kind
        // /created_at) must not be trusted as a snapshot — otherwise it would
        // re-seed every requested candidate `offline` from an unverifiable body.
        let requested = "a".repeat(64);
        let pubkeys = vec![requested.clone()];
        let profile =
            events_json(&[json!({ "pubkey": requested.clone(), "created_at": 1_705_276_800_u64 })]);
        let vacuous = events_json(&[json!({ "pubkey": requested.clone(), "content": "online" })]);

        let map = hints_from_results(&pubkeys, Ok(vacuous), Ok(profile));

        assert!(
            map.values().all(|h| h.presence.is_none()),
            "a vacuous non-event object must yield no presence labels: {map:?}"
        );
        assert_eq!(
            map[&requested].profile_updated_at,
            Some(1_705_276_800),
            "the completed profile sibling must still contribute hints: {map:?}"
        );
    }

    #[test]
    fn hints_from_results_unrequested_subject_makes_snapshot_untrusted() {
        // A fully-shaped, correctly-signed presence event whose subject is NOT
        // one of the requested pubkeys is not a snapshot of the requested set;
        // trusting it would seed the requested duplicates `offline` from an
        // answer about someone else entirely.
        let requested = "a".repeat(64);
        let other = "b".repeat(64);
        let pubkeys = vec![requested.clone()];
        let profile =
            events_json(&[json!({ "pubkey": requested.clone(), "created_at": 1_705_276_800_u64 })]);
        let presence = events_json(&[presence_event(&other, "online")]);

        let map = hints_from_results(&pubkeys, Ok(presence), Ok(profile));

        assert!(
            map.values().all(|h| h.presence.is_none()),
            "an event for an unrequested subject must yield no presence labels: {map:?}"
        );
        assert_eq!(
            map[&requested].profile_updated_at,
            Some(1_705_276_800),
            "the completed profile sibling must still contribute hints: {map:?}"
        );
    }

    #[test]
    fn hints_from_results_mixed_p_tags_makes_snapshot_untrusted() {
        // The relay emits exactly one `p` tag per presence event. A hostile
        // responder could return `[["p","<unrequested>"],["p","<requested>"]]`:
        // a weaker "any requested `p` tag" gate would accept it, but the
        // consumer reads the FIRST `p` tag (the unrequested subject) — so it
        // would overlay the wrong subject and leave the requested candidate
        // falsely seeded `offline`. Requiring exactly one `p`-tag subject that
        // is requested rejects both an unrequested-first ordering and any event
        // carrying more than one `p` tag.
        let requested = "a".repeat(64);
        let unrequested = "b".repeat(64);
        let pubkeys = vec![requested.clone()];
        let profile =
            events_json(&[json!({ "pubkey": requested.clone(), "created_at": 1_705_276_800_u64 })]);

        // Case 1: an unrequested `p` tag before a requested one.
        let mixed = events_json(&[presence_event_with_p_tags(
            &[&unrequested, &requested],
            "online",
        )]);
        // Case 2: a valid requested `p` tag plus a second (also requested) —
        // still off-contract: more than one `p` tag.
        let two_requested = events_json(&[presence_event_with_p_tags(
            &[&requested, &requested],
            "online",
        )]);

        for body in [mixed, two_requested] {
            let map = hints_from_results(&pubkeys, Ok(body.clone()), Ok(profile.clone()));

            assert!(
                map.values().all(|h| h.presence.is_none()),
                "a multi-`p`-tag event must yield no presence labels: {map:?}"
            );
            assert_eq!(
                map[&requested].profile_updated_at,
                Some(1_705_276_800),
                "the completed profile sibling must still contribute hints: {map:?}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn join_bounded_queries_completed_presence_survives_hung_profile() {
        let timeout = std::time::Duration::from_secs(3);
        let (presence, profile) = join_bounded_queries(
            timeout,
            // Presence completes immediately.
            async { Ok::<String, CliError>("[]".to_string()) },
            // Profile hangs past the timeout.
            async {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                Ok::<String, CliError>("[]".to_string())
            },
        )
        .await;

        assert!(
            presence.is_ok(),
            "the completed presence lookup must be retained, not discarded by the hung sibling"
        );
        assert!(profile.is_err(), "the hung profile lookup must time out");
    }

    #[tokio::test(start_paused = true)]
    async fn join_bounded_queries_completed_profile_survives_hung_presence() {
        let timeout = std::time::Duration::from_secs(3);
        let (presence, profile) = join_bounded_queries(
            timeout,
            async {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                Ok::<String, CliError>("[]".to_string())
            },
            async { Ok::<String, CliError>("[]".to_string()) },
        )
        .await;

        assert!(presence.is_err(), "the hung presence lookup must time out");
        assert!(
            profile.is_ok(),
            "the completed profile lookup must be retained despite the hung presence sibling"
        );
    }

    /// Production-wiring seam: drive `fetch_candidate_hints` itself against a
    /// controlled `/query` server where the presence query completes and the
    /// profile query hangs past the timeout. The completed presence hint (an
    /// `online` overlay plus `offline` seeds for the requested pubkeys) must
    /// survive. This is what protects the `fetch_candidate_hints` call site: if
    /// the old shared `timeout(join!(...))` is restored, the hung profile query
    /// discards the completed presence result and the map comes back empty.
    #[tokio::test]
    async fn fetch_candidate_hints_completed_presence_survives_hung_profile_query() {
        use axum::{extract::State, routing::post, Router};
        use serde_json::Value;
        use std::net::SocketAddr;
        use tokio::net::TcpListener;

        let online_pk = "a".repeat(64);
        let offline_pk = "b".repeat(64);

        // Server dispatches on filter kind: presence (40902) returns one online
        // event immediately; profile (kind 0) hangs well past the timeout.
        let online_for_server = online_pk.clone();
        let app = Router::new()
            .route(
                "/query",
                post(move |State(()): State<()>, body: axum::body::Bytes| {
                    let online_pk = online_for_server.clone();
                    async move {
                        let filters: Vec<Value> = serde_json::from_slice(&body).unwrap_or_default();
                        let kind = filters
                            .first()
                            .and_then(|f| f.get("kinds"))
                            .and_then(|k| k.as_array())
                            .and_then(|k| k.first())
                            .and_then(Value::as_u64);
                        if kind == Some(0) {
                            // Profile query hangs past the 100ms test timeout.
                            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                        }
                        let body =
                            serde_json::to_string(&vec![presence_event(&online_pk, "online")])
                                .unwrap();
                        axum::response::Response::builder()
                            .header("content-type", "application/json")
                            .body(axum::body::Body::from(body))
                            .unwrap()
                    }
                }),
            )
            .with_state(());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let keys =
            nostr::Keys::parse("0000000000000000000000000000000000000000000000000000000000000001")
                .expect("valid test key");
        let client = BuzzClient::new(format!("http://{addr}"), keys, None, None)
            .expect("client construction should not fail");

        let map = fetch_candidate_hints(
            &client,
            &[online_pk.clone(), offline_pk.clone()],
            std::time::Duration::from_millis(100),
        )
        .await;

        // Presence completed: online overlay present, absent pubkey seeded offline.
        assert_eq!(
            map[&online_pk].presence.as_deref(),
            Some("online"),
            "the completed presence result must survive the hung profile query: {map:?}"
        );
        assert_eq!(
            map[&offline_pk].presence.as_deref(),
            Some("offline"),
            "the trusted snapshot must seed the absent candidate offline: {map:?}"
        );
        // Profile hung → no profile timestamps.
        assert!(
            map.values().all(|h| h.profile_updated_at.is_none()),
            "the hung profile query must contribute nothing: {map:?}"
        );
    }
}

/// Command-level coverage for `cmd_set_canvas`'s writer discipline: it must read
/// the head, stamp `created_at` strictly ahead of a future-dated head, emit NO
/// `expected-revision` tag (unconditional replace), fall back to stamping at `now`
/// when no head exists, and refuse to submit against a structurally poisoned head.
/// These pin the fix so a revert to `created_at = now` / no head read fails a test
/// rather than silently recreating the false-success bug.
///
/// Each test spins up a local axum relay that answers `POST /query` with a fixed
/// head and captures the event submitted to `POST /events`, then inspects the
/// signed event — behaviour at the command seam, not implementation details.
#[cfg(test)]
mod set_canvas_tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Bytes;
    use axum::extract::State;
    use axum::routing::post;
    use axum::Router;
    use nostr::Keys;
    use serde_json::{json, Value};
    use tokio::net::TcpListener;

    use super::cmd_set_canvas;
    use crate::client::BuzzClient;

    const CHANNEL: &str = "326d56bc-c96c-4af0-86a1-5e804cd1b467";

    #[derive(Clone)]
    struct RelayState {
        query_response: Arc<String>,
        submitted: Arc<Mutex<Option<Value>>>,
        /// All filter arrays sent to POST /query, in call order.
        query_bodies: Arc<Mutex<Vec<Vec<Value>>>>,
    }

    /// Spawn a relay that returns `query_response` from `POST /query` and records
    /// the event posted to `POST /events`. `submitted` stays `None` until (and
    /// unless) `set` actually publishes.
    ///
    /// Returns `(url, submitted_event, query_bodies)`.
    async fn relay(
        query_response: &str,
    ) -> (
        String,
        Arc<Mutex<Option<Value>>>,
        Arc<Mutex<Vec<Vec<Value>>>>,
    ) {
        let submitted: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let query_bodies: Arc<Mutex<Vec<Vec<Value>>>> = Arc::new(Mutex::new(Vec::new()));
        let state = RelayState {
            query_response: Arc::new(query_response.to_string()),
            submitted: submitted.clone(),
            query_bodies: query_bodies.clone(),
        };
        let app = Router::new()
            .route(
                "/query",
                post(|State(s): State<RelayState>, body: Bytes| async move {
                    let filters: Vec<Value> = serde_json::from_slice(&body).unwrap_or_default();
                    s.query_bodies.lock().unwrap().push(filters);
                    (*s.query_response).clone()
                }),
            )
            .route(
                "/events",
                post(|State(s): State<RelayState>, body: Bytes| async move {
                    let event: Value = serde_json::from_slice(&body).expect("event json");
                    *s.submitted.lock().unwrap() = Some(event);
                    r#"{"event_id":"deadbeef","accepted":true,"message":""}"#.to_string()
                }),
            )
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), submitted, query_bodies)
    }

    /// Return all filter objects from all /query calls flattened into one vec.
    fn all_filters(query_bodies: &Arc<Mutex<Vec<Vec<Value>>>>) -> Vec<Value> {
        query_bodies
            .lock()
            .unwrap()
            .iter()
            .flat_map(|batch| batch.clone())
            .collect()
    }

    fn client(base_url: &str) -> BuzzClient {
        BuzzClient::new(base_url.to_string(), Keys::generate(), None, None).unwrap()
    }

    fn tag_value(event: &Value, key: &str) -> Option<String> {
        event
            .get("tags")?
            .as_array()?
            .iter()
            .find(|t| {
                t.as_array()
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    == Some(key)
            })
            .and_then(|t| t.as_array())
            .and_then(|a| a.get(1))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    /// A future-dated head (within the skew ceiling) forces the
    /// `max(now, head + 1)` bump to resolve to `head + 1` deterministically (no
    /// clock dependence, no sleeps), so the submitted event sorts strictly
    /// ahead of the head under `created_at DESC, id ASC`. `set` is unconditional:
    /// no `expected-revision` tag is emitted even though the head was read.
    #[tokio::test]
    async fn set_stamps_ahead_of_future_head_and_is_untagged() {
        let head_id = "a".repeat(64);
        // A head ahead of `now` but inside the 60-second ceiling: `head + 1`
        // deterministically wins the `max`, and the guard accepts it. Computed
        // from `now` so it tracks the wall clock without a hardcoded date.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let future = now + 30;
        let head = json!([{
            "id": head_id,
            "pubkey": "b".repeat(64),
            "kind": 40100,
            "content": "old",
            "created_at": future,
            "tags": [["h", CHANNEL]],
        }]);
        let (url, submitted, _) = relay(&head.to_string()).await;

        cmd_set_canvas(&client(&url), CHANNEL, "new content")
            .await
            .expect("set succeeds");

        let event = submitted.lock().unwrap().clone().expect("set must submit");
        assert_eq!(
            event.get("created_at").and_then(|v| v.as_u64()),
            Some(future + 1),
            "must stamp strictly ahead of the future-dated head"
        );
        // `set` is unconditional — no expected-revision tag regardless of head presence.
        assert_eq!(
            tag_value(&event, "expected-revision"),
            None,
            "set must NOT emit expected-revision — it is an unconditional replace"
        );
        assert_eq!(event.get("kind").and_then(|v| v.as_u64()), Some(40100));
    }

    /// `set` inherits the SDK skew guard via `build_set_canvas_after_head`: a
    /// head timestamped far beyond the future ceiling is refused with a clear
    /// error and nothing is published, rather than extending a poisoned timeline.
    #[tokio::test]
    async fn set_refuses_a_poisoned_future_head() {
        let head = json!([{
            "id": "a".repeat(64),
            "pubkey": "b".repeat(64),
            "kind": 40100,
            "content": "old",
            "created_at": 4_102_444_800u64, // 2100-01-01Z — far past the ceiling
            "tags": [["h", CHANNEL]],
        }]);
        let (url, submitted, _) = relay(&head.to_string()).await;

        let err = cmd_set_canvas(&client(&url), CHANNEL, "new content")
            .await
            .expect_err("a poisoned future head must be refused");

        assert!(
            err.to_string().contains("too far in the future"),
            "unexpected message: {err}"
        );
        assert!(
            submitted.lock().unwrap().is_none(),
            "no event may be published against a poisoned head"
        );
    }

    /// No head yet: `set` still submits (create is not an error) and does NOT
    /// emit an `expected-revision` tag — `set` is unconditional regardless of
    /// head presence.
    #[tokio::test]
    async fn set_with_no_head_creates_without_expected_revision() {
        let (url, submitted, _) = relay("[]").await;

        cmd_set_canvas(&client(&url), CHANNEL, "first content")
            .await
            .expect("set on empty channel succeeds");

        let event = submitted.lock().unwrap().clone().expect("set must submit");
        assert_eq!(
            tag_value(&event, "expected-revision"),
            None,
            "set must NOT emit expected-revision even on first create"
        );
    }

    /// `set` sends `"consistency": "strong"` on its head-read (write-influencing)
    /// but NOT on other queries. This is the request-filter side of the writer-pin
    /// contract: removing the injection from `fetch_canvas_head(writer_pinned=true)`
    /// must turn this test red.
    ///
    /// Mutation oracle: if `filter["consistency"] = …` is deleted from the
    /// `writer_pinned` branch of `fetch_canvas_head`, the head-read filter no
    /// longer carries `"consistency"`, and the assertion below fails.
    #[tokio::test]
    async fn set_head_read_carries_strong_consistency() {
        let head_id = "a".repeat(64);
        let head = json!([{
            "id": head_id,
            "pubkey": "b".repeat(64),
            "kind": 40100,
            "content": "old",
            "created_at": 1_700_000_000u64,
            "tags": [["h", CHANNEL]],
        }]);
        let (url, _, query_bodies) = relay(&head.to_string()).await;
        cmd_set_canvas(&client(&url), CHANNEL, "new content")
            .await
            .expect("set succeeds");

        let filters = all_filters(&query_bodies);
        // The head read is a non-`ids` filter for kind 40100 with limit 1.
        let head_reads: Vec<_> = filters
            .iter()
            .filter(|f| {
                f.get("ids").is_none()
                    && f.get("kinds")
                        .and_then(|k| k.as_array())
                        .map(|a| a.iter().any(|k| k == 40100))
                        .unwrap_or(false)
                    && f.get("limit").and_then(|v| v.as_u64()) == Some(1)
            })
            .collect();
        assert!(
            !head_reads.is_empty(),
            "set must issue at least one head-read filter"
        );
        for f in &head_reads {
            assert_eq!(
                f.get("consistency").and_then(|v| v.as_str()),
                Some("strong"),
                "set head-read must carry consistency=strong: {f}"
            );
        }
    }

    /// `get` (display path) does NOT send `"consistency"` — it stays
    /// replica-eligible. This is the inverse of the writer-pin contract: a
    /// display read that silently acquired the pin would unnecessarily load
    /// the writer pool.
    ///
    /// Mutation oracle: if `fetch_canvas_head(writer_pinned=false)` were changed
    /// to always inject `consistency`, the assertion below would fail.
    #[tokio::test]
    async fn get_head_read_does_not_carry_consistency() {
        use super::cmd_get_canvas;

        let head_id = "a".repeat(64);
        let head = json!([{
            "id": head_id,
            "pubkey": "b".repeat(64),
            "kind": 40100,
            "content": "canvas content",
            "created_at": 1_700_000_000u64,
            "tags": [["h", CHANNEL]],
        }]);
        let (url, _, query_bodies) = relay(&head.to_string()).await;
        cmd_get_canvas(&client(&url), CHANNEL, None)
            .await
            .expect("get succeeds");

        let filters = all_filters(&query_bodies);
        let head_reads: Vec<_> = filters
            .iter()
            .filter(|f| {
                f.get("ids").is_none()
                    && f.get("kinds")
                        .and_then(|k| k.as_array())
                        .map(|a| a.iter().any(|k| k == 40100))
                        .unwrap_or(false)
                    && f.get("limit").and_then(|v| v.as_u64()) == Some(1)
            })
            .collect();
        assert!(!head_reads.is_empty(), "get must issue a head-read filter");
        for f in &head_reads {
            assert!(
                f.get("consistency").is_none(),
                "display head-read must NOT carry consistency: {f}"
            );
        }
    }
}

/// Command-level coverage for `cmd_restore_canvas`'s post-write supersession
/// check. After publishing the restored revision, the command re-reads the head
/// once and classifies the outcome via the SDK's `canvas_write_survived`
/// predicate. These pin all three post-write outcomes deterministically —
/// controlled relay responses, no timing, no sleeps — so a revert of the
/// verification hunk fails a test rather than silently reporting hollow success.
///
/// The relay dispatches by filter shape: an `ids` query returns the target
/// revision's content; the first plain head query returns the pre-write head,
/// and the post-write head query is synthesized from the captured submission per
/// a per-test [`PostHead`] mode — so the head can echo our own event id (which
/// we cannot predict before signing under a random key).
#[cfg(test)]
mod restore_canvas_tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Bytes;
    use axum::extract::State;
    use axum::routing::post;
    use axum::Router;
    use nostr::Keys;
    use serde_json::{json, Value};
    use tokio::net::TcpListener;

    use super::cmd_restore_canvas;
    use crate::client::BuzzClient;
    use crate::CliError;

    const CHANNEL: &str = "326d56bc-c96c-4af0-86a1-5e804cd1b467";
    const REVISION: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const HEAD: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const STRANGER: &str = "3333333333333333333333333333333333333333333333333333333333333333";
    const THIRD: &str = "4444444444444444444444444444444444444444444444444444444444444444";

    /// How the relay synthesizes the post-write head from the submitted event.
    #[derive(Clone, Copy)]
    enum PostHead {
        /// The head is our own submitted event → survived.
        OurEvent,
        /// The head is a stranger whose `expected-revision` names our submitted
        /// event → a later write legitimately built on us → survived.
        StrangerBuildsOnUs,
        /// The head is a stranger two links above ours: C(exp=B)→B(exp=ours)→
        /// ours, all present in the stream → survived by ancestry walk.
        StrangerBuildsOnUsTransitively,
        /// The head is an unrelated stranger → our restore was superseded.
        Stranger,
        /// The post-write verification read fails (HTTP 500). The submit was
        /// accepted, so the restore must still report success (exit 0).
        ReadFails,
    }

    #[derive(Clone)]
    struct RelayState {
        revision_response: Arc<String>,
        pre_head: Arc<String>,
        post_head: PostHead,
        /// Head queries seen so far: read 0 is pre-write, read 1 is post-write.
        head_reads: Arc<Mutex<u32>>,
        submitted: Arc<Mutex<Option<Value>>>,
        /// All filter arrays sent to POST /query, in call order.
        query_bodies: Arc<Mutex<Vec<Vec<Value>>>>,
    }

    async fn relay(
        post_head: PostHead,
    ) -> (
        String,
        Arc<Mutex<Option<Value>>>,
        Arc<Mutex<Vec<Vec<Value>>>>,
    ) {
        let submitted: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let query_bodies: Arc<Mutex<Vec<Vec<Value>>>> = Arc::new(Mutex::new(Vec::new()));
        let state = RelayState {
            revision_response: Arc::new(json!([canvas_event(REVISION, 1_000, None)]).to_string()),
            pre_head: Arc::new(json!([canvas_event(HEAD, 2_000, None)]).to_string()),
            post_head,
            head_reads: Arc::new(Mutex::new(0)),
            submitted: submitted.clone(),
            query_bodies: query_bodies.clone(),
        };
        let app = Router::new()
            .route(
                "/query",
                post(|State(s): State<RelayState>, body: Bytes| async move {
                    use axum::http::StatusCode;
                    // Capture every request body for filter-field assertions.
                    if let Ok(filters) = serde_json::from_slice::<Vec<Value>>(&body) {
                        s.query_bodies.lock().unwrap().push(filters);
                    }
                    let is_ids_query = std::str::from_utf8(&body)
                        .map(|b| b.contains("\"ids\""))
                        .unwrap_or(false);
                    if is_ids_query {
                        return (StatusCode::OK, (*s.revision_response).clone());
                    }
                    let mut reads = s.head_reads.lock().unwrap();
                    let read = *reads;
                    *reads += 1;
                    if read == 0 {
                        return (StatusCode::OK, (*s.pre_head).clone());
                    }
                    // Post-write verification read that fails: the submit was
                    // accepted, so the command reports success despite this 500.
                    if matches!(s.post_head, PostHead::ReadFails) {
                        return (StatusCode::INTERNAL_SERVER_ERROR, String::new());
                    }
                    // Post-write head: synthesize from the captured submission.
                    let our_id = s
                        .submitted
                        .lock()
                        .unwrap()
                        .as_ref()
                        .and_then(|e| e.get("id"))
                        .and_then(|v| v.as_str())
                        .expect("post-write head read must follow a submit")
                        .to_string();
                    let body = match s.post_head {
                        PostHead::OurEvent => {
                            json!([canvas_event(&our_id, 2_001, Some(HEAD))]).to_string()
                        }
                        PostHead::StrangerBuildsOnUs => {
                            json!([canvas_event(STRANGER, 2_002, Some(&our_id))]).to_string()
                        }
                        PostHead::StrangerBuildsOnUsTransitively => {
                            // C(head, exp=STRANGER) → B(STRANGER, exp=ours) →
                            // ours: the whole chain is in the returned stream so
                            // the ancestry walk reaches ours. Newest first.
                            json!([
                                canvas_event(THIRD, 2_003, Some(STRANGER)),
                                canvas_event(STRANGER, 2_002, Some(&our_id)),
                                canvas_event(&our_id, 2_001, Some(HEAD)),
                            ])
                            .to_string()
                        }
                        PostHead::Stranger => {
                            json!([canvas_event(STRANGER, 2_002, Some(HEAD))]).to_string()
                        }
                        PostHead::ReadFails => unreachable!("handled above"),
                    };
                    (StatusCode::OK, body)
                }),
            )
            .route(
                "/events",
                post(|State(s): State<RelayState>, body: Bytes| async move {
                    let event: Value = serde_json::from_slice(&body).expect("event json");
                    *s.submitted.lock().unwrap() = Some(event);
                    r#"{"event_id":"deadbeef","accepted":true,"message":""}"#.to_string()
                }),
            )
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), submitted, query_bodies)
    }

    fn client(base_url: &str) -> BuzzClient {
        BuzzClient::new(base_url.to_string(), Keys::generate(), None, None).unwrap()
    }

    /// Return all filter objects from all /query calls flattened into one vec.
    fn all_filters(query_bodies: &Arc<Mutex<Vec<Vec<Value>>>>) -> Vec<Value> {
        query_bodies
            .lock()
            .unwrap()
            .iter()
            .flat_map(|batch| batch.clone())
            .collect()
    }

    /// A canvas event JSON with the given id/created_at and optional
    /// `expected-revision` ancestry tag.
    fn canvas_event(id: &str, created_at: u64, expected_revision: Option<&str>) -> Value {
        let mut tags = vec![json!(["h", CHANNEL])];
        if let Some(rev) = expected_revision {
            tags.push(json!(["expected-revision", rev]));
        }
        json!({
            "id": id,
            "pubkey": "c".repeat(64),
            "kind": 40100,
            "content": "target content",
            "created_at": created_at,
            "tags": tags,
        })
    }

    /// The post-write head is our own event → restore holds the head → success.
    #[tokio::test]
    async fn restore_survives_when_head_is_our_event() {
        let (url, submitted, _) = relay(PostHead::OurEvent).await;
        cmd_restore_canvas(&client(&url), CHANNEL, REVISION, &mut vec![])
            .await
            .expect("restore that holds the head succeeds");
        assert!(submitted.lock().unwrap().is_some(), "restore must publish");
    }

    /// A later write built on our restore (its `expected-revision` names us):
    /// the restore is still in the accepted chain → success.
    #[tokio::test]
    async fn restore_survives_when_head_builds_on_it() {
        let (url, submitted, _) = relay(PostHead::StrangerBuildsOnUs).await;
        cmd_restore_canvas(&client(&url), CHANNEL, REVISION, &mut vec![])
            .await
            .expect("restore a later write built on succeeds");
        assert!(submitted.lock().unwrap().is_some(), "restore must publish");
    }

    /// A later write two links above our restore (C→B→ours, whole chain in the
    /// stream): the ancestry walk reaches ours → success, not a supersession.
    #[tokio::test]
    async fn restore_survives_when_head_builds_on_it_transitively() {
        let (url, submitted, _) = relay(PostHead::StrangerBuildsOnUsTransitively).await;
        cmd_restore_canvas(&client(&url), CHANNEL, REVISION, &mut vec![])
            .await
            .expect("a transitive descendant of our restore succeeds");
        assert!(submitted.lock().unwrap().is_some(), "restore must publish");
    }

    /// A concurrent stranger won the visible head (no ancestry to our restore):
    /// the command returns `CliError::Conflict` naming the persisted revision.
    #[tokio::test]
    async fn restore_conflicts_when_head_is_a_stranger() {
        let (url, submitted, _) = relay(PostHead::Stranger).await;
        let err = cmd_restore_canvas(&client(&url), CHANNEL, REVISION, &mut vec![])
            .await
            .expect_err("a stranger head must be a supersession conflict");

        assert!(
            matches!(err, CliError::Conflict(_)),
            "expected Conflict (exit 5), got {err:?}"
        );
        let our_id = submitted
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|e| e.get("id"))
            .and_then(|v| v.as_str())
            .expect("the restore did publish before it was superseded")
            .to_string();
        let msg = err.to_string();
        assert!(
            msg.contains("superseded") && msg.contains("preserved in history"),
            "conflict must describe the supersession: {msg}"
        );
        assert!(
            msg.contains(&our_id),
            "conflict must name the persisted revision id {our_id}: {msg}"
        );
    }

    /// The post-write verification read fails after an accepted submit: the
    /// restore is durable, so the command reports success (exit 0), not a
    /// conflict or error — a failed read must never masquerade as a failed
    /// restore.
    #[tokio::test]
    async fn restore_succeeds_when_verification_read_fails() {
        let (url, submitted, _) = relay(PostHead::ReadFails).await;
        let mut out = vec![];
        cmd_restore_canvas(&client(&url), CHANNEL, REVISION, &mut out)
            .await
            .expect("an accepted restore whose verification read fails still succeeds");
        assert!(
            submitted.lock().unwrap().is_some(),
            "the restore did publish before the verification read failed"
        );
        let json: serde_json::Value = serde_json::from_slice(&out)
            .expect("stdout must be valid JSON even when verification read fails");
        assert!(
            json.get("event_id").is_some(),
            "stdout JSON must contain event_id"
        );
        assert!(
            json.get("accepted").is_some(),
            "stdout JSON must contain accepted"
        );
        assert!(
            json.get("message").is_some(),
            "stdout JSON must contain message"
        );
    }

    /// `restore` sends `"consistency": "strong"` on the write-influencing
    /// head reads (pre-write precondition and post-write ancestry) and does NOT
    /// send it on the revision-by-id lookup (display read).
    ///
    /// Mutation oracle — removing either `"consistency"` injection must flip the
    /// relevant assertion:
    ///   * `fetch_canvas_head(writer_pinned=true)` → pre-write head filter loses
    ///     the field.
    ///   * `fetch_canvas_ancestry` → post-write filter loses it.
    ///   * Adding `"consistency"` to `fetch_canvas_revision` → ids filter
    ///     incorrectly gains it.
    #[tokio::test]
    async fn restore_write_influencing_reads_carry_strong_consistency() {
        let (url, _, query_bodies) = relay(PostHead::OurEvent).await;
        cmd_restore_canvas(&client(&url), CHANNEL, REVISION, &mut vec![])
            .await
            .expect("restore succeeds");

        let filters = all_filters(&query_bodies);

        // IDs filter (revision fetch — display read): must NOT have consistency.
        let ids_filters: Vec<_> = filters.iter().filter(|f| f.get("ids").is_some()).collect();
        assert!(!ids_filters.is_empty(), "restore must issue an ids filter");
        for f in &ids_filters {
            assert!(
                f.get("consistency").is_none(),
                "revision-fetch (ids) filter must NOT carry consistency: {f}"
            );
        }

        // Non-ids kind-40100 filters (head reads — write-influencing): must all
        // carry consistency=strong. These are the pre-write precondition head and
        // the post-write ancestry verification read.
        let head_filters: Vec<_> = filters
            .iter()
            .filter(|f| {
                f.get("ids").is_none()
                    && f.get("kinds")
                        .and_then(|k| k.as_array())
                        .map(|a| a.iter().any(|k| k == 40100))
                        .unwrap_or(false)
            })
            .collect();
        assert!(
            !head_filters.is_empty(),
            "restore must issue at least one head/ancestry filter"
        );
        for f in &head_filters {
            assert_eq!(
                f.get("consistency").and_then(|v| v.as_str()),
                Some("strong"),
                "write-influencing head/ancestry filter must carry consistency=strong: {f}"
            );
        }
    }

    /// Relay for conflict-reconciliation tests. Models the real ambiguous retry
    /// seam in `submit_stored_event`:
    ///
    /// 1. Attempt 1 — POST /events: A is persisted on the relay; the relay then
    ///    returns HTTP 503 (a retryable status), so `with_retry_body` retries
    ///    with the same bytes.
    /// 2. A concurrent writer arrives and becomes the new head.
    /// 3. Attempt 2 — POST /events: returns the canonical 409 conflict body
    ///    because the concurrent write is now the head. `cmd_restore_canvas`
    ///    enters the reconciliation branch.
    /// 4. Ancestry walk — POST /query: returns a stream shaped by `scenario`.
    /// 5. IDs existence check — POST /query (only for `GenuinelyAbsent`,
    ///    `LegacySupersession`, and `ExistenceReadFails`): the ancestry walk
    ///    returns `canvas_write_survived` = false, so the command follows up
    ///    with a writer-pinned IDs lookup for our event.
    ///
    /// The event ID is captured so tests can assert it appears in error messages.
    #[derive(Clone, Copy)]
    enum ConflictScenario {
        /// B(expected=A) is head, A is a reachable ancestor → `canvas_write_survived`
        /// true → accepted JSON, exit 0 (same as accepted-submit success path).
        ReachableAncestor,
        /// B(no expected-revision) is head, A exists in the stream but is not
        /// an ancestor → `canvas_write_survived` false, IDs lookup finds A →
        /// supersession naming A.
        LegacySupersession,
        /// A is absent (only a stranger, no A in stream or IDs lookup) → genuine
        /// conflict → original `CliError::Relay { 409, .. }` returned unchanged.
        GenuinelyAbsent,
        /// Ancestry read fails (HTTP 500) → `CliError::DeliveryUnknown`.
        ReadFails,
        /// Ancestry succeeds with A unreachable (canvas_write_survived = false),
        /// then the IDs existence check fails (HTTP 500) → `CliError::DeliveryUnknown`.
        ExistenceReadFails,
    }

    async fn conflict_relay(
        scenario: ConflictScenario,
    ) -> (
        String,
        Arc<Mutex<Option<Value>>>,
        Arc<Mutex<Option<String>>>,
    ) {
        use std::sync::atomic::{AtomicU32, Ordering};

        let submitted: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let submitted_q = submitted.clone();
        let submitted_ev = submitted.clone();
        // Captures the first post-submit IDs query body (the writer-pinned
        // existence check). `None` when the query is never reached (e.g.
        // ReachableAncestor where canvas_write_survived = true).
        let ids_query_body: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let ids_query_body_q = ids_query_body.clone();
        let events_attempt: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));
        let events_attempt2 = events_attempt.clone();

        let app = Router::new()
            .route(
                "/query",
                post(move |body: Bytes| {
                    let sub = submitted_q.clone();
                    let ids_cap = ids_query_body_q.clone();
                    async move {
                        use axum::http::StatusCode;
                        let body_str = std::str::from_utf8(&body).unwrap_or("");
                        let is_ids_query = body_str.contains("\"ids\"");
                        let our_id_opt = sub
                            .lock()
                            .unwrap()
                            .as_ref()
                            .and_then(|e| e.get("id"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        if is_ids_query && our_id_opt.is_none() {
                            // Pre-submit revision content fetch — always succeeds.
                            return (
                                StatusCode::OK,
                                json!([canvas_event(REVISION, 1_000, None)]).to_string(),
                            );
                        }
                        if our_id_opt.is_none() {
                            // Pre-submit head read — REVISION ≠ HEAD so command proceeds.
                            return (
                                StatusCode::OK,
                                json!([canvas_event(HEAD, 2_000, None)]).to_string(),
                            );
                        }
                        let our_id = our_id_opt.unwrap();
                        if is_ids_query {
                            // Post-conflict writer-pinned IDs existence check for our_id.
                            // Only reached when canvas_write_survived is false (i.e.
                            // LegacySupersession, GenuinelyAbsent, and ExistenceReadFails
                            // scenarios). Capture the first IDs body for consistency tests.
                            {
                                let mut cap = ids_cap.lock().unwrap();
                                if cap.is_none() {
                                    *cap = Some(body_str.to_owned());
                                }
                            }
                            return match scenario {
                                ConflictScenario::LegacySupersession => {
                                    // A is in the relay — return it so the existence
                                    // check confirms persistence.
                                    (
                                        StatusCode::OK,
                                        json!([canvas_event(&our_id, 2_001, Some(HEAD))])
                                            .to_string(),
                                    )
                                }
                                ConflictScenario::GenuinelyAbsent => {
                                    // A was never stored — empty response.
                                    (StatusCode::OK, json!([]).to_string())
                                }
                                ConflictScenario::ExistenceReadFails => {
                                    // IDs existence check itself fails — the outcome
                                    // cannot be determined → DeliveryUnknown.
                                    (StatusCode::INTERNAL_SERVER_ERROR, String::new())
                                }
                                // ReachableAncestor: canvas_write_survived = true,
                                // never reaches the existence check.
                                // ReadFails: ancestry read returns 500,
                                // never reaches the existence check.
                                _ => unreachable!(
                                    "existence check only reached for LegacySupersession/GenuinelyAbsent/ExistenceReadFails"
                                ),
                            };
                        }
                        // Post-conflict ancestry walk: shape per scenario.
                        match scenario {
                            ConflictScenario::ReadFails => {
                                (StatusCode::INTERNAL_SERVER_ERROR, String::new())
                            }
                            ConflictScenario::GenuinelyAbsent => {
                                // A is absent — genuine conflict. Stream contains
                                // only an unrelated stranger (no A, no B).
                                (
                                    StatusCode::OK,
                                    json!([canvas_event(STRANGER, 2_002, None)]).to_string(),
                                )
                            }
                            ConflictScenario::ExistenceReadFails => {
                                // Ancestry succeeds but A is unreachable (stranger
                                // only, canvas_write_survived = false). The IDs check
                                // that follows returns 500 above.
                                (
                                    StatusCode::OK,
                                    json!([canvas_event(STRANGER, 2_002, None)]).to_string(),
                                )
                            }
                            ConflictScenario::ReachableAncestor => {
                                // B(expected=A) is head: B built on A, A is in the
                                // ancestry chain → canvas_write_survived = true →
                                // command returns accepted JSON (exit 0).
                                (
                                    StatusCode::OK,
                                    json!([
                                        canvas_event(STRANGER, 2_002, Some(&our_id)),
                                        canvas_event(&our_id, 2_001, Some(HEAD)),
                                    ])
                                    .to_string(),
                                )
                            }
                            ConflictScenario::LegacySupersession => {
                                // B(no expected-revision) is head: B does not link
                                // to A, so canvas_write_survived = false, but A is
                                // in the stream (not at head). The IDs existence
                                // check follows and confirms A is stored.
                                (
                                    StatusCode::OK,
                                    json!([
                                        canvas_event(STRANGER, 2_002, None),
                                        canvas_event(&our_id, 2_001, Some(HEAD)),
                                    ])
                                    .to_string(),
                                )
                            }
                        }
                    }
                }),
            )
            .route(
                "/events",
                post(move |body: Bytes| {
                    let sub = submitted_ev.clone();
                    let attempt_ctr = events_attempt2.clone();
                    async move {
                        use axum::http::StatusCode;
                        let event: Value = serde_json::from_slice(&body).expect("event json");
                        let attempt = attempt_ctr.fetch_add(1, Ordering::SeqCst);
                        if attempt == 0 {
                            // Attempt 1: A is persisted; simulate a lost response by
                            // returning 503 (retried by with_retry_body). This models
                            // a network-level response loss where the relay stored the
                            // event but the client never received confirmation.
                            // We capture the event ID here for use in ancestry responses.
                            *sub.lock().unwrap() = Some(event);
                            return (
                                StatusCode::SERVICE_UNAVAILABLE,
                                r#"{"error":"relay temporarily unavailable"}"#.to_string(),
                            );
                        }
                        // Attempt 2 (same bytes — `submit_stored_event` retry):
                        // A is already stored; B built on A; relay returns canonical 409.
                        (
                            StatusCode::CONFLICT,
                            r#"{"error":"conflict: canvas changed since it was loaded"}"#
                                .to_string(),
                        )
                    }
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), submitted, ids_query_body)
    }

    /// A non-409 relay error whose body happens to contain "canvas changed"
    /// must NOT enter the conflict-reconciliation branch — it must propagate
    /// as a plain `CliError::Relay` without triggering an ancestry walk.
    ///
    /// Mutation oracle: removing the `status: 409` guard from the match arm
    /// makes this return `CliError::Conflict` (ancestry walk finds our event
    /// reachable, so reconciliation misclassifies it as a supersession) instead
    /// of `CliError::Relay { status: 500, .. }`.
    #[tokio::test]
    async fn non_409_lookalike_with_conflict_phrase_is_not_reconciled() {
        // Relay: /events returns 500 + conflict phrase but DOES capture the event
        // ID. /query post-submit returns a stream where our event IS reachable
        // (so that if the classifier incorrectly fires, it produces Conflict — a
        // distinct, detectable outcome).
        let submitted: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let submitted_q = submitted.clone();
        let submitted_ev = submitted.clone();
        let app = Router::new()
            .route(
                "/query",
                post(move |body: Bytes| {
                    let sub = submitted_q.clone();
                    async move {
                        use axum::http::StatusCode;
                        let is_ids_query = std::str::from_utf8(&body)
                            .map(|b| b.contains("\"ids\""))
                            .unwrap_or(false);
                        if is_ids_query {
                            return (
                                StatusCode::OK,
                                json!([canvas_event(REVISION, 1_000, None)]).to_string(),
                            );
                        }
                        let our_id_opt = sub
                            .lock()
                            .unwrap()
                            .as_ref()
                            .and_then(|e| e.get("id"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        if our_id_opt.is_none() {
                            // Pre-submit head — REVISION ≠ HEAD so command proceeds.
                            return (
                                StatusCode::OK,
                                json!([canvas_event(HEAD, 2_000, None)]).to_string(),
                            );
                        }
                        // Post-error ancestry walk: our event IS reachable.
                        // If the status guard is missing, reconciliation fires here
                        // and returns CliError::Conflict — the oracle fires.
                        let our_id = our_id_opt.unwrap();
                        (
                            StatusCode::OK,
                            json!([
                                canvas_event(STRANGER, 2_002, Some(&our_id)),
                                canvas_event(&our_id, 2_001, Some(HEAD)),
                            ])
                            .to_string(),
                        )
                    }
                }),
            )
            .route(
                "/events",
                post(move |body: Bytes| {
                    let sub = submitted_ev.clone();
                    async move {
                        use axum::http::StatusCode;
                        let event: Value = serde_json::from_slice(&body).expect("event json");
                        *sub.lock().unwrap() = Some(event);
                        // 500 with the conflict phrase in the body — a lookalike.
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            r#"{"error":"canvas changed — internal relay fault"}"#.to_string(),
                        )
                    }
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}");
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let err = cmd_restore_canvas(&client(&url), CHANNEL, REVISION, &mut vec![])
            .await
            .expect_err("a 500 error must propagate as Relay, not enter reconciliation");

        // With the 409 status guard: the 500 exits the match via Err(e) => return Err(e)
        // and arrives here as Relay{500}.
        // Without the guard: the body phrase matches → reconciliation fires →
        // ancestry walk finds our event reachable → CliError::Conflict returned instead.
        assert!(
            !matches!(err, CliError::Conflict(_)),
            "non-409 error with conflict phrase must not be classified as Conflict: {err:?}"
        );
        assert!(
            matches!(err, CliError::Relay { status: 500, .. }),
            "non-409 body-lookalike must propagate as Relay{{500}}, got {err:?}"
        );
    }

    /// A conflict-shaped submit where `canvas_write_survived` is true (B(expected=A)
    /// is head, A is a reachable ancestor): A committed before the 409 response was
    /// lost. The command must return accepted JSON (exit 0) — identical to a clean
    /// accepted submit — NOT `CliError::Conflict`, which would incorrectly prompt
    /// the caller to re-restore over B.
    ///
    /// Mutation oracle: removing the conflict-reconciliation block from
    /// `cmd_restore_canvas` (restoring `client.submit_event(event).await?`) makes
    /// this return `CliError::Relay` instead of `Ok(())`.
    #[tokio::test]
    async fn conflict_shaped_submit_succeeds_when_our_event_is_reachable_ancestor() {
        let (url, submitted, _) = conflict_relay(ConflictScenario::ReachableAncestor).await;
        let mut out = vec![];
        cmd_restore_canvas(&client(&url), CHANNEL, REVISION, &mut out)
            .await
            .expect("a conflict where A is a reachable ancestor must succeed (exit 0)");
        assert!(
            submitted.lock().unwrap().is_some(),
            "the restore must have submitted before the 409"
        );
        // Must emit the same accepted JSON as a clean submit.
        let json: serde_json::Value =
            serde_json::from_slice(&out).expect("stdout must be valid JSON");
        assert!(
            json.get("event_id").is_some(),
            "stdout JSON must contain event_id: {json}"
        );
        assert!(
            json.get("accepted").is_some(),
            "stdout JSON must contain accepted: {json}"
        );
        assert!(
            json.get("message").is_some(),
            "stdout JSON must contain message: {json}"
        );
    }

    /// A conflict-shaped submit where an unconditional (legacy, no `expected-revision`)
    /// write B is head and our event A is present in the stream but not an ancestor:
    /// `canvas_write_survived` is false, but the writer-pinned IDs lookup confirms A
    /// exists. The command must return `CliError::Conflict` naming A — superseded,
    /// preserved in history — NOT the original 409 relay error that would wrongly
    /// tell the caller the restore was rejected.
    ///
    /// Mutation oracle: removing the IDs existence check (returning the original
    /// 409 relay error whenever `canvas_write_survived` is false) makes this return
    /// `CliError::Relay` instead of `CliError::Conflict`.
    #[tokio::test]
    async fn conflict_shaped_submit_superseded_by_legacy_write_when_event_exists_but_not_ancestor()
    {
        let (url, submitted, _) = conflict_relay(ConflictScenario::LegacySupersession).await;
        let err = cmd_restore_canvas(&client(&url), CHANNEL, REVISION, &mut vec![])
            .await
            .expect_err("a conflict where A exists but is not an ancestor must be a supersession");

        assert!(
            matches!(err, CliError::Conflict(_)),
            "expected Conflict (exit 5 — superseded by legacy write, preserved), got {err:?}"
        );
        let our_id = submitted
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|e| e.get("id"))
            .and_then(|v| v.as_str())
            .expect("the restore must have submitted before the 409")
            .to_string();
        let msg = err.to_string();
        assert!(
            msg.contains("superseded") && msg.contains("preserved in history"),
            "conflict message must describe the supersession: {msg}"
        );
        assert!(
            msg.contains(&our_id),
            "conflict message must name the persisted revision id {our_id}: {msg}"
        );
    }

    /// A conflict-shaped submit where the ancestry walk shows our event is
    /// absent: the relay genuinely rejected the write (stale precondition, not
    /// a lost response). The command must return the ORIGINAL `CliError::Relay`
    /// unchanged — status 409, body as received from the relay — rather than a
    /// fabricated replacement body.
    ///
    /// Mutation oracle: always returning `Conflict` from the reconciliation
    /// block (ignoring `canvas_write_survived`) makes this return `CliError::Conflict`
    /// instead of `CliError::Relay`.
    #[tokio::test]
    async fn conflict_shaped_submit_stays_relay_error_when_our_event_absent() {
        let (url, _, _) = conflict_relay(ConflictScenario::GenuinelyAbsent).await;
        let err = cmd_restore_canvas(&client(&url), CHANNEL, REVISION, &mut vec![])
            .await
            .expect_err("a genuine conflict must error");

        // Must be the original Relay error (409) — not fabricated, not Conflict.
        match &err {
            CliError::Relay { status, body } => {
                assert_eq!(
                    *status, 409,
                    "absent case must preserve the original 409 status"
                );
                assert!(
                    body.contains("canvas changed"),
                    "absent case must preserve the original conflict body: {body}"
                );
            }
            other => {
                panic!("expected Relay error (genuine conflict, nothing stored), got {other:?}")
            }
        }
    }

    /// A conflict-shaped submit where the ancestry walk itself fails. The
    /// outcome is genuinely unknown: the restore may or may not be stored.
    /// The command must return `CliError::DeliveryUnknown` (exit 2,
    /// category `delivery_unknown`) — not a false success, not a false conflict,
    /// and not the generic `CliError::Other` (exit 4, category `error`).
    ///
    /// Mutation oracle: returning `CliError::Conflict` when the ancestry read
    /// fails makes this return `Conflict` instead of `DeliveryUnknown`.
    #[tokio::test]
    async fn conflict_shaped_submit_with_failed_ancestry_read_returns_unknown_outcome() {
        let (url, submitted, _) = conflict_relay(ConflictScenario::ReadFails).await;
        let err = cmd_restore_canvas(&client(&url), CHANNEL, REVISION, &mut vec![])
            .await
            .expect_err("unknown outcome must error");

        assert!(
            matches!(err, CliError::DeliveryUnknown(_)),
            "expected DeliveryUnknown (exit 2, category delivery_unknown — outcome unknown), got {err:?}"
        );
        let our_id = submitted
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|e| e.get("id"))
            .and_then(|v| v.as_str())
            .expect("the restore must have submitted before the failed read")
            .to_string();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown"),
            "error message must state outcome is unknown: {msg}"
        );
        assert!(
            msg.contains(&our_id),
            "error message must name the event id {our_id}: {msg}"
        );
    }

    /// A conflict-shaped submit where the ancestry walk finds A unreachable
    /// (canvas_write_survived = false) and then the writer-pinned IDs existence
    /// check itself fails (HTTP 500). The outcome is unknown — A may or may not
    /// be stored. The command must return `CliError::DeliveryUnknown` naming A's
    /// ID, not a false `CliError::Relay` (absent/original 409) and not a false
    /// `CliError::Conflict` (treated as supersession without confirmation).
    ///
    /// Mutation oracle: changing the existence-query `Err(_)` arm to return the
    /// original 409 relay error makes this return `CliError::Relay` instead of
    /// `CliError::DeliveryUnknown`.
    #[tokio::test]
    async fn conflict_shaped_submit_with_failed_existence_read_returns_unknown_outcome() {
        let (url, submitted, _) = conflict_relay(ConflictScenario::ExistenceReadFails).await;
        let err = cmd_restore_canvas(&client(&url), CHANNEL, REVISION, &mut vec![])
            .await
            .expect_err("unknown outcome from existence-check failure must error");

        assert!(
            matches!(err, CliError::DeliveryUnknown(_)),
            "expected DeliveryUnknown (existence check failed — outcome unknown), got {err:?}"
        );
        let our_id = submitted
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|e| e.get("id"))
            .and_then(|v| v.as_str())
            .expect("the restore must have submitted before the failed existence check")
            .to_string();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown"),
            "error message must state outcome is unknown: {msg}"
        );
        assert!(
            msg.contains(&our_id),
            "error message must name the event id {our_id}: {msg}"
        );
    }

    /// The writer-pinned IDs existence check sent during 409-reconciliation
    /// must carry `consistency=strong` with exactly the expected filter fields
    /// so that replica lag cannot hide a durable event A and produce a false
    /// absence classification.
    ///
    /// Mutation oracle: removing `"consistency": "strong"` from
    /// `fetch_canvas_event_exists` makes the assertion on the captured request
    /// body fail.
    #[tokio::test]
    async fn conflict_shaped_submit_existence_read_carries_strong_consistency() {
        // Use LegacySupersession: ancestry succeeds with canvas_write_survived =
        // false, so the existence check is reached and its body is captured.
        let (url, submitted, ids_body_cap) =
            conflict_relay(ConflictScenario::LegacySupersession).await;
        cmd_restore_canvas(&client(&url), CHANNEL, REVISION, &mut vec![])
            .await
            .expect_err("legacy supersession must return Conflict");

        let our_id = submitted
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|e| e.get("id"))
            .and_then(|v| v.as_str())
            .expect("the restore must have submitted before the existence check")
            .to_string();

        let raw = ids_body_cap
            .lock()
            .unwrap()
            .clone()
            .expect("existence query body must have been captured");
        // query() wraps the filter in a JSON array ([filter]) per the HTTP bridge contract.
        let filters: Vec<serde_json::Value> =
            serde_json::from_str(&raw).expect("existence query body must be a valid JSON array");
        let filter = filters
            .into_iter()
            .next()
            .expect("existence query body must contain at least one filter");

        // ids=[our_id] — queries precisely the event we submitted.
        assert_eq!(
            filter.get("ids").and_then(|v| v.as_array()),
            Some(&vec![serde_json::json!(our_id)]),
            "existence filter must query exactly our submitted event id: {filter}"
        );
        // kinds=[40100] — scoped to canvas events only.
        assert_eq!(
            filter.get("kinds").and_then(|v| v.as_array()),
            Some(&vec![serde_json::json!(40100)]),
            "existence filter must restrict to kind 40100: {filter}"
        );
        // #h=[channel] — scoped to the correct channel.
        assert_eq!(
            filter.get("#h").and_then(|v| v.as_array()),
            Some(&vec![serde_json::json!(CHANNEL)]),
            "existence filter must restrict to the correct channel: {filter}"
        );
        // limit=1 — one-shot check.
        assert_eq!(
            filter.get("limit").and_then(|v| v.as_u64()),
            Some(1),
            "existence filter must set limit=1: {filter}"
        );
        // consistency=strong — writer-pinned, no replica lag.
        assert_eq!(
            filter.get("consistency").and_then(|v| v.as_str()),
            Some("strong"),
            "existence filter must carry consistency=strong to prevent replica-lag false absence: {filter}"
        );
    }
}

/// Item 4 regression: the already-current short-circuit must emit structured
/// JSON on stdout (matching the JSON-only stdout contract) with the fields
/// `{event_id, accepted, message}`. A bare prose `println!` violates the
/// JSON-only stdout contract.
#[cfg(test)]
mod restore_canvas_already_current_tests {
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::routing::post;
    use axum::Router;
    use nostr::Keys;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    use super::cmd_restore_canvas;
    use crate::client::BuzzClient;

    const CHANNEL: &str = "326d56bc-c96c-4af0-86a1-5e804cd1b467";
    // REVISION == HEAD so the command short-circuits without submitting.
    const REVISION_EQ_HEAD: &str =
        "2222222222222222222222222222222222222222222222222222222222222222";

    /// Relay that serves REVISION_EQ_HEAD both as the revision-by-id content
    /// and as the current head, so `cmd_restore_canvas` takes the already-current
    /// short-circuit. No `/events` route is needed because the command returns
    /// before building or submitting the event.
    async fn already_current_relay() -> (String, Arc<Mutex<Vec<Vec<Value>>>>) {
        let query_bodies: Arc<Mutex<Vec<Vec<Value>>>> = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route(
                "/query",
                post(
                    |State(qb): State<Arc<Mutex<Vec<Vec<Value>>>>>, body: Bytes| async move {
                        if let Ok(filters) = serde_json::from_slice::<Vec<Value>>(&body) {
                            qb.lock().unwrap().push(filters);
                        }
                        let is_ids_query = std::str::from_utf8(&body)
                            .map(|b| b.contains("\"ids\""))
                            .unwrap_or(false);
                        let event = json!({
                            "id": REVISION_EQ_HEAD,
                            "pubkey": "b".repeat(64),
                            "kind": 40100,
                            "content": "canvas content",
                            "created_at": 2_000u64,
                            "tags": [["h", CHANNEL]],
                        });
                        if is_ids_query {
                            return json!([event]).to_string();
                        }
                        // Head query — returns the same id so the command short-circuits.
                        json!([event]).to_string()
                    },
                ),
            )
            .with_state(query_bodies.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), query_bodies)
    }

    fn client(base_url: &str) -> BuzzClient {
        BuzzClient::new(base_url.to_string(), Keys::generate(), None, None).unwrap()
    }

    /// The already-current short-circuit must return Ok (exit 0) AND emit
    /// structured JSON on stdout with `event_id`, `accepted`, and `message`
    /// fields. Captured via the injected `out` writer.
    ///
    /// Mutation oracle: reverting to `println!("revision {revision} is already
    /// the current revision")` emits prose instead of JSON. Parsing the output
    /// with `serde_json::from_str` would fail, and the `accepted` / `event_id` /
    /// `message` field assertions would not hold.
    #[tokio::test]
    async fn already_current_restore_emits_json_with_required_fields() {
        let (url, _) = already_current_relay().await;
        let mut out: Vec<u8> = Vec::new();
        cmd_restore_canvas(&client(&url), CHANNEL, REVISION_EQ_HEAD, &mut out)
            .await
            .expect("already-current restore must succeed");

        let output = String::from_utf8(out).expect("stdout must be valid UTF-8");
        let json: Value = serde_json::from_str(output.trim()).expect("stdout must be valid JSON");

        assert_eq!(
            json.get("event_id").and_then(|v| v.as_str()),
            Some(REVISION_EQ_HEAD),
            "event_id must match the revision: {json}"
        );
        assert_eq!(
            json.get("accepted").and_then(|v| v.as_bool()),
            Some(true),
            "accepted must be true: {json}"
        );
        assert!(
            json.get("message").and_then(|v| v.as_str()).is_some(),
            "message field must be present: {json}"
        );
    }
}
