//! Crash-safe preparation and replay of fully signed Buzz message events.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use nostr::{Event, EventId, JsonUtil, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::client::BuzzClient;
use crate::error::CliError;

const MAX_PREPARED_BYTES: u64 = 256 * 1024;
const MAX_INTENT_BYTES: u64 = 128 * 1024;

/// Replayable response material installed before the first network await.
/// It contains no signing key or owner authorization secret; startup supplies
/// and revalidates the current managed identity before using it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DurableReplyIntent {
    version: u8,
    execution_id: String,
    relay: String,
    agent_pubkey: String,
    owner_pubkey: String,
    auth_tag_hash: String,
    channel: String,
    content: String,
    reply_to: String,
    thread_root: String,
    mentions: Vec<String>,
    fingerprint: String,
}

pub(crate) struct DurableReplyIntentGuard {
    path: PathBuf,
    intent: DurableReplyIntent,
}

#[derive(Debug)]
pub(crate) struct PreparedCommandOutput {
    pub exit_code: i32,
    pub stdout: Option<Value>,
    pub stderr: Option<Value>,
}

#[derive(Debug)]
pub(crate) enum PreparedCommand<'a> {
    Prepare {
        channel: &'a str,
        content_flag: &'a str,
        reply_to: Option<&'a str>,
        thread_root: Option<&'a str>,
        execution_id: &'a str,
        mentions: &'a [String],
        out: &'a Path,
    },
    Publish {
        file: &'a Path,
    },
}

struct PrepareInput<'a> {
    channel: &'a str,
    content_flag: &'a str,
    reply_to: Option<&'a str>,
    thread_root: Option<&'a str>,
    execution_id: &'a str,
    mentions: &'a [String],
    out: &'a Path,
    stdin: &'a [u8],
}

#[derive(Clone, Copy)]
pub(crate) struct DurableReplyInput<'a> {
    pub channel: &'a str,
    pub content: &'a str,
    pub reply_to: &'a str,
    pub thread_root: &'a str,
    pub execution_id: &'a str,
    pub mentions: &'a [String],
    pub out: &'a Path,
}

impl DurableReplyIntent {
    fn from_input(
        client: &BuzzClient,
        input: &DurableReplyInput<'_>,
    ) -> Result<Self, PreparedFailure> {
        if input.content.is_empty() {
            return Err(PreparedFailure::User(
                "durable reply content is empty".into(),
            ));
        }
        if input.content.len() > 64 * 1024 {
            return Err(PreparedFailure::User(
                "durable reply content exceeds 64 KiB".into(),
            ));
        }
        if !input.out.is_absolute() {
            return Err(PreparedFailure::User(
                "durable reply path must be absolute".into(),
            ));
        }
        let execution_id = normalize_execution_id(input.execution_id)?;
        let expected_name = format!("buzz-outbox-{execution_id}.json");
        if input.out.file_name().and_then(|value| value.to_str()) != Some(expected_name.as_str()) {
            return Err(PreparedFailure::User(
                "durable reply path does not match its execution ID".into(),
            ));
        }
        let channel = Uuid::parse_str(input.channel)
            .map_err(|_| PreparedFailure::User("invalid channel UUID".into()))?
            .to_string();
        let reply_to = EventId::from_hex(input.reply_to)
            .map_err(|_| PreparedFailure::User("invalid reply event ID".into()))?
            .to_hex();
        let thread_root = EventId::from_hex(input.thread_root)
            .map_err(|_| PreparedFailure::User("invalid thread root event ID".into()))?
            .to_hex();
        let mentions = normalize_mentions(input.mentions)?;
        let owner_pubkey = client.auth_tag_owner_hex().ok_or_else(|| {
            PreparedFailure::User("verified NIP-OA owner auth is required".into())
        })?;
        let auth_tag_hash = client.auth_tag_hash().ok_or_else(|| {
            PreparedFailure::User("verified NIP-OA owner auth is required".into())
        })?;
        let fingerprint = fingerprint(
            &channel,
            &thread_root,
            &reply_to,
            &execution_id,
            input.content,
            &mentions,
        )?;
        Ok(Self {
            version: 1,
            execution_id,
            relay: client.relay_url().to_string(),
            agent_pubkey: client.keys().public_key().to_hex().to_ascii_lowercase(),
            owner_pubkey,
            auth_tag_hash,
            channel,
            content: input.content.to_string(),
            reply_to,
            thread_root,
            mentions,
            fingerprint,
        })
    }

    fn validate(&self, client: &BuzzClient) -> Result<(), PreparedFailure> {
        if self.version != 1
            || self.execution_id != normalize_execution_id(&self.execution_id)?
            || self.channel
                != Uuid::parse_str(&self.channel)
                    .map_err(|_| PreparedFailure::User("reply intent channel is invalid".into()))?
                    .to_string()
            || self.reply_to
                != EventId::from_hex(&self.reply_to)
                    .map_err(|_| PreparedFailure::User("reply intent parent is invalid".into()))?
                    .to_hex()
            || self.thread_root
                != EventId::from_hex(&self.thread_root)
                    .map_err(|_| PreparedFailure::User("reply intent root is invalid".into()))?
                    .to_hex()
            || self.content.is_empty()
            || self.content.len() > 64 * 1024
            || self.mentions != normalize_mentions(&self.mentions)?
            || self.relay != client.relay_url()
            || self.agent_pubkey != client.keys().public_key().to_hex().to_ascii_lowercase()
            || client.auth_tag_owner_hex().as_deref() != Some(self.owner_pubkey.as_str())
            || client.auth_tag_hash().as_deref() != Some(self.auth_tag_hash.as_str())
            || self.fingerprint
                != fingerprint(
                    &self.channel,
                    &self.thread_root,
                    &self.reply_to,
                    &self.execution_id,
                    &self.content,
                    &self.mentions,
                )?
        {
            return Err(PreparedFailure::User(
                "durable reply intent destination, identity, or content does not match".into(),
            ));
        }
        Ok(())
    }

