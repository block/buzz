//! Slack workspace export parsing.
//!
//! A Slack import accepts one or more directories that collectively contain
//! `users.json` (or the Enterprise `org_users.json`), optional `channels.json`,
//! `groups.json`, `dms.json`, and `mpims.json`, plus one subdirectory per
//! conversation holding `YYYY-MM-DD.json` message arrays.
//!
//! Slackdump deliberately follows that native layout. It may emit public and
//! private conversations into separate export roots, so [`SlackExport`] can
//! merge multiple roots while retaining one workspace-wide conversation index.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::error::CliError;

/// A user record from `users.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct SlackUser {
    /// Slack user ID (`U...`).
    pub id: String,
    /// Login-style short name.
    #[serde(default)]
    pub name: String,
    /// Nested profile fields.
    #[serde(default)]
    pub profile: SlackUserProfile,
    /// Whether Slack marks this account as a bot.
    #[serde(default)]
    pub is_bot: bool,
    /// Whether this account is deactivated.
    #[serde(default)]
    pub deleted: bool,
}

/// The `profile` object nested in a user record.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SlackUserProfile {
    /// Preferred display name (may be empty).
    #[serde(default)]
    pub display_name: String,
    /// Full real name (may be empty).
    #[serde(default)]
    pub real_name: String,
}

impl SlackUser {
    /// Best available human-readable name: display name, then real name,
    /// then the login name, then the raw ID.
    pub fn best_name(&self) -> &str {
        if !self.profile.display_name.is_empty() {
            &self.profile.display_name
        } else if !self.profile.real_name.is_empty() {
            &self.profile.real_name
        } else if !self.name.is_empty() {
            &self.name
        } else {
            &self.id
        }
    }
}

/// Slack conversation class. Private channels map to private Buzz streams;
/// DMs and MPIMs open native Buzz DM conversations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SlackConversationKind {
    /// Public Slack channel.
    #[default]
    PublicChannel,
    /// Private Slack channel.
    PrivateChannel,
    /// One-to-one Slack direct message.
    DirectMessage,
    /// Slack multi-person direct message.
    GroupDirectMessage,
}

impl SlackConversationKind {
    /// Stable machine-readable name used by dry-run output and provenance.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PublicChannel => "public_channel",
            Self::PrivateChannel => "private_channel",
            Self::DirectMessage => "direct_message",
            Self::GroupDirectMessage => "group_direct_message",
        }
    }

    /// Whether this conversation must remain private in Buzz.
    pub fn is_private(self) -> bool {
        !matches!(self, Self::PublicChannel)
    }

    /// Buzz channel kind used for the imported conversation.
    pub fn buzz_channel_kind(self) -> buzz_sdk::ChannelKind {
        match self {
            Self::DirectMessage | Self::GroupDirectMessage => buzz_sdk::ChannelKind::Dm,
            Self::PublicChannel | Self::PrivateChannel => buzz_sdk::ChannelKind::Stream,
        }
    }
}

/// A conversation record normalized from `channels.json`, `groups.json`,
/// `dms.json`, or `mpims.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct SlackChannel {
    /// Slack conversation ID (`C...`, `G...`, or `D...`).
    pub id: String,
    /// User-facing channel name. DMs get a deterministic synthetic name.
    #[serde(default)]
    pub name: String,
    /// Whether the channel is archived in Slack.
    #[serde(default)]
    pub is_archived: bool,
    /// Channel topic.
    #[serde(default)]
    pub topic: SlackTopicLike,
    /// Channel purpose (description).
    #[serde(default)]
    pub purpose: SlackTopicLike,
    /// Slack member IDs allowed to see the conversation.
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub members: Vec<String>,
    /// Slack private-channel flag (used while normalizing `channels.json`).
    #[serde(default)]
    pub is_private: bool,
    /// Slack one-to-one direct-message flag.
    #[serde(default)]
    pub is_im: bool,
    /// Slack multi-person direct-message flag.
    #[serde(default)]
    pub is_mpim: bool,
    /// Normalized conversation class.
    #[serde(skip)]
    pub kind: SlackConversationKind,
    /// Export subdirectory holding daily message JSON.
    #[serde(skip)]
    export_directory: String,
}

/// Shared shape of Slack `topic` / `purpose` objects.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SlackTopicLike {
    /// The text value.
    #[serde(default)]
    pub value: String,
}

