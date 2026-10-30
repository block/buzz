//! `buzz artifacts` — editable channel records (NIP-AR, kind 45010).
//!
//! The relay owns identity, access and revision order. Every write is a
//! complete snapshot that names the current revision in `prev`; a lost race is
//! a relay conflict and exits 5. Reads use bounded `/query` artifact views.

use std::collections::HashSet;
use std::io::Read;

use buzz_core::artifact::{self, MAX_PAGE_SIZE};
use buzz_core::kind::KIND_ARTIFACT;
use buzz_sdk::{build_artifact_revision, derive_artifact_id, ArtifactOp, ArtifactRevision};
use serde_json::{json, Value};
use uuid::Uuid;

use super::event_output::format_events;
use crate::client::{
    create_response_with_id_if_accepted, normalize_events, normalize_write_response, BuzzClient,
};
use crate::error::CliError;
use crate::validate::{sdk_err, validate_content_size, MAX_CONTENT_BYTES};
use crate::{ArtifactsCmd, OutputFormat};

/// Conflict rounds `create --ensure` tries before reporting the conflict.
const ENSURE_MAX_ATTEMPTS: usize = 3;

/// Default page size for `list`.
pub const LIST_DEFAULT_LIMIT: u32 = 100;

/// Largest page `list` may request; the relay enforces the same bound.
pub const LIST_MAX_LIMIT: u32 = MAX_PAGE_SIZE as u32;

/// A parsed kind 45010 revision as the relay returned it.
#[derive(Debug)]
struct Revision {
    event: Value,
    event_id: String,
    artifact_id: String,
    channel: String,
    artifact_type: String,
    deleted: bool,
    title: Option<String>,
    content: String,
    root: Option<String>,
    tags: Vec<Vec<String>>,
}

fn malformed(detail: impl std::fmt::Display) -> CliError {
    CliError::Other(format!(
        "relay returned a malformed artifact revision: {detail}"
    ))
}

impl Revision {
    fn parse(event: &Value) -> Result<Self, CliError> {
        if event.get("kind").and_then(Value::as_u64) != Some(u64::from(KIND_ARTIFACT)) {
            return Err(malformed("not a kind 45010 event"));
        }
        let text = |key: &str| {
            event
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| malformed(format!("missing {key}")))
        };
        let mut envelope = std::collections::HashMap::new();
        let mut tags = Vec::new();
        for raw in event
            .get("tags")
            .and_then(Value::as_array)
            .ok_or_else(|| malformed("missing tags"))?
        {
            let tag: Vec<String> = raw
                .as_array()
                .and_then(|parts| {
                    parts
                        .iter()
                        .map(|p| p.as_str().map(str::to_string))
                        .collect()
                })
                .ok_or_else(|| malformed("tag is not an array of strings"))?;
            let name = tag.first().ok_or_else(|| malformed("empty tag"))?.as_str();
            if ["ar", "d", "h", "type", "title", "op", "root", "prev"].contains(&name) {
                envelope.insert(name.to_string(), tag.get(1).cloned().unwrap_or_default());
            } else if name != "auth" {
                tags.push(tag);
            }
        }
        let require = |name: &str| {
            envelope
                .get(name)
                .cloned()
                .ok_or_else(|| malformed(format!("missing {name} tag")))
        };
        Ok(Self {
            event_id: text("id")?,
            artifact_id: require("d")?,
            channel: require("h")?,
            artifact_type: require("type")?,
            deleted: require("op")? == "delete",
            title: envelope.get("title").cloned(),
            content: text("content")?,
            root: envelope.get("root").cloned(),
            tags,
            event: event.clone(),
        })
    }
}

fn canonical_uuid(value: &str, what: &str) -> Result<Uuid, CliError> {
    artifact::canonical_uuid(value).map_err(|e| CliError::Usage(format!("{what} {value:?}: {e}")))
}

fn read_limited(mut reader: impl Read, what: &str) -> Result<String, CliError> {
    let mut buf = String::new();
    reader
        .by_ref()
        .take(MAX_CONTENT_BYTES as u64 + 1)
        .read_to_string(&mut buf)
        .map_err(|e| CliError::Usage(format!("failed to read {what}: {e}")))?;
    validate_content_size(&buf)?;
    Ok(buf)
}