    fn as_input<'a>(&'a self, out: &'a Path) -> DurableReplyInput<'a> {
        DurableReplyInput {
            channel: &self.channel,
            content: &self.content,
            reply_to: &self.reply_to,
            thread_root: &self.thread_root,
            execution_id: &self.execution_id,
            mentions: &self.mentions,
            out,
        }
    }
}

impl DurableReplyIntentGuard {
    pub(crate) fn begin(
        client: &BuzzClient,
        input: &DurableReplyInput<'_>,
    ) -> Result<Self, PreparedFailure> {
        let intent = DurableReplyIntent::from_input(client, input)?;
        let parent = input
            .out
            .parent()
            .ok_or_else(|| PreparedFailure::User("durable reply path has no parent".into()))?;
        validate_secure_parent(parent)?;
        let path = parent.join(format!("buzz-intent-{}.json", intent.execution_id));
        if path.exists() {
            let existing = read_intent(&path)?;
            existing.validate(client)?;
            if existing != intent {
                return Err(PreparedFailure::User(
                    "durable reply intent already exists with different response material".into(),
                ));
            }
        } else {
            let bytes = serde_json::to_vec(&intent)
                .map_err(|_| PreparedFailure::User("reply intent encoding failed".into()))?;
            if bytes.is_empty() || bytes.len() as u64 > MAX_INTENT_BYTES {
                return Err(PreparedFailure::User(
                    "durable reply intent exceeds size limit".into(),
                ));
            }
            install_record(&path, &bytes)?;
        }
        Ok(Self { path, intent })
    }

    /// Remove the response intent only after the exact reply is conclusively
    /// accepted or recovered as a relay duplicate.
    pub(crate) fn complete(self) -> Result<(), PreparedFailure> {
        remove_intent(&self.path, &self.intent)
    }
}

#[derive(Debug)]
pub(crate) enum PreparedFailure {
    User(String),
    Network,
    DeliveryUnknown(String),
    ManualReview {
        reason: &'static str,
        event_id: String,
    },
}

