//! Normalized Slack export records shared by parsing, planning, and import.

use serde::Deserialize;

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
    pub(super) export_directory: String,
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

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

fn deserialize_null_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_null_default(deserializer)
}