/// Resolve `--content` / `--content-file` (`-` is stdin) to bounded text.
fn read_content(
    content: Option<String>,
    content_file: Option<String>,
) -> Result<Option<String>, CliError> {
    match (content, content_file) {
        (Some(_), Some(_)) => Err(CliError::Usage(
            "pass either --content or --content-file, not both".into(),
        )),
        (Some(content), None) => {
            validate_content_size(&content)?;
            Ok(Some(content))
        }
        (None, Some(path)) if path == "-" => {
            read_limited(std::io::stdin().lock(), "stdin").map(Some)
        }
        (None, Some(path)) => {
            let file = std::fs::File::open(&path)
                .map_err(|e| CliError::Usage(format!("failed to read {path:?}: {e}")))?;
            read_limited(file, &path).map(Some)
        }
        (None, None) => Ok(None),
    }
}

async fn query_events(client: &BuzzClient, filter: Value) -> Result<Vec<Value>, CliError> {
    let raw = client.query(&filter).await?;
    serde_json::from_str::<Vec<Value>>(&raw)
        .map_err(|e| CliError::Other(format!("relay artifact query returned non-array JSON: {e}")))
}

/// Keep only live current revisions that match the request.
///
/// The relay chooses the head. This refuses a reply that cannot be one: a
/// foreign event, a different channel or type, or two revisions of one
/// artifact. Deletions are dropped.
fn select_heads(
    events: &[Value],
    channel: &str,
    artifact_type: Option<&str>,
) -> Result<Vec<Value>, CliError> {
    let mut seen = HashSet::new();
    let mut heads = Vec::new();
    for event in events {
        let revision = Revision::parse(event)?;
        if revision.channel != channel || artifact_type.is_some_and(|t| t != revision.artifact_type)
        {
            return Err(malformed("revision outside the requested channel or type"));
        }
        if !seen.insert(revision.artifact_id.clone()) {
            return Err(malformed(format!(
                "several revisions of artifact {}",
                revision.artifact_id
            )));
        }
        if !revision.deleted {
            heads.push(event.clone());
        }
    }
    Ok(heads)
}

/// Current revision of `id` including a deletion, or `None` when the relay
/// reports nothing (unknown, or not readable by this key).
async fn fetch_head(client: &BuzzClient, id: &str) -> Result<Option<Revision>, CliError> {
    let events = query_events(
        client,
        json!({"kinds": [KIND_ARTIFACT], "artifact": "head", "#d": [id]}),
    )
    .await?;
    match events.as_slice() {
        [] => Ok(None),
        [event] => {
            let revision = Revision::parse(event)?;
            if revision.artifact_id != id {
                return Err(malformed("revision of a different artifact"));
            }
            Ok(Some(revision))
        }
        _ => Err(malformed("several heads for one artifact")),
    }
}

fn not_found(id: &str, channel: &str) -> CliError {
    CliError::NotFound(format!(
        "artifact {id} not found in channel {channel} (unknown, deleted, moved, or not readable)"
    ))
}

async fn fetch_live_head(
    client: &BuzzClient,
    channel: &str,
    id: &str,
) -> Result<Revision, CliError> {
    match fetch_head(client, id).await? {
        Some(head) if head.channel == channel && !head.deleted => Ok(head),
        _ => Err(not_found(id, channel)),
    }
}

fn conflict(error: CliError) -> CliError {
    match error {
        CliError::Relay { status: 409, body } => CliError::Conflict(format!(
            "{}; run `buzz artifacts get`, reapply the change to the current revision, then retry",
            body.strip_prefix("conflict: ").unwrap_or(&body)
        )),
        other => other,
    }
}