/// One message from a per-day export file.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SlackMessage {
    /// Message type — importable messages have `"message"`.
    #[serde(default, rename = "type")]
    pub msg_type: String,
    /// Slack subtype (`channel_join`, `bot_message`, ...); absent for
    /// ordinary user messages.
    #[serde(default)]
    pub subtype: Option<String>,
    /// Author user ID (`U...`); absent for some bot messages.
    #[serde(default)]
    pub user: Option<String>,
    /// Author bot ID (`B...`) for bot messages.
    #[serde(default)]
    pub bot_id: Option<String>,
    /// Display username for bot messages.
    #[serde(default)]
    pub username: Option<String>,
    /// Message text in Slack mrkdwn.
    #[serde(default)]
    pub text: String,
    /// Microsecond-precision timestamp string, e.g. `"1610000000.000200"`.
    /// Unique per channel — Slack's message primary key.
    pub ts: String,
    /// Thread root `ts` when this message is part of a thread.
    #[serde(default)]
    pub thread_ts: Option<String>,
    /// Emoji reactions on this message.
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub reactions: Vec<SlackReaction>,
    /// Attached files.
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub files: Vec<SlackFile>,
    /// Classic Slack message attachments (integration cards, unfurls, fields).
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub attachments: Vec<SlackAttachment>,
    /// Slack Block Kit / rich-text blocks.
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub blocks: Vec<serde_json::Value>,
}

/// One emoji reaction group on a message. Bot mode signs one reaction per
/// distinct emoji, so per-reactor identity cannot be reproduced; the source
/// users and count are still parsed for dry-run fidelity auditing.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SlackReaction {
    /// Emoji shortcode without colons (may carry `::skin-tone-N`).
    pub name: String,
    /// Slack user IDs returned for this reaction group. Slack may omit some
    /// users even when `count` is larger.
    #[serde(default)]
    pub users: Vec<String>,
    /// Authoritative aggregate count supplied by Slack.
    #[serde(default)]
    pub count: u64,
}

/// One file attachment stub.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SlackFile {
    /// File name.
    #[serde(default)]
    pub name: Option<String>,
    /// Human title.
    #[serde(default)]
    pub title: Option<String>,
    /// Slack-hosted permalink (requires Slack auth to fetch).
    #[serde(default)]
    pub permalink: Option<String>,
    /// Private download URL (requires Slack auth to fetch).
    #[serde(default)]
    pub url_private: Option<String>,
}

/// A classic Slack attachment. Slack integrations emit many optional fields;
/// these are the user-visible ones that can be represented faithfully in
/// Markdown without importing interactive application actions.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct SlackAttachment {
    /// Text shown before the attachment body.
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub pretext: String,
    /// Main attachment text.
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub text: String,
    /// Attachment title.
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub title: String,
    /// Optional link for the title.
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub title_link: String,
    /// Plain-text fallback supplied by the Slack integration.
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub fallback: String,
    /// Attachment author/integration label.
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub author_name: String,
    /// Optional author link.
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub author_link: String,
    /// Structured label/value fields.
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub fields: Vec<SlackAttachmentField>,
    /// Rich-text blocks nested in the attachment.
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub blocks: Vec<serde_json::Value>,
    /// Unfurl/image target.
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub image_url: String,
    /// Original unfurled URL.
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub original_url: String,
    /// Source URL for the unfurl.
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub from_url: String,
}

impl SlackAttachment {
    /// Whether the attachment has a non-interactive representation worth
    /// importing.
    pub fn has_content(&self) -> bool {
        [
            &self.pretext,
            &self.text,
            &self.title,
            &self.fallback,
            &self.author_name,
            &self.image_url,
            &self.original_url,
            &self.from_url,
        ]
        .iter()
        .any(|value| !value.trim().is_empty())
            || !self.fields.is_empty()
            || !self.blocks.is_empty()
    }
}

/// One title/value pair in a classic Slack attachment.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct SlackAttachmentField {
    /// Field heading.
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub title: String,
    /// Field value (Slack mrkdwn).
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub value: String,
}

impl SlackFile {
    /// Best display label for the attachment.
    pub fn label(&self) -> &str {
        match (&self.name, &self.title) {
            (Some(n), _) if !n.is_empty() => n,
            (_, Some(t)) if !t.is_empty() => t,
            _ => "attachment",
        }
    }

    /// Best link target, preferring the permalink.
    pub fn link(&self) -> Option<&str> {
        self.permalink
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(self.url_private.as_deref().filter(|s| !s.is_empty()))
    }
}

/// A loaded Slack export: user/channel indexes plus the directory root for
/// lazy per-channel message reads.
#[derive(Debug)]
pub struct SlackExport {
    /// Users indexed by Slack user ID.
    pub users: HashMap<String, SlackUser>,
    /// Conversations in source-file order, across all export roots.
    pub channels: Vec<SlackChannel>,
    roots: Vec<PathBuf>,
}

/// Per-conversation parsing/audit totals. Dry-run output aggregates these so
/// intentionally skipped Slack records are visible rather than disappearing
/// behind the importable-message count.
#[derive(Debug, Default)]
pub struct SlackMessageScan {
    /// Importable messages, deduplicated by Slack timestamp and sorted.
    pub messages: Vec<SlackMessage>,
    /// All JSON records encountered in daily files.
    pub records_seen: u64,
    /// Non-message records.
    pub non_message_records: u64,
    /// Membership/channel lifecycle events that Buzz materializes itself.
    pub system_records: u64,
    /// Edit/delete/tombstone records retained by Slack compliance exports.
    pub mutation_records: u64,
    /// Message-shaped records with no renderable text, attachment, file, or
    /// block content.
    pub contentless_records: u64,
    /// Repeated importable Slack timestamps collapsed deterministically.
    pub duplicate_records: u64,
}