impl PreparedFailure {
    pub(crate) fn output(self) -> PreparedCommandOutput {
        match self {
            Self::User(message) => PreparedCommandOutput {
                exit_code: 1,
                stdout: None,
                stderr: Some(
                    json!({"error": "user_error", "retryable": false, "message": message}),
                ),
            },
            Self::Network => PreparedCommandOutput {
                exit_code: 2,
                stdout: None,
                stderr: Some(json!({"error": "network_error", "retryable": true})),
            },
            Self::DeliveryUnknown(event_id) => PreparedCommandOutput {
                exit_code: 2,
                stdout: None,
                stderr: Some(json!({
                    "error": "delivery_unknown",
                    "retryable": true,
                    "event_id": event_id,
                })),
            },
            Self::ManualReview { reason, event_id } => PreparedCommandOutput {
                exit_code: 1,
                stdout: None,
                stderr: Some(json!({
                    "error": "manual_review",
                    "retryable": false,
                    "reason": reason,
                    "event_id": event_id,
                })),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct MemberClaim {
    pubkey: String,
    role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChannelClaim {
    id: String,
    channel_type: String,
    owner_pubkey: String,
    agent_pubkey: String,
    members: Vec<MemberClaim>,
    membership_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplyClaim {
    root_event_id: String,
    parent_event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AuthClaim {
    agent_pubkey: String,
    owner_pubkey: String,
    auth_tag_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedRecord {
    version: u8,
    execution_id: String,
    relay: String,
    relay_info_hash: String,
    fingerprint: String,
    content_hash: String,
    mentions: Vec<String>,
    channel: ChannelClaim,
    reply: ReplyClaim,
    auth: AuthClaim,
    event: Value,
}

pub(crate) async fn execute(
    client: &BuzzClient,
    command: PreparedCommand<'_>,
    stdin: &[u8],
) -> PreparedCommandOutput {
    let result = match command {
        PreparedCommand::Prepare {
            channel,
            content_flag,
            reply_to,
            thread_root,
            execution_id,
            mentions,
            out,
        } => {
            prepare(
                client,
                PrepareInput {
                    channel,
                    content_flag,
                    reply_to,
                    thread_root,
                    execution_id,
                    mentions,
                    out,
                    stdin,
                },
            )
            .await
        }
        PreparedCommand::Publish { file } => publish(client, file).await,
    };
    match result {
        Ok(value) => PreparedCommandOutput {
            exit_code: 0,
            stdout: Some(value),
            stderr: None,
        },
        Err(error) => error.output(),
    }
}

pub(crate) async fn prepare_and_publish(
    client: &BuzzClient,
    input: DurableReplyInput<'_>,
) -> Result<Value, PreparedFailure> {
    prepare(
        client,
        PrepareInput {
            channel: input.channel,
            content_flag: "-",
            reply_to: Some(input.reply_to),
            thread_root: Some(input.thread_root),
            execution_id: input.execution_id,
            mentions: input.mentions,
            out: input.out,
            stdin: input.content.as_bytes(),
        },
    )
    .await?;
    publish(client, input.out).await
}

/// Replay every durable prepared reply before a managed harness subscribes to
/// new work. The dedicated outbox is fail-closed: unexpected entries or any
/// reply that cannot be conclusively reconciled prevent startup.
pub(crate) async fn replay_directory(
    client: &BuzzClient,
    directory: &Path,
) -> Result<usize, PreparedFailure> {
    if !directory.is_absolute() {
        return Err(PreparedFailure::User(
            "prepared outbox must be an absolute path".into(),
        ));
    }
    validate_secure_parent(directory)?;
    let paths = std::fs::read_dir(directory)
        .map_err(|_| PreparedFailure::User("prepared outbox is unavailable".into()))?
        .map(|entry| {
            entry
                .map(|value| value.path())
                .map_err(|_| PreparedFailure::User("prepared outbox scan failed".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut intents = Vec::new();
    let mut prepared = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| PreparedFailure::User("prepared outbox entry is invalid".into()))?;
        if name.starts_with("buzz-outbox-") && name.ends_with(".json.manual-review") {
            return Err(PreparedFailure::User(
                "prepared outbox contains a cancellation-ambiguous reply requiring manual review"
                    .into(),
            ));
        }
        // A crash before the no-clobber hard-link may leave this exact private
        // staging shape. It is not a committed reply. If the crash occurred
        // after the link, the canonical .json sibling is present and replayed.
        if is_exact_staging_name(name) {
            let metadata = std::fs::symlink_metadata(&path).map_err(|_| {
                PreparedFailure::User("prepared staging entry is unavailable".into())
            })?;
            if metadata.is_file() && !metadata.file_type().is_symlink() {
                continue;
            }
        }
        if let Some(execution_id) = name
            .strip_prefix("buzz-intent-")
            .and_then(|value| value.strip_suffix(".json"))
        {
            normalize_execution_id(execution_id)?;
            intents.push((execution_id.to_string(), path));
            continue;
        }
        let execution_id = name
            .strip_prefix("buzz-outbox-")
            .and_then(|value| value.strip_suffix(".json"))
            .ok_or_else(|| {
                PreparedFailure::User("prepared outbox contains an unexpected entry".into())
            })?;
        normalize_execution_id(execution_id)?;
        prepared.push((execution_id.to_string(), path));
    }
    intents.sort_by(|left, right| left.0.cmp(&right.0));
    prepared.sort_by(|left, right| left.0.cmp(&right.0));

    let mut reconciled_ids = std::collections::HashSet::new();
    for (execution_id, path) in intents {
        let intent = read_intent(&path)?;
        if intent.execution_id != execution_id {
            return Err(PreparedFailure::User(
                "reply intent filename does not match its execution ID".into(),
            ));
        }
        intent.validate(client)?;
        let out = directory.join(format!("buzz-outbox-{execution_id}.json"));
        prepare_and_publish(client, intent.as_input(&out)).await?;
        remove_intent(&path, &intent)?;
        reconciled_ids.insert(execution_id);
    }
    for (execution_id, path) in prepared {
        if reconciled_ids.contains(&execution_id) {
            continue;
        }
        let record = read_record(&path)?;
        if record.execution_id != execution_id {
            return Err(PreparedFailure::User(
                "prepared filename does not match its execution ID".into(),
            ));
        }
        publish(client, &path).await?;
        reconciled_ids.insert(execution_id);
    }
    Ok(reconciled_ids.len())
}

fn is_exact_staging_name(name: &str) -> bool {
    let Some(staging) = name.strip_prefix('.').and_then(|value| {
        value
            .strip_prefix("buzz-outbox-")
            .or_else(|| value.strip_prefix("buzz-intent-"))
            .and_then(|value| value.strip_suffix(".tmp"))
    }) else {
        return false;
    };
    let Some((execution_id, pid)) = staging.split_once(".json.") else {
        return false;
    };
    normalize_execution_id(execution_id).is_ok()
        && !pid.is_empty()
        && pid.bytes().all(|byte| byte.is_ascii_digit())
}

async fn prepare(client: &BuzzClient, input: PrepareInput<'_>) -> Result<Value, PreparedFailure> {
    let PrepareInput {
        channel,
        content_flag,
        reply_to,
        thread_root,
        execution_id,
        mentions,
        out,
        stdin,
    } = input;
    if content_flag != "-" {
        return Err(PreparedFailure::User(
            "prepared reply content must be supplied on stdin with --content -".into(),
        ));
    }
    if !out.is_absolute() {
        return Err(PreparedFailure::User(
            "--out must be an absolute path".into(),
        ));
    }
    if stdin.len() > 64 * 1024 {
        return Err(PreparedFailure::User("reply content exceeds 64 KiB".into()));
    }
    let content = std::str::from_utf8(stdin)
        .map_err(|_| PreparedFailure::User("reply content must be UTF-8".into()))?;
    if content.is_empty() {
        return Err(PreparedFailure::User("reply content is empty".into()));
    }
    let channel_id = Uuid::parse_str(channel)
        .map_err(|_| PreparedFailure::User("invalid channel UUID".into()))?;
    let reply_to = reply_to.ok_or_else(|| {
        PreparedFailure::User("--reply-to is required for prepared P0 replies".into())
    })?;
    let execution_id = normalize_execution_id(execution_id)?;
    let parent_id = EventId::from_hex(reply_to)
        .map_err(|_| PreparedFailure::User("invalid --reply-to event ID".into()))?;
    let normalized_mentions = normalize_mentions(mentions)?;

    let snapshot = fetch_snapshot(client, channel_id).await?;
    let parent = fetch_event(client, reply_to).await?;
    validate_parent(&parent, channel)?;
    let derived_root = thread_root_from_parent(&parent).unwrap_or_else(|| reply_to.to_string());
    let root = thread_root.unwrap_or(&derived_root);
    EventId::from_hex(root)
        .map_err(|_| PreparedFailure::User("invalid --thread-root event ID".into()))?;
    if root != derived_root {
        return Err(PreparedFailure::User(
            "--thread-root does not match the authoritative parent event".into(),
        ));
    }
    if root != reply_to {
        let root_event = fetch_event(client, root).await?;
        validate_parent(&root_event, channel)?;
    }

    let fingerprint = fingerprint(
        channel,
        root,
        reply_to,
        &execution_id,
        content,
        &normalized_mentions,
    )?;
    if out.exists() {
        let existing = read_record(out)?;
        if existing.fingerprint != fingerprint {
            return Err(PreparedFailure::User(
                "prepared record already exists with a different execution fingerprint".into(),
            ));
        }
        validate_record(client, &existing)?;
        let event_id = existing.event["id"]
            .as_str()
            .ok_or_else(|| PreparedFailure::User("prepared event is missing id".into()))?;
        return Ok(json!({
            "prepared": true,
            "event_id": event_id,
            "path": out,
            "adopted": true,
        }));
    }

    let mention_refs = normalized_mentions
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let thread = buzz_sdk::ThreadRef {
        root_event_id: EventId::from_hex(root)
            .map_err(|_| PreparedFailure::User("invalid thread root".into()))?,
        parent_event_id: parent_id,
    };
    let revision_tag = Tag::parse([
        "buzz_membership_revision",
        snapshot.channel.membership_revision.as_str(),
    ])
    .map_err(|_| PreparedFailure::User("invalid membership revision tag".into()))?;
    let builder = buzz_sdk::build_message(
        channel_id,
        content,
        Some(&thread),
        &mention_refs,
        false,
        &[],
    )
    .map_err(|error| PreparedFailure::User(format!("failed to build prepared message: {error}")))?
    .tags([revision_tag]);
    let event = client.sign_event(builder).map_err(map_client_failure)?;
    event
        .verify()
        .map_err(|_| PreparedFailure::User("prepared event signature is invalid".into()))?;
    client
        .verify_event_authorization(&event)
        .map_err(map_client_failure)?;
    let event = serde_json::to_value(event)
        .map_err(|_| PreparedFailure::User("failed to encode prepared event".into()))?;
    let record = PreparedRecord {
        version: 1,
        execution_id,
        relay: client.relay_url().to_string(),
        relay_info_hash: snapshot.relay_info_hash,
        fingerprint,
        content_hash: sha256_hex(content.as_bytes()),
        mentions: normalized_mentions,
        channel: snapshot.channel,
        reply: ReplyClaim {
            root_event_id: root.to_string(),
            parent_event_id: reply_to.to_string(),
        },
        auth: snapshot.auth,
        event,
    };
    validate_record(client, &record)?;
    let bytes = serde_json::to_vec(&record)
        .map_err(|_| PreparedFailure::User("failed to encode prepared record".into()))?;
    if bytes.len() as u64 > MAX_PREPARED_BYTES {
        return Err(PreparedFailure::User(
            "prepared record exceeds size limit".into(),
        ));
    }
    install_record(out, &bytes)?;
    let event_id = record.event["id"]
        .as_str()
        .ok_or_else(|| PreparedFailure::User("prepared event is missing id".into()))?;
    Ok(json!({
        "prepared": true,
        "event_id": event_id,
        "path": out,
        "adopted": false,
    }))
}

async fn publish(client: &BuzzClient, file: &Path) -> Result<Value, PreparedFailure> {
    let record = read_record(file)?;
    validate_record(client, &record)?;
    let event_id = record.event["id"]
        .as_str()
        .ok_or_else(|| PreparedFailure::User("prepared event is missing id".into()))?
        .to_string();

    match query_exact_event(client, &event_id).await {
        Ok(Some(existing)) => {
            if existing != record.event {
                return Err(PreparedFailure::ManualReview {
                    reason: "event_body_mismatch",
                    event_id,
                });
            }
            return Ok(json!({"accepted": true, "event_id": event_id, "duplicate": true}));
        }
        Ok(None) => {}
        Err(PreparedFailure::User(_)) => {
            return Err(PreparedFailure::ManualReview {
                reason: "query_unauthorized",
                event_id,
            });
        }
        Err(error) => return Err(error),
    }

    let channel_id = Uuid::parse_str(&record.channel.id)
        .map_err(|_| PreparedFailure::User("prepared channel is invalid".into()))?;
    let current = fetch_snapshot(client, channel_id).await?;
    if current.channel != record.channel
        || current.auth != record.auth
        || current.relay_info_hash != record.relay_info_hash
        || client.relay_url() != record.relay
    {
        return Err(PreparedFailure::ManualReview {
            reason: "destination_precondition_changed",
            event_id,
        });
    }
    let parent = fetch_event(client, &record.reply.parent_event_id).await?;
    validate_parent(&parent, &record.channel.id)?;
    if thread_root_from_parent(&parent).unwrap_or_else(|| record.reply.parent_event_id.clone())
        != record.reply.root_event_id
    {
        return Err(PreparedFailure::ManualReview {
            reason: "reply_anchor_changed",
            event_id,
        });
    }
    if record.reply.root_event_id != record.reply.parent_event_id {
        let root = fetch_event(client, &record.reply.root_event_id).await?;
        validate_parent(&root, &record.channel.id)?;
    }

    let event_bytes = serde_json::to_vec(&record.event)
        .map_err(|_| PreparedFailure::User("failed to encode prepared event".into()))?;
    match client.submit_prepared_event_bytes(event_bytes, 9).await {
        Ok(_) => Ok(json!({"accepted": true, "event_id": event_id, "duplicate": false})),
        Err(CliError::DeliveryUnknown(_)) => Err(PreparedFailure::DeliveryUnknown(event_id)),
        Err(error) => Err(map_client_failure(error)),
    }
}

struct Snapshot {
    channel: ChannelClaim,
    auth: AuthClaim,
    relay_info_hash: String,
}

async fn fetch_snapshot(client: &BuzzClient, channel: Uuid) -> Result<Snapshot, PreparedFailure> {
    let metadata = query_kind(client, 39000, channel).await?;
    let members_event = query_kind(client, 39002, channel).await?;
    if metadata.pubkey != members_event.pubkey {
        return Err(PreparedFailure::User(
            "channel metadata and membership have different authorities".into(),
        ));
    }
    let is_dm = metadata.tags.iter().any(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some("t")
            && values.get(1).map(String::as_str) == Some("dm"))
            || values.first().map(String::as_str) == Some("hidden")
    });
    if !is_dm {
        return Err(PreparedFailure::User(
            "prepared P0 replies require a DM channel".into(),
        ));
    }
    let agent_pubkey = client.keys().public_key().to_hex().to_ascii_lowercase();
    let owner_pubkey = client
        .auth_tag_owner_hex()
        .ok_or_else(|| PreparedFailure::User("verified NIP-OA owner auth is required".into()))?
        .to_ascii_lowercase();
    let mut members = members_event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("p")).then(|| MemberClaim {
                pubkey: values
                    .get(1)
                    .cloned()
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
                role: values.get(3).cloned().unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    members.sort_by(|left, right| left.pubkey.as_bytes().cmp(right.pubkey.as_bytes()));
    if members.len() != 2
        || !members
            .iter()
            .any(|member| member.pubkey == owner_pubkey && member.role == "member")
        || !members
            .iter()
            .any(|member| member.pubkey == agent_pubkey && member.role == "member")
    {
        return Err(PreparedFailure::User(
            "current channel membership is not the exact owner/agent DM set".into(),
        ));
    }
    let channel_text = channel.to_string();
    let revision = membership_revision(&channel_text, &members)?;
    let info = client
        .get_public("/info")
        .await
        .map_err(map_client_failure)?;
    let info_value: Value = serde_json::from_str(&info)
        .map_err(|_| PreparedFailure::User("relay info response is malformed".into()))?;
    let relay_info_hash = sha256_hex(canonical_json(&info_value)?.as_bytes());
    Ok(Snapshot {
        channel: ChannelClaim {
            id: channel_text,
            channel_type: "dm".into(),
            owner_pubkey: owner_pubkey.clone(),
            agent_pubkey: agent_pubkey.clone(),
            members,
            membership_revision: revision,
        },
        auth: AuthClaim {
            agent_pubkey,
            owner_pubkey,
            auth_tag_hash: client.auth_tag_hash().ok_or_else(|| {
                PreparedFailure::User("verified NIP-OA owner auth is required".into())
            })?,
        },
        relay_info_hash,
    })
}

async fn query_kind(
    client: &BuzzClient,
    kind: u16,
    channel: Uuid,
) -> Result<Event, PreparedFailure> {
    let raw = client
        .query(&json!({"kinds": [kind], "#d": [channel.to_string()], "limit": 1}))
        .await
        .map_err(map_client_failure)?;
    parse_one_event(&raw, "channel state")
}

async fn fetch_event(client: &BuzzClient, id: &str) -> Result<Event, PreparedFailure> {
    let raw = client
        .query(&json!({"ids": [id], "limit": 1}))
        .await
        .map_err(map_client_failure)?;
    parse_one_event(&raw, "event")
}

async fn query_exact_event(
    client: &BuzzClient,
    id: &str,
) -> Result<Option<Value>, PreparedFailure> {
    let raw = client
        .query(&json!({"ids": [id], "limit": 1}))
        .await
        .map_err(map_client_failure)?;
    let values: Vec<Value> = serde_json::from_str(&raw)
        .map_err(|_| PreparedFailure::User("event query response is malformed".into()))?;
    let Some(value) = values.into_iter().next() else {
        return Ok(None);
    };
    let event: Event = serde_json::from_value(value.clone())
        .map_err(|_| PreparedFailure::User("queried event is malformed".into()))?;
    event
        .verify()
        .map_err(|_| PreparedFailure::User("queried event signature is invalid".into()))?;
    Ok(Some(value))
}

fn parse_one_event(raw: &str, label: &str) -> Result<Event, PreparedFailure> {
    let events: Vec<Event> = serde_json::from_str(raw)
        .map_err(|_| PreparedFailure::User(format!("{label} query response is malformed")))?;
    let event = events
        .into_iter()
        .next()
        .ok_or_else(|| PreparedFailure::User(format!("{label} was not found")))?;
    event
        .verify()
        .map_err(|_| PreparedFailure::User(format!("{label} signature is invalid")))?;
    Ok(event)
}

fn validate_parent(parent: &Event, channel: &str) -> Result<(), PreparedFailure> {
    if parent.kind != Kind::Custom(9)
        || !parent.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.first().map(String::as_str) == Some("h")
                && values.get(1).map(String::as_str) == Some(channel)
        })
    {
        return Err(PreparedFailure::User(
            "reply parent does not belong to the prepared channel".into(),
        ));
    }
    Ok(())
}

fn thread_root_from_parent(parent: &Event) -> Option<String> {
    let mut root = None;
    let mut reply = None;
    for tag in parent.tags.iter() {
        let values = tag.as_slice();
        if values.first().map(String::as_str) != Some("e") || values.len() < 4 {
            continue;
        }
        match values.get(3).map(String::as_str) {
            Some("root") => root = values.get(1).cloned(),
            Some("reply") => reply = values.get(1).cloned(),
            _ => {}
        }
    }
    root.or(reply)
}

fn validate_record(client: &BuzzClient, record: &PreparedRecord) -> Result<(), PreparedFailure> {
    if record.version != 1 || record.relay != client.relay_url() {
        return Err(PreparedFailure::User(
            "prepared record version or destination does not match".into(),
        ));
    }
    let event: Event = Event::from_json(record.event.to_string())
        .map_err(|_| PreparedFailure::User("prepared event is malformed".into()))?;
    event
        .verify()
        .map_err(|_| PreparedFailure::User("prepared event signature is invalid".into()))?;
    client
        .verify_event_authorization(&event)
        .map_err(map_client_failure)?;
    if event.kind != Kind::Custom(9)
        || event.pubkey.to_hex().to_ascii_lowercase() != record.auth.agent_pubkey
        || sha256_hex(event.content.as_bytes()) != record.content_hash
    {
        return Err(PreparedFailure::User(
            "prepared event body does not match its frozen claims".into(),
        ));
    }
    let tags = event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .collect::<Vec<_>>();
    let tag_equals = |name: &str, value: &str| {
        tags.iter().any(|tag| {
            tag.first().map(String::as_str) == Some(name)
                && tag.get(1).map(String::as_str) == Some(value)
        })
    };
    let exact_thread_tag = |event_id: &str, marker: &str| {
        tags.iter().any(|tag| {
            tag.len() == 4
                && tag.first().map(String::as_str) == Some("e")
                && tag.get(1).map(String::as_str) == Some(event_id)
                && tag.get(2).map(String::as_str) == Some("")
                && tag.get(3).map(String::as_str) == Some(marker)
        })
    };
    let thread_tags_valid = if record.reply.root_event_id == record.reply.parent_event_id {
        exact_thread_tag(&record.reply.parent_event_id, "reply")
    } else {
        exact_thread_tag(&record.reply.root_event_id, "root")
            && exact_thread_tag(&record.reply.parent_event_id, "reply")
    };
    let mut event_mentions = tags
        .iter()
        .filter_map(|tag| {
            (tag.first().map(String::as_str) == Some("p"))
                .then(|| tag.get(1).cloned())
                .flatten()
        })
        .collect::<Vec<_>>();
    event_mentions.sort();
    let mut claimed_mentions = record.mentions.clone();
    claimed_mentions.sort();
    let canonical_execution_id = normalize_execution_id(&record.execution_id)?;
    if canonical_execution_id != record.execution_id {
        return Err(PreparedFailure::User(
            "prepared execution ID is not canonical".into(),
        ));
    }
    let fingerprint = fingerprint(
        &record.channel.id,
        &record.reply.root_event_id,
        &record.reply.parent_event_id,
        &record.execution_id,
        &event.content,
        &record.mentions,
    )?;
    let exact_members =
        record.channel.members.len() == 2
            && record.channel.members.iter().any(|member| {
                member.pubkey == record.channel.owner_pubkey && member.role == "member"
            })
            && record.channel.members.iter().any(|member| {
                member.pubkey == record.channel.agent_pubkey && member.role == "member"
            });
    if !tag_equals("h", &record.channel.id)
        || !tag_equals(
            "buzz_membership_revision",
            &record.channel.membership_revision,
        )
        || !thread_tags_valid
        || event_mentions != claimed_mentions
        || fingerprint != record.fingerprint
        || record.channel.channel_type != "dm"
        || !exact_members
        || record.channel.owner_pubkey != record.auth.owner_pubkey
        || record.channel.agent_pubkey != record.auth.agent_pubkey
        || client.keys().public_key().to_hex().to_ascii_lowercase() != record.auth.agent_pubkey
        || client.auth_tag_owner_hex().as_deref() != Some(record.auth.owner_pubkey.as_str())
        || client.auth_tag_hash().as_deref() != Some(record.auth.auth_tag_hash.as_str())
    {
        return Err(PreparedFailure::User(
            "prepared event destination or auth claims do not match".into(),
        ));
    }
    let expected_revision = membership_revision(&record.channel.id, &record.channel.members)?;
    if expected_revision != record.channel.membership_revision {
        return Err(PreparedFailure::User(
            "prepared membership revision is invalid".into(),
        ));
    }
    Ok(())
}

fn normalize_mentions(mentions: &[String]) -> Result<Vec<String>, PreparedFailure> {
    let mut normalized = Vec::new();
    for mention in mentions {
        let pubkey = PublicKey::parse(mention)
            .map_err(|_| PreparedFailure::User("invalid --mention pubkey".into()))?
            .to_hex()
            .to_ascii_lowercase();
        if !normalized.contains(&pubkey) {
            normalized.push(pubkey);
        }
    }
    Ok(normalized)
}

fn normalize_execution_id(value: &str) -> Result<String, PreparedFailure> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PreparedFailure::User(
            "--execution-id must be 64 hexadecimal characters".into(),
        ));
    }
    Ok(normalized)
}

fn fingerprint(
    channel: &str,
    root: &str,
    parent: &str,
    execution_id: &str,
    content: &str,
    mentions: &[String],
) -> Result<String, PreparedFailure> {
    let value = json!({
        "channel": channel,
        "root": root,
        "parent": parent,
        "executionId": execution_id,
        "contentHash": sha256_hex(content.as_bytes()),
        "mentions": mentions,
    });
    Ok(sha256_hex(canonical_json(&value)?.as_bytes()))
}

fn membership_revision(channel: &str, members: &[MemberClaim]) -> Result<String, PreparedFailure> {
    let value = json!({"version": 1, "channelId": channel, "members": members});
    Ok(format!(
        "v1:{}",
        sha256_hex(canonical_json(&value)?.as_bytes())
    ))
}

fn canonical_json(value: &Value) -> Result<String, PreparedFailure> {
    match value {
        Value::Null => Ok("null".into()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) if value.is_i64() || value.is_u64() => Ok(value.to_string()),
        Value::Number(_) => Err(PreparedFailure::User(
            "floating-point canonical JSON is not supported".into(),
        )),
        Value::String(value) => serde_json::to_string(value)
            .map_err(|_| PreparedFailure::User("failed to canonicalize string".into())),
        Value::Array(values) => Ok(format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Result<Vec<_>, _>>()?
                .join(",")
        )),
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
            let fields = keys
                .into_iter()
                .map(|key| {
                    Ok(format!(
                        "{}:{}",
                        serde_json::to_string(key).map_err(|_| PreparedFailure::User(
                            "failed to canonicalize key".into()
                        ))?,
                        canonical_json(&values[key])?
                    ))
                })
                .collect::<Result<Vec<_>, PreparedFailure>>()?;
            Ok(format!("{{{}}}", fields.join(",")))
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn read_intent(path: &Path) -> Result<DurableReplyIntent, PreparedFailure> {
    if !path.is_absolute() {
        return Err(PreparedFailure::User(
            "reply intent path must be absolute".into(),
        ));
    }
    let link_meta = std::fs::symlink_metadata(path)
        .map_err(|_| PreparedFailure::User("reply intent is unavailable".into()))?;
    if link_meta.file_type().is_symlink() || !link_meta.is_file() {
        return Err(PreparedFailure::User(
            "reply intent must be a regular non-symlink file".into(),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| PreparedFailure::User("reply intent has no parent".into()))?;
    validate_secure_parent(parent)?;
    let file = File::open(path)
        .map_err(|_| PreparedFailure::User("reply intent could not be opened".into()))?;
    let opened = file
        .metadata()
        .map_err(|_| PreparedFailure::User("reply intent metadata failed".into()))?;
    validate_open_file(&link_meta, &opened, parent)?;
    if opened.len() == 0 || opened.len() > MAX_INTENT_BYTES {
        return Err(PreparedFailure::User(
            "reply intent is empty, truncated, or oversized".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.take(MAX_INTENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PreparedFailure::User("reply intent read failed".into()))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| PreparedFailure::User("reply intent is malformed or truncated".into()))
}

fn remove_intent(path: &Path, expected: &DurableReplyIntent) -> Result<(), PreparedFailure> {
    if read_intent(path)? != *expected {
        return Err(PreparedFailure::User(
            "durable reply intent changed before completion".into(),
        ));
    }
    std::fs::remove_file(path)
        .map_err(|_| PreparedFailure::User("durable reply intent cleanup failed".into()))?;
    let parent = path
        .parent()
        .ok_or_else(|| PreparedFailure::User("reply intent has no parent".into()))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| PreparedFailure::User("reply intent directory fsync failed".into()))?;
    Ok(())
}

fn read_record(path: &Path) -> Result<PreparedRecord, PreparedFailure> {
    if !path.is_absolute() {
        return Err(PreparedFailure::User(
            "prepared path must be absolute".into(),
        ));
    }
    let link_meta = std::fs::symlink_metadata(path)
        .map_err(|_| PreparedFailure::User("prepared record is unavailable".into()))?;
    if link_meta.file_type().is_symlink() || !link_meta.is_file() {
        return Err(PreparedFailure::User(
            "prepared record must be a regular non-symlink file".into(),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| PreparedFailure::User("prepared path has no parent".into()))?;
    validate_secure_parent(parent)?;
    let file = File::open(path)
        .map_err(|_| PreparedFailure::User("prepared record could not be opened".into()))?;
    let opened = file
        .metadata()
        .map_err(|_| PreparedFailure::User("prepared record metadata failed".into()))?;
    validate_open_file(&link_meta, &opened, parent)?;
    if opened.len() == 0 || opened.len() > MAX_PREPARED_BYTES {
        return Err(PreparedFailure::User(
            "prepared record is empty, truncated, or oversized".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.take(MAX_PREPARED_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PreparedFailure::User("prepared record read failed".into()))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| PreparedFailure::User("prepared record is malformed or truncated".into()))
}

fn install_record(path: &Path, bytes: &[u8]) -> Result<(), PreparedFailure> {
    let parent = path
        .parent()
        .ok_or_else(|| PreparedFailure::User("prepared path has no parent".into()))?;
    validate_secure_parent(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| PreparedFailure::User("prepared filename is invalid".into()))?;
    let temp = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temp)
        .map_err(|_| PreparedFailure::User("could not create secure prepared temp file".into()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| PreparedFailure::User("prepared record fsync failed".into()))?;
    std::fs::hard_link(&temp, path)
        .map_err(|_| PreparedFailure::User("prepared record already exists".into()))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| PreparedFailure::User("prepared directory fsync failed".into()))?;
    std::fs::remove_file(&temp)
        .map_err(|_| PreparedFailure::User("prepared temp cleanup failed".into()))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| PreparedFailure::User("prepared directory fsync failed".into()))?;
    Ok(())
}

#[cfg(unix)]
fn validate_secure_parent(parent: &Path) -> Result<(), PreparedFailure> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = std::fs::symlink_metadata(parent)
        .map_err(|_| PreparedFailure::User("prepared parent is unavailable".into()))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.permissions().mode() & 0o777 != 0o700
        || metadata.nlink() == 0
        || metadata.uid() != nix::unistd::geteuid().as_raw()
    {
        return Err(PreparedFailure::User(
            "prepared parent must be an owned 0700 non-symlink directory".into(),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_secure_parent(parent: &Path) -> Result<(), PreparedFailure> {
    let metadata = std::fs::symlink_metadata(parent)
        .map_err(|_| PreparedFailure::User("prepared parent is unavailable".into()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PreparedFailure::User(
            "prepared parent must be a non-symlink directory".into(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_open_file(
    before: &std::fs::Metadata,
    opened: &std::fs::Metadata,
    parent: &Path,
) -> Result<(), PreparedFailure> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let parent_meta = std::fs::metadata(parent)
        .map_err(|_| PreparedFailure::User("prepared parent metadata failed".into()))?;
    if opened.permissions().mode() & 0o777 != 0o600
        || before.dev() != opened.dev()
        || before.ino() != opened.ino()
        || opened.nlink() == 0
        || opened.uid() != parent_meta.uid()
    {
        return Err(PreparedFailure::User(
            "prepared record owner, mode, or identity changed".into(),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_open_file(
    _before: &std::fs::Metadata,
    opened: &std::fs::Metadata,
    _parent: &Path,
) -> Result<(), PreparedFailure> {
    if !opened.is_file() {
        return Err(PreparedFailure::User(
            "prepared record must be a regular file".into(),
        ));
    }
    Ok(())
}

fn map_client_failure(error: CliError) -> PreparedFailure {
    match error {
        CliError::Network(_)
        | CliError::Relay {
            status: 429 | 500..=599,
            ..
        } => PreparedFailure::Network,
        CliError::Relay { status, .. } => PreparedFailure::User(format!(
            "relay rejected the prepared-event request with HTTP {status}"
        )),
        CliError::DeliveryUnknown(_) => PreparedFailure::Network,
        other => PreparedFailure::User(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_name_classifier_accepts_only_the_exact_crash_temp_shape() {
        let execution_id = "ab".repeat(32);
        assert!(is_exact_staging_name(&format!(
            ".buzz-outbox-{execution_id}.json.12345.tmp"
        )));
        assert!(is_exact_staging_name(&format!(
            ".buzz-intent-{execution_id}.json.12345.tmp"
        )));
        for invalid in [
            format!("buzz-outbox-{execution_id}.json.12345.tmp"),
            format!(".buzz-outbox-{execution_id}.json.pid.tmp"),
            format!(".buzz-outbox-{execution_id}.json.12345.tmp.extra"),
            ".buzz-outbox-short.json.12345.tmp".to_string(),
        ] {
            assert!(!is_exact_staging_name(&invalid), "accepted {invalid}");
        }
    }
}