async fn publish(client: &BuzzClient, revision: &ArtifactRevision<'_>) -> Result<String, CliError> {
    let event = client.sign_event(build_artifact_revision(revision).map_err(sdk_err)?)?;
    artifact::validate(&event)
        .map_err(|e| CliError::Usage(format!("invalid artifact revision: {e}")))?;
    let raw = client.submit_event(event).await.map_err(conflict)?;
    let accepted = serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|v| v.get("accepted").and_then(Value::as_bool));
    if accepted != Some(true) {
        return Err(CliError::Other(format!(
            "relay did not accept the artifact revision: {raw}"
        )));
    }
    Ok(normalize_write_response(&raw))
}

fn with_artifact_id(response: &str, id: &str) -> String {
    create_response_with_id_if_accepted(response, "artifact_id", id)
}

/// Snapshot of `head` as a new revision of the same artifact and channel.
fn follow_up<'a>(
    head: &'a Revision,
    channel_id: Uuid,
    artifact_id: Uuid,
    op: ArtifactOp,
    title: Option<&'a str>,
    content: &'a str,
) -> ArtifactRevision<'a> {
    ArtifactRevision {
        artifact_id,
        channel_id,
        artifact_type: &head.artifact_type,
        op,
        prev: Some(&head.event_id),
        root: head.root.as_deref(),
        title,
        tags: &head.tags,
        content,
    }
}

async fn list(
    client: &BuzzClient,
    format: &OutputFormat,
    channel: &str,
    artifact_type: Option<&str>,
    limit: u32,
) -> Result<String, CliError> {
    canonical_uuid(channel, "channel")?;
    if let Some(t) = artifact_type {
        artifact::validate_type(t).map_err(|e| CliError::Usage(format!("--type: {e}")))?;
    }
    if !(1..=LIST_MAX_LIMIT).contains(&limit) {
        return Err(CliError::Usage(format!(
            "--limit must be between 1 and {LIST_MAX_LIMIT}"
        )));
    }
    let mut filter = json!({
        "kinds": [KIND_ARTIFACT],
        "artifact": "current",
        "#h": [channel],
        "limit": limit,
    });
    if let Some(t) = artifact_type {
        filter["#type"] = json!([t]);
    }
    let events = query_events(client, filter).await?;
    let heads = select_heads(&events, channel, artifact_type)?;
    Ok(format_events(&normalize_events(&heads), format))
}

async fn get(
    client: &BuzzClient,
    format: &OutputFormat,
    channel: &str,
    id: &str,
) -> Result<String, CliError> {
    canonical_uuid(channel, "channel")?;
    canonical_uuid(id, "artifact id")?;
    let head = fetch_live_head(client, channel, id).await?;
    Ok(format_events(
        &normalize_events(std::slice::from_ref(&head.event)),
        format,
    ))
}

struct CreateArgs {
    channel: String,
    artifact_type: String,
    title: String,
    content: Option<String>,
    content_file: Option<String>,
    ensure: Option<String>,
}

async fn create(client: &BuzzClient, args: CreateArgs) -> Result<String, CliError> {
    let channel_id = canonical_uuid(&args.channel, "channel")?;
    artifact::validate_type(&args.artifact_type)
        .map_err(|e| CliError::Usage(format!("--type: {e}")))?;
    let content = read_content(args.content, args.content_file)?.unwrap_or_default();
    let id = match &args.ensure {
        Some(key) => derive_artifact_id(channel_id, &args.artifact_type, key).map_err(sdk_err)?,
        None => Uuid::new_v4(),
    };
    let revision = ArtifactRevision {
        artifact_id: id,
        channel_id,
        artifact_type: &args.artifact_type,
        op: ArtifactOp::Create,
        prev: None,
        root: None,
        title: Some(&args.title),
        tags: &[],
        content: &content,
    };
    match publish(client, &revision).await {
        Err(CliError::Conflict(_)) if args.ensure.is_some() => {
            ensure(
                client,
                channel_id,
                id,
                &args.title,
                &args.artifact_type,
                &content,
            )
            .await
        }
        result => result.map(|response| with_artifact_id(&response, &id.to_string())),
    }
}