impl SlackExport {
    /// Merge one or more Slack/Slackdump export roots.
    ///
    /// Exact duplicate metadata is tolerated so independently produced public
    /// and private exports can share user indexes. A reused Slack conversation
    /// ID with conflicting metadata, or two different IDs with the same target
    /// name, is rejected before any relay write.
    pub fn load_many(dirs: &[PathBuf]) -> Result<Self, CliError> {
        if dirs.is_empty() {
            return Err(CliError::Usage(
                "at least one --export-dir is required".into(),
            ));
        }

        let mut users: HashMap<String, SlackUser> = HashMap::new();
        let mut channels: Vec<SlackChannel> = Vec::new();
        let mut channel_indexes: HashMap<String, usize> = HashMap::new();
        let mut channel_names: HashMap<String, String> = HashMap::new();
        let mut roots = Vec::new();
        let mut saw_users_file = false;

        // Load the workspace-wide user index before normalizing any DMs. A
        // multi-root export may carry users.json in only one root, and the
        // root order must not change synthetic DM names or duplicate checks.
        for dir in dirs {
            if !dir.is_dir() {
                return Err(CliError::Usage(format!(
                    "--export-dir is not a directory: {}",
                    dir.display()
                )));
            }
            roots.push(dir.clone());

            let users_path = if dir.join("users.json").is_file() {
                Some(dir.join("users.json"))
            } else if dir.join("org_users.json").is_file() {
                Some(dir.join("org_users.json"))
            } else {
                None
            };
            if let Some(users_path) = users_path {
                saw_users_file = true;
                for user in read_json::<Vec<SlackUser>>(&users_path)? {
                    users.entry(user.id.clone()).or_insert(user);
                }
            }
        }

        if !saw_users_file {
            return Err(CliError::Usage(format!(
                "none of the export directories contains users.json or org_users.json: {}",
                dirs.iter()
                    .map(|dir| dir.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }

        for dir in dirs {
            let mut records = Vec::new();
            records.extend(
                read_optional_json::<Vec<SlackChannel>>(&dir.join("channels.json"))?
                    .into_iter()
                    .map(|channel| normalize_channel(channel, None, &users))
                    .collect::<Result<Vec<_>, _>>()?,
            );
            records.extend(
                read_optional_json::<Vec<SlackChannel>>(&dir.join("groups.json"))?
                    .into_iter()
                    .map(|channel| {
                        normalize_channel(
                            channel,
                            Some(SlackConversationKind::PrivateChannel),
                            &users,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            );
            records.extend(
                read_optional_json::<Vec<SlackChannel>>(&dir.join("dms.json"))?
                    .into_iter()
                    .map(|channel| {
                        normalize_channel(
                            channel,
                            Some(SlackConversationKind::DirectMessage),
                            &users,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            );
            records.extend(
                read_optional_json::<Vec<SlackChannel>>(&dir.join("mpims.json"))?
                    .into_iter()
                    .map(|channel| {
                        normalize_channel(
                            channel,
                            Some(SlackConversationKind::GroupDirectMessage),
                            &users,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            );

            for record in records {
                if let Some(index) = channel_indexes.get(&record.id).copied() {
                    merge_duplicate_conversation(&mut channels[index], &record)?;
                    continue;
                }
                if let Some(existing_id) = channel_names.get(&record.name) {
                    return Err(CliError::Usage(format!(
                        "Slack export maps conversation ids {existing_id} and {} to the same \
                         Buzz channel name {:?}; rename or filter one before importing",
                        record.id, record.name
                    )));
                }
                channel_names.insert(record.name.clone(), record.id.clone());
                channel_indexes.insert(record.id.clone(), channels.len());
                channels.push(record);
            }
        }

        if channels.is_empty() {
            return Err(CliError::Usage(
                "Slack export contains no conversations".into(),
            ));
        }

        Ok(Self {
            users,
            channels,
            roots,
        })
    }

    /// Read every day file for a channel, keeping only importable messages,
    /// deduplicated by `ts` and sorted chronologically.
    ///
    /// Returns `Ok(vec![])` with no error if the channel directory is
    /// missing (an empty channel exports no directory).
    pub fn channel_messages(&self, channel: &SlackChannel) -> Result<Vec<SlackMessage>, CliError> {
        Ok(self.channel_message_scan(channel)?.messages)
    }

    /// Read and classify every daily record for one conversation.
    pub fn channel_message_scan(
        &self,
        channel: &SlackChannel,
    ) -> Result<SlackMessageScan, CliError> {
        let mut day_files = Vec::new();
        for root in &self.roots {
            let dir = root.join(&channel.export_directory);
            if !dir.is_dir() {
                continue;
            }
            let entries = std::fs::read_dir(&dir)
                .map_err(|e| CliError::Other(format!("cannot read {}: {e}", dir.display())))?;
            for entry in entries {
                let path = entry
                    .map_err(|e| {
                        CliError::Other(format!("cannot read an entry in {}: {e}", dir.display()))
                    })?
                    .path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    day_files.push(path);
                }
            }
        }
        day_files.sort();

        let mut scan = SlackMessageScan::default();
        let mut by_ts: HashMap<String, (SlackTimestamp, SlackMessage)> = HashMap::new();
        for file in &day_files {
            let messages: Vec<SlackMessage> = read_json(file)?;
            for msg in messages {
                scan.records_seen += 1;
                match message_disposition(&msg) {
                    SlackMessageDisposition::NonMessage => {
                        scan.non_message_records += 1;
                        continue;
                    }
                    SlackMessageDisposition::System => {
                        scan.system_records += 1;
                        continue;
                    }
                    SlackMessageDisposition::Mutation => {
                        scan.mutation_records += 1;
                        continue;
                    }
                    SlackMessageDisposition::Contentless => {
                        scan.contentless_records += 1;
                        continue;
                    }
                    SlackMessageDisposition::Import => {}
                }
                let timestamp = parse_slack_ts(&msg.ts).ok_or_else(|| {
                    CliError::Usage(format!(
                        "malformed Slack ts {:?} in {}",
                        msg.ts,
                        file.display()
                    ))
                })?;
                match by_ts.entry(msg.ts.clone()) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert((timestamp, msg));
                    }
                    std::collections::hash_map::Entry::Occupied(entry) => {
                        if entry.get().1 != msg {
                            return Err(CliError::Usage(format!(
                                "Slack timestamp {} has conflicting message records across \
                                 export roots (latest conflict in {})",
                                msg.ts,
                                file.display()
                            )));
                        }
                        scan.duplicate_records += 1;
                    }
                }
            }
        }
        let mut messages: Vec<(SlackTimestamp, SlackMessage)> = by_ts.into_values().collect();
        messages.sort_by_key(|(timestamp, _)| *timestamp);
        scan.messages = messages.into_iter().map(|(_, message)| message).collect();
        Ok(scan)
    }

    /// Whether a Slack member can meaningfully map to a Buzz human account.
    /// Deactivated users and bot identities still retain message attribution,
    /// but are not treated as missing private-channel membership mappings.
    pub fn is_mappable_member(&self, slack_id: &str) -> bool {
        self.users
            .get(slack_id)
            .is_none_or(|user| !user.is_bot && !user.deleted)
            && slack_id != "USLACKBOT"
    }
}

fn normalize_channel(
    mut channel: SlackChannel,
    source_kind: Option<SlackConversationKind>,
    users: &HashMap<String, SlackUser>,
) -> Result<SlackChannel, CliError> {
    if channel.id.trim().is_empty() {
        return Err(CliError::Usage(
            "Slack conversation metadata contains an empty id".into(),
        ));
    }
    channel.members.sort();
    channel.members.dedup();
    channel.kind = source_kind.unwrap_or({
        if channel.is_im {
            SlackConversationKind::DirectMessage
        } else if channel.is_mpim {
            SlackConversationKind::GroupDirectMessage
        } else if channel.is_private {
            SlackConversationKind::PrivateChannel
        } else {
            SlackConversationKind::PublicChannel
        }
    });

    channel.export_directory = match channel.kind {
        SlackConversationKind::DirectMessage => channel.id.clone(),
        _ if !channel.name.trim().is_empty() => channel.name.trim().to_string(),
        SlackConversationKind::GroupDirectMessage => channel.id.clone(),
        SlackConversationKind::PublicChannel | SlackConversationKind::PrivateChannel => {
            return Err(CliError::Usage(format!(
                "Slack channel {} is missing its name",
                channel.id
            )))
        }
    };
    validate_export_directory(&channel.export_directory)?;

    if channel.kind == SlackConversationKind::DirectMessage {
        channel.name = direct_message_channel_name(&channel, users);
        if channel.purpose.value.is_empty() {
            channel.purpose.value = direct_message_purpose(&channel, users);
        }
    } else if channel.name.trim().is_empty() {
        channel.name = format!("mpdm-{}", channel.id.to_ascii_lowercase().replace('_', "-"));
    } else {
        channel.name = channel.name.trim().to_string();
    }
    Ok(channel)
}

fn validate_export_directory(value: &str) -> Result<(), CliError> {
    let path = Path::new(value);
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(CliError::Usage(format!(
            "unsafe Slack export conversation directory {value:?}"
        )));
    }
    Ok(())
}

fn direct_message_channel_name(
    channel: &SlackChannel,
    users: &HashMap<String, SlackUser>,
) -> String {
    let mut labels: Vec<String> = channel
        .members
        .iter()
        .map(|id| {
            users
                .get(id)
                .map(|user| {
                    if user.name.is_empty() {
                        user.best_name()
                    } else {
                        &user.name
                    }
                })
                .unwrap_or(id)
        })
        .map(slug_component)
        .filter(|label| !label.is_empty())
        .collect();
    labels.sort();
    labels.dedup();
    let participants = if labels.is_empty() {
        "unknown".to_string()
    } else {
        labels.join("-")
    };
    format!(
        "dm-{participants}-{}",
        channel.id.to_ascii_lowercase().replace('_', "-")
    )
}

fn direct_message_purpose(channel: &SlackChannel, users: &HashMap<String, SlackUser>) -> String {
    let labels: Vec<&str> = channel
        .members
        .iter()
        .map(|id| users.get(id).map(SlackUser::best_name).unwrap_or(id))
        .collect();
    if labels.is_empty() {
        "Imported Slack direct message".into()
    } else {
        format!(
            "Imported Slack direct message between {}",
            labels.join(", ")
        )
    }
}

fn slug_component(value: &str) -> String {
    let mut out = String::new();
    let mut previous_separator = false;
    for ch in value.chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
            previous_separator = false;
        } else if !previous_separator {
            out.push('-');
            previous_separator = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn merge_duplicate_conversation(
    existing: &mut SlackChannel,
    duplicate: &SlackChannel,
) -> Result<(), CliError> {
    if existing.name != duplicate.name
        || existing.kind != duplicate.kind
        || existing.export_directory != duplicate.export_directory
    {
        return Err(CliError::Usage(format!(
            "Slack conversation id {} has conflicting metadata across export directories",
            existing.id
        )));
    }
    existing.is_archived |= duplicate.is_archived;
    existing.members.extend(duplicate.members.iter().cloned());
    existing.members.sort();
    existing.members.dedup();
    if existing.topic.value.is_empty() {
        existing.topic = duplicate.topic.clone();
    }
    if existing.purpose.value.is_empty() {
        existing.purpose = duplicate.purpose.clone();
    }
    Ok(())
}

/// Whether a message carries content worth importing. System messages
/// (joins, renames, topic changes, ...) are excluded; the relay materializes
/// its own membership history.
#[cfg(test)]
pub fn is_importable(msg: &SlackMessage) -> bool {
    message_disposition(msg) == SlackMessageDisposition::Import
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlackMessageDisposition {
    Import,
    NonMessage,
    System,
    Mutation,
    Contentless,
}

fn message_disposition(msg: &SlackMessage) -> SlackMessageDisposition {
    if msg.msg_type != "message" {
        return SlackMessageDisposition::NonMessage;
    }
    if matches!(
        msg.subtype.as_deref(),
        Some("message_changed" | "message_deleted" | "tombstone")
    ) {
        return SlackMessageDisposition::Mutation;
    }
    if matches!(
        msg.subtype.as_deref(),
        Some(
            "channel_join"
                | "channel_leave"
                | "channel_topic"
                | "channel_purpose"
                | "channel_name"
                | "channel_archive"
                | "channel_unarchive"
                | "channel_convert_to_private"
                | "group_join"
                | "group_leave"
                | "group_topic"
                | "group_purpose"
                | "group_name"
                | "group_archive"
                | "group_unarchive"
                | "bot_add"
                | "bot_remove"
                | "bot_enable"
                | "bot_disable"
                | "app_conversation_join"
                | "pinned_item"
                | "unpinned_item"
                | "reminder_add"
                | "retention_threshold"
                | "tabbed_canvas_updated"
                | "channel_tab_added"
                | "channel_canvas_updated"
                | "automatic_ai_huddle_notes_enabled"
        )
    ) {
        return SlackMessageDisposition::System;
    }
    if msg.text.trim().is_empty()
        && msg.files.is_empty()
        && !msg.attachments.iter().any(SlackAttachment::has_content)
        && msg.blocks.is_empty()
    {
        return SlackMessageDisposition::Contentless;
    }
    SlackMessageDisposition::Import
}

/// Whole-second part of a Slack `ts` — becomes the Nostr `created_at`.
pub fn ts_seconds(ts: &str) -> Result<u64, CliError> {
    parse_slack_ts(ts)
        .map(|timestamp| timestamp.seconds)
        .ok_or_else(|| CliError::Other(format!("malformed Slack ts: {ts:?}")))
}

/// Exact, microsecond-resolution Slack timestamp used for chronological sort.
///
/// Slack exports normally use six fractional digits. Shorter fractions are
/// accepted for compatibility with hand-written fixtures and normalized by
/// right-padding; malformed and over-precise values are rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SlackTimestamp {
    seconds: u64,
    micros: u32,
}

fn parse_slack_ts(ts: &str) -> Option<SlackTimestamp> {
    let (seconds, fraction) = match ts.split_once('.') {
        Some((seconds, fraction)) => (seconds, Some(fraction)),
        None => (ts, None),
    };
    if seconds.is_empty() || !seconds.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let seconds = seconds.parse().ok()?;
    let micros = match fraction {
        None => 0,
        Some(fraction)
            if !fraction.is_empty()
                && fraction.len() <= 6
                && fraction.bytes().all(|b| b.is_ascii_digit()) =>
        {
            let value: u32 = fraction.parse().ok()?;
            value * 10_u32.pow(6 - fraction.len() as u32)
        }
        Some(_) => return None,
    };
    Some(SlackTimestamp { seconds, micros })
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, CliError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| CliError::Usage(format!("cannot read {}: {e}", path.display())))?;
    serde_json::from_str(&raw)
        .map_err(|e| CliError::Usage(format!("cannot parse {}: {e}", path.display())))
}

fn read_optional_json<T>(path: &Path) -> Result<T, CliError>
where
    T: serde::de::DeserializeOwned + Default,
{
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|e| CliError::Usage(format!("cannot parse {}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(CliError::Usage(format!(
            "cannot read {}: {e}",
            path.display()
        ))),
    }
}

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

fn deserialize_null_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_null_default(deserializer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(json: &str) -> SlackMessage {
        serde_json::from_str(json).expect("test message parses")
    }

    #[test]
    fn importable_filters_system_subtypes() {
        assert!(is_importable(&msg(
            r#"{"type":"message","user":"U1","text":"hi","ts":"1.000"}"#
        )));
        assert!(is_importable(&msg(
            r#"{"type":"message","subtype":"thread_broadcast","user":"U1","text":"hi","ts":"1.000"}"#
        )));
        assert!(is_importable(&msg(
            r#"{"type":"message","subtype":"bot_message","bot_id":"B1","text":"hi","ts":"1.000"}"#
        )));
        assert!(!is_importable(&msg(
            r#"{"type":"message","subtype":"channel_join","user":"U1","text":"<@U1> joined","ts":"1.000"}"#
        )));
        // Empty text with no files carries nothing to import.
        assert!(!is_importable(&msg(
            r#"{"type":"message","user":"U1","text":"","ts":"1.000"}"#
        )));
        // Empty text but a file attachment is importable.
        assert!(is_importable(&msg(
            r#"{"type":"message","user":"U1","text":"","ts":"1.000","files":[{"name":"a.png"}]}"#
        )));
        // Classic attachment-only integration posts retain their visible
        // fallback/text instead of disappearing.
        assert!(is_importable(&msg(
            r#"{"type":"message","subtype":"bot_message","bot_id":"B1","text":"","ts":"1.000",
                "attachments":[{"fallback":"build passed","title":"CI"}]}"#
        )));
        // Unknown future subtypes are accepted when they carry renderable
        // content; known membership/system events remain excluded.
        assert!(is_importable(&msg(
            r#"{"type":"message","subtype":"future_app_card","text":"useful","ts":"1.000"}"#
        )));
        // A contentless bot_message thread root (null user, no text, no files)
        // is filtered out — this is the real-export shape whose replies the
        // importer must promote to top-level rather than defer forever.
        assert!(!is_importable(&msg(
            r#"{"type":"message","subtype":"bot_message","bot_id":"B1","user":null,"text":"","ts":"1.000","thread_ts":"1.000"}"#
        )));
        assert!(!is_importable(&msg(
            r#"{"type":"message","subtype":"channel_join","user":"U1","text":"joined","ts":"1.000",
                "attachments":[{"fallback":"joined"}]}"#
        )));
    }

    #[test]
    fn ts_seconds_parses_whole_part() {
        assert_eq!(ts_seconds("1610000000.000200").expect("parses"), 1610000000);
        assert_eq!(ts_seconds("1610000000").expect("parses"), 1610000000);
        assert!(ts_seconds("not-a-ts").is_err());
        assert!(ts_seconds("1610000000.bad").is_err());
        assert!(ts_seconds("1610000000.").is_err());
        assert!(ts_seconds("1610000000.0000001").is_err());
    }

    #[test]
    fn slack_timestamps_sort_exactly() {
        let mut timestamps = [
            parse_slack_ts("1700000000.000010").expect("valid"),
            parse_slack_ts("1700000000.000002").expect("valid"),
            parse_slack_ts("1699999999.999999").expect("valid"),
        ];
        timestamps.sort();
        assert_eq!(timestamps[0].seconds, 1_699_999_999);
        assert_eq!(timestamps[1].micros, 2);
        assert_eq!(timestamps[2].micros, 10);
    }

    #[test]
    fn user_best_name_falls_back() {
        let user: SlackUser = serde_json::from_str(
            r#"{"id":"U1","name":"alice","profile":{"display_name":"","real_name":"Alice A"}}"#,
        )
        .expect("parses");
        assert_eq!(user.best_name(), "Alice A");
        let bare: SlackUser = serde_json::from_str(r#"{"id":"U2"}"#).expect("parses");
        assert_eq!(bare.best_name(), "U2");
    }

    #[test]
    fn loads_fixture_export_directory() {
        let dir = std::env::temp_dir().join(format!("buzz-slack-export-{}", std::process::id()));
        let general = dir.join("general");
        std::fs::create_dir_all(&general).expect("mkdir");
        std::fs::write(
            dir.join("channels.json"),
            r#"[{"id":"C1","name":"general","is_archived":false,
                 "topic":{"value":"the topic"},"purpose":{"value":"the purpose"}},
                {"id":"C2","name":"empty"}]"#,
        )
        .expect("write channels");
        std::fs::write(
            dir.join("users.json"),
            r#"[{"id":"U1","name":"alice","profile":{"display_name":"Alice"}}]"#,
        )
        .expect("write users");
        // Two day files, out of order on disk, with a system message, a
        // duplicate ts, and a threaded reply.
        std::fs::write(
            general.join("2024-01-02.json"),
            r#"[{"type":"message","user":"U1","text":"reply","ts":"200.000100","thread_ts":"100.000100"},
                {"type":"message","subtype":"channel_join","user":"U1","text":"joined","ts":"150.0"}]"#,
        )
        .expect("write day 2");
        std::fs::write(
            general.join("2024-01-01.json"),
            r#"[{"type":"message","user":"U1","text":"root","ts":"100.000100","thread_ts":"100.000100",
                 "reactions":[{"name":"+1","users":["U1"]}]},
                {"type":"message","user":"U1","text":"root","ts":"100.000100","thread_ts":"100.000100",
                 "reactions":[{"name":"+1","users":["U1"]}]}]"#,
        )
        .expect("write day 1");

        let export = SlackExport::load_many(std::slice::from_ref(&dir)).expect("load");
        assert_eq!(export.channels.len(), 2);
        assert_eq!(export.users["U1"].best_name(), "Alice");

        let scan = export
            .channel_message_scan(&export.channels[0])
            .expect("messages");
        assert_eq!(scan.records_seen, 4);
        assert_eq!(scan.system_records, 1);
        assert_eq!(scan.duplicate_records, 1);
        assert_eq!(scan.contentless_records, 0);
        let messages = scan.messages;
        // join filtered, duplicate ts collapsed, sorted oldest-first
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].ts, "100.000100");
        assert_eq!(messages[1].ts, "200.000100");
        assert_eq!(messages[0].reactions.len(), 1);