/// Make "this artifact exists with this title and content" true after a
/// `create` lost to an existing identity (NIP-AR ensure flow).
async fn ensure(
    client: &BuzzClient,
    channel_id: Uuid,
    id: Uuid,
    title: &str,
    artifact_type: &str,
    content: &str,
) -> Result<String, CliError> {
    let (channel, id_text) = (channel_id.to_string(), id.to_string());
    for _ in 0..ENSURE_MAX_ATTEMPTS {
        let head = fetch_head(client, &id_text).await?.ok_or_else(|| {
            CliError::Conflict(format!(
                "artifact identity {id_text} is taken but its head is not readable by this key"
            ))
        })?;
        if head.channel != channel || head.artifact_type != artifact_type {
            return Err(CliError::Conflict(format!(
                "artifact {id_text} already exists with a different channel or type"
            )));
        }
        let outcome = if head.deleted {
            let revision = follow_up(
                &head,
                channel_id,
                id,
                ArtifactOp::Restore,
                Some(title),
                content,
            );
            publish(client, &revision).await
        } else if head.title.as_deref() == Some(title) && head.content == content {
            return Ok(with_artifact_id(
                &json!({"event_id": head.event_id, "accepted": true, "message": "unchanged"})
                    .to_string(),
                &id_text,
            ));
        } else {
            let revision = follow_up(
                &head,
                channel_id,
                id,
                ArtifactOp::Update,
                Some(title),
                content,
            );
            publish(client, &revision).await
        };
        match outcome {
            Err(CliError::Conflict(_)) => continue,
            result => return result.map(|response| with_artifact_id(&response, &id_text)),
        }
    }
    Err(CliError::Conflict(format!(
        "artifact {id_text} kept changing; gave up after {ENSURE_MAX_ATTEMPTS} attempts"
    )))
}

async fn update(
    client: &BuzzClient,
    channel: &str,
    id: &str,
    title: Option<String>,
    content: Option<String>,
    content_file: Option<String>,
) -> Result<String, CliError> {
    let channel_id = canonical_uuid(channel, "channel")?;
    let artifact_id = canonical_uuid(id, "artifact id")?;
    let content = read_content(content, content_file)?;
    if title.is_none() && content.is_none() {
        return Err(CliError::Usage(
            "nothing to update: pass --title, --content or --content-file".into(),
        ));
    }
    let head = fetch_live_head(client, channel, id).await?;
    let title = title.or_else(|| head.title.clone());
    let content = content.unwrap_or_else(|| head.content.clone());
    let revision = follow_up(
        &head,
        channel_id,
        artifact_id,
        ArtifactOp::Update,
        title.as_deref(),
        &content,
    );
    publish(client, &revision)
        .await
        .map(|response| with_artifact_id(&response, id))
}

async fn move_to(
    client: &BuzzClient,
    channel: &str,
    id: &str,
    to: &str,
    confirmed: bool,
) -> Result<String, CliError> {
    canonical_uuid(channel, "channel")?;
    let artifact_id = canonical_uuid(id, "artifact id")?;
    let destination = canonical_uuid(to, "--to")?;
    if channel == to {
        return Err(CliError::Usage(
            "--to must differ from --channel; use `update` to edit in place".into(),
        ));
    }
    let head = fetch_live_head(client, channel, id).await?;
    if !confirmed {
        return Err(CliError::Usage(describe_move(client, &head, to).await?));
    }
    let revision = ArtifactRevision {
        artifact_id,
        channel_id: destination,
        artifact_type: &head.artifact_type,
        op: ArtifactOp::Move,
        prev: Some(&head.event_id),
        root: None,
        title: head.title.as_deref(),
        tags: &head.tags,
        content: &head.content,
    };
    publish(client, &revision)
        .await
        .map(|response| with_artifact_id(&response, id))
}

/// The audience and payload a move would publish, shown before confirmation.
async fn describe_move(client: &BuzzClient, head: &Revision, to: &str) -> Result<String, CliError> {
    let events = query_events(client, json!({"kinds": [39000], "#d": [to], "limit": 1})).await?;
    let tags: Vec<&Vec<Value>> = events
        .first()
        .and_then(|e| e.get("tags")?.as_array())
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_array())
        .collect();
    let has_tag = |name: &str| {
        tags.iter()
            .any(|t| t.first().and_then(Value::as_str) == Some(name))
    };
    let tag = |name: &str| {
        tags.iter()
            .find(|t| t.first().and_then(Value::as_str) == Some(name))
            .and_then(|t| t.get(1)?.as_str().map(str::to_string))
    };
    let name = tag("name").ok_or_else(|| {
        CliError::NotFound(format!(
            "destination channel {to} not found or not readable"
        ))
    })?;
    let audience = if has_tag("private") {
        "private: only its members can read it"
    } else {
        "open: its audience can read it"
    };
    Ok(format!(
        "move not performed: it would publish {title:?} ({artifact_type}, {bytes} content bytes) into channel {name:?} ({to}), {audience}. Earlier revisions stay in the source channel. Re-run with --yes to confirm.",
        title = head.title.as_deref().unwrap_or(""),
        artifact_type = head.artifact_type,
        bytes = head.content.len(),
    ))
}

async fn delete(client: &BuzzClient, channel: &str, id: &str) -> Result<String, CliError> {
    let channel_id = canonical_uuid(channel, "channel")?;
    let artifact_id = canonical_uuid(id, "artifact id")?;
    let head = fetch_live_head(client, channel, id).await?;
    let revision = ArtifactRevision {
        artifact_id,
        channel_id,
        artifact_type: &head.artifact_type,
        op: ArtifactOp::Delete,
        prev: Some(&head.event_id),
        root: head.root.as_deref(),
        title: None,
        tags: &[],
        content: "",
    };
    publish(client, &revision)
        .await
        .map(|response| with_artifact_id(&response, id))
}

async fn restore(
    client: &BuzzClient,
    channel: &str,
    id: &str,
    title: &str,
    content: Option<String>,
    content_file: Option<String>,
) -> Result<String, CliError> {
    let channel_id = canonical_uuid(channel, "channel")?;
    let artifact_id = canonical_uuid(id, "artifact id")?;
    let content = read_content(content, content_file)?.unwrap_or_default();
    let head = match fetch_head(client, id).await? {
        Some(head) if head.channel == channel => head,
        _ => return Err(not_found(id, channel)),
    };
    if !head.deleted {
        return Err(CliError::Usage(format!("artifact {id} is not deleted")));
    }
    let revision = follow_up(
        &head,
        channel_id,
        artifact_id,
        ArtifactOp::Restore,
        Some(title),
        &content,
    );
    publish(client, &revision)
        .await
        .map(|response| with_artifact_id(&response, id))
}

/// Run an artifacts subcommand and return the text to print on success.
pub async fn run(
    cmd: ArtifactsCmd,
    client: &BuzzClient,
    format: &OutputFormat,
) -> Result<String, CliError> {
    match cmd {
        ArtifactsCmd::List {
            channel,
            artifact_type,
            limit,
        } => list(client, format, &channel, artifact_type.as_deref(), limit).await,
        ArtifactsCmd::Get { channel, id } => get(client, format, &channel, &id).await,
        ArtifactsCmd::Create {
            channel,
            artifact_type,
            title,
            content,
            content_file,
            ensure,
        } => {
            create(
                client,
                CreateArgs {
                    channel,
                    artifact_type,
                    title,
                    content,
                    content_file,
                    ensure,
                },
            )
            .await
        }
        ArtifactsCmd::Update {
            channel,
            id,
            title,
            content,
            content_file,
        } => update(client, &channel, &id, title, content, content_file).await,
        ArtifactsCmd::Move {
            channel,
            id,
            to,
            yes,
        } => move_to(client, &channel, &id, &to, yes).await,
        ArtifactsCmd::Delete { channel, id } => delete(client, &channel, &id).await,
        ArtifactsCmd::Restore {
            channel,
            id,
            title,
            content,
            content_file,
        } => restore(client, &channel, &id, &title, content, content_file).await,
    }
}

/// Run an artifacts subcommand and print its result.
pub async fn dispatch(
    cmd: ArtifactsCmd,
    client: &BuzzClient,
    format: &OutputFormat,
) -> Result<(), CliError> {
    println!("{}", run(cmd, client, format).await?);
    Ok(())
}

#[cfg(test)]
mod tests;