        // Missing channel directory is an empty channel, not an error.
        let empty = export
            .channel_messages(&export.channels[1])
            .expect("empty ok");
        assert!(empty.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_label_and_link() {
        let f: SlackFile =
            serde_json::from_str(r#"{"name":"a.png","permalink":"https://x/p"}"#).expect("parses");
        assert_eq!(f.label(), "a.png");
        assert_eq!(f.link(), Some("https://x/p"));
        let empty: SlackFile = serde_json::from_str(r#"{}"#).expect("parses");
        assert_eq!(empty.label(), "attachment");
        assert_eq!(empty.link(), None);
    }

    #[test]
    fn rejects_conflicting_duplicate_message_timestamps() {
        let dir = std::env::temp_dir().join(format!(
            "buzz-slack-message-conflict-{}",
            std::process::id()
        ));
        let general = dir.join("general");
        std::fs::create_dir_all(&general).expect("mkdir");
        std::fs::write(dir.join("users.json"), "[]").expect("users");
        std::fs::write(
            dir.join("channels.json"),
            r#"[{"id":"C1","name":"general"}]"#,
        )
        .expect("channels");
        std::fs::write(
            general.join("2024-01-01.json"),
            r#"[{"type":"message","user":"U1","text":"first","ts":"100.0"}]"#,
        )
        .expect("day one");
        std::fs::write(
            general.join("2024-01-02.json"),
            r#"[{"type":"message","user":"U1","text":"different","ts":"100.0"}]"#,
        )
        .expect("day two");

        let export = SlackExport::load_many(std::slice::from_ref(&dir)).expect("export");
        let error = export
            .channel_messages(&export.channels[0])
            .expect_err("conflicting duplicate must fail");
        assert!(error.to_string().contains("conflicting message records"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn merges_slackdump_public_private_dm_and_mpim_roots() {
        let base =
            std::env::temp_dir().join(format!("buzz-slackdump-roots-{}", std::process::id()));
        let public = base.join("public");
        let private = base.join("private");
        std::fs::create_dir_all(public.join("general")).expect("public dirs");
        std::fs::create_dir_all(private.join("secret")).expect("private dirs");
        std::fs::create_dir_all(private.join("group-secret")).expect("group dirs");
        std::fs::create_dir_all(private.join("D1")).expect("dm dirs");
        std::fs::create_dir_all(private.join("mpdm-alice--bob--carol-1")).expect("mpim dirs");

        std::fs::write(
            public.join("users.json"),
            r#"[
                {"id":"U1","name":"alice","profile":{"display_name":"Alice"}},
                {"id":"U2","name":"bob"},
                {"id":"U3","name":"carol"}
            ]"#,
        )
        .expect("users");
        std::fs::write(
            public.join("channels.json"),
            r#"[{"id":"C1","name":"general","members":["U1","U2"]}]"#,
        )
        .expect("public channels");
        std::fs::write(
            private.join("channels.json"),
            r#"[{"id":"C2","name":"secret","is_private":true,
                 "is_archived":true,"members":["U1","U2"]}]"#,
        )
        .expect("private channels");
        std::fs::write(
            private.join("groups.json"),
            r#"[{"id":"G1","name":"group-secret","members":["U2","U3"]}]"#,
        )
        .expect("groups");
        std::fs::write(
            private.join("dms.json"),
            r#"[{"id":"D1","members":["U1","U2"]}]"#,
        )
        .expect("dms");
        std::fs::write(
            private.join("mpims.json"),
            r#"[{"id":"M1","name":"mpdm-alice--bob--carol-1",
                 "members":["U1","U2","U3"]}]"#,
        )
        .expect("mpims");
        std::fs::write(
            private.join("D1").join("2024-01-01.json"),
            r#"[{"type":"message","user":"U1","text":"private","ts":"100.0"}]"#,
        )
        .expect("dm messages");

        // The user index exists only in the second root; normalization must be
        // root-order independent so the DM still gets stable human labels.
        let export =
            SlackExport::load_many(&[private.clone(), public.clone()]).expect("merged export");
        assert_eq!(export.channels.len(), 5);
        assert_eq!(
            export
                .channels
                .iter()
                .filter(|channel| channel.kind == SlackConversationKind::PublicChannel)
                .count(),
            1
        );
        assert_eq!(
            export
                .channels
                .iter()
                .filter(|channel| channel.kind == SlackConversationKind::PrivateChannel)
                .count(),
            2
        );
        assert_eq!(
            export
                .channels
                .iter()
                .filter(|channel| channel.kind == SlackConversationKind::DirectMessage)
                .count(),
            1
        );
        assert_eq!(
            export
                .channels
                .iter()
                .filter(|channel| channel.kind == SlackConversationKind::GroupDirectMessage)
                .count(),
            1
        );
        let dm = export
            .channels
            .iter()
            .find(|channel| channel.id == "D1")
            .expect("dm");
        assert_eq!(dm.name, "dm-alice-bob-d1");
        assert_eq!(dm.kind.buzz_channel_kind(), buzz_sdk::ChannelKind::Dm);
        assert!(dm.kind.is_private());
        assert_eq!(export.channel_messages(dm).expect("dm messages").len(), 1);
        let archived = export
            .channels
            .iter()
            .find(|channel| channel.id == "C2")
            .expect("archived private");
        assert!(archived.is_archived);
        assert!(archived.kind.is_private());

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn split_private_root_may_omit_channels_json() {
        let base = std::env::temp_dir().join(format!(
            "buzz-slackdump-optional-channels-{}",
            uuid::Uuid::new_v4()
        ));
        let public = base.join("public");
        let private = base.join("private");
        std::fs::create_dir_all(&public).expect("public dir");
        std::fs::create_dir_all(&private).expect("private dir");
        std::fs::write(public.join("users.json"), "[]").expect("users");
        std::fs::write(
            public.join("channels.json"),
            r#"[{"id":"C1","name":"general"}]"#,
        )
        .expect("public channels");
        std::fs::write(
            private.join("groups.json"),
            r#"[{"id":"G1","name":"secret"}]"#,
        )
        .expect("private groups");

        let export =
            SlackExport::load_many(&[private.clone(), public.clone()]).expect("merged export");
        assert_eq!(export.channels.len(), 2);
        assert!(export.channels.iter().any(|channel| channel.id == "C1"));
        assert!(export.channels.iter().any(|channel| channel.id == "G1"));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn rejects_conflicting_duplicate_conversation_names() {
        let dir =
            std::env::temp_dir().join(format!("buzz-slack-name-conflict-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("users.json"), "[]").expect("users");
        std::fs::write(
            dir.join("channels.json"),
            r#"[{"id":"C1","name":"same"},{"id":"C2","name":"same"}]"#,
        )
        .expect("channels");

        let error = SlackExport::load_many(std::slice::from_ref(&dir))
            .expect_err("duplicate target names must fail");
        assert!(error.to_string().contains("same Buzz channel name"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_unsafe_conversation_directory_names() {
        let dir =
            std::env::temp_dir().join(format!("buzz-slack-path-traversal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("users.json"), "[]").expect("users");
        std::fs::write(
            dir.join("channels.json"),
            r#"[{"id":"C1","name":"../outside"}]"#,
        )
        .expect("channels");

        let error = SlackExport::load_many(std::slice::from_ref(&dir))
            .expect_err("path traversal must fail");
        assert!(error.to_string().contains("unsafe Slack export"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
