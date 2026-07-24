//! Slack workspace export parsing.
//!
//! A standard Slack export is a directory containing `channels.json`,
//! `users.json`, and one subdirectory per channel (named by channel name)
//! holding `YYYY-MM-DD.json` message arrays.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// A channel record from `channels.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct SlackChannel {
    /// Slack channel ID (`C...`).
    pub id: String,
    /// Channel name (also the export subdirectory name).
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
}

/// Shared shape of Slack `topic` / `purpose` objects.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SlackTopicLike {
    /// The text value.
    #[serde(default)]
    pub value: String,
}

/// One message from a per-day export file.
#[derive(Debug, Clone, Deserialize)]
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
    #[serde(default)]
    pub reactions: Vec<SlackReaction>,
    /// Attached files.
    #[serde(default)]
    pub files: Vec<SlackFile>,
}

/// One emoji reaction group on a message. Only the emoji name is used: bot
/// mode signs one reaction per distinct emoji, so per-reactor identity (the
/// export's `users` array) cannot be reproduced and is not parsed.
#[derive(Debug, Clone, Deserialize)]
pub struct SlackReaction {
    /// Emoji shortcode without colons (may carry `::skin-tone-N`).
    pub name: String,
}

/// One file attachment stub.
#[derive(Debug, Clone, Deserialize)]
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
pub struct SlackExport {
    /// Users indexed by Slack user ID.
    pub users: HashMap<String, SlackUser>,
    /// Channels in `channels.json` order.
    pub channels: Vec<SlackChannel>,
    root: PathBuf,
}

impl SlackExport {
    /// Load `channels.json` and `users.json` from an export directory.
    pub fn load(dir: &Path) -> Result<Self, CliError> {
        if !dir.is_dir() {
            return Err(CliError::Usage(format!(
                "--export-dir is not a directory: {}",
                dir.display()
            )));
        }
        let channels: Vec<SlackChannel> = read_json(&dir.join("channels.json"))?;
        let user_list: Vec<SlackUser> = read_json(&dir.join("users.json"))?;
        let users = user_list.into_iter().map(|u| (u.id.clone(), u)).collect();
        Ok(Self {
            users,
            channels,
            root: dir.to_path_buf(),
        })
    }

    /// Read every day file for a channel, keeping only importable messages,
    /// deduplicated by `ts` and sorted chronologically.
    ///
    /// Returns `Ok(vec![])` with no error if the channel directory is
    /// missing (an empty channel exports no directory).
    pub fn channel_messages(&self, channel_name: &str) -> Result<Vec<SlackMessage>, CliError> {
        let dir = self.root.join(channel_name);
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| CliError::Other(format!("cannot read {}: {e}", dir.display())))?;
        let mut day_files = Vec::new();
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
        day_files.sort();

        let mut by_ts: HashMap<String, (SlackTimestamp, SlackMessage)> = HashMap::new();
        for file in &day_files {
            let messages: Vec<SlackMessage> = read_json(file)?;
            for msg in messages {
                if is_importable(&msg) {
                    let timestamp = parse_slack_ts(&msg.ts).ok_or_else(|| {
                        CliError::Usage(format!(
                            "malformed Slack ts {:?} in {}",
                            msg.ts,
                            file.display()
                        ))
                    })?;
                    by_ts.entry(msg.ts.clone()).or_insert((timestamp, msg));
                }
            }
        }
        let mut messages: Vec<(SlackTimestamp, SlackMessage)> = by_ts.into_values().collect();
        messages.sort_by_key(|(timestamp, _)| *timestamp);
        Ok(messages.into_iter().map(|(_, message)| message).collect())
    }
}

/// Whether a message carries content worth importing. System messages
/// (joins, renames, topic changes, ...) are excluded; the relay materializes
/// its own membership history.
pub fn is_importable(msg: &SlackMessage) -> bool {
    if msg.msg_type != "message" {
        return false;
    }
    let subtype_ok = matches!(
        msg.subtype.as_deref(),
        None | Some("thread_broadcast")
            | Some("bot_message")
            | Some("file_share")
            | Some("me_message")
    );
    subtype_ok && (!msg.text.is_empty() || !msg.files.is_empty())
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
        // A contentless bot_message thread root (null user, no text, no files)
        // is filtered out — this is the real-export shape whose replies the
        // importer must promote to top-level rather than defer forever.
        assert!(!is_importable(&msg(
            r#"{"type":"message","subtype":"bot_message","bot_id":"B1","user":null,"text":"","ts":"1.000","thread_ts":"1.000"}"#
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
                 "topic":{"value":"the topic"},"purpose":{"value":"the purpose"}}]"#,
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
                {"type":"message","user":"U1","text":"dupe","ts":"100.000100"}]"#,
        )
        .expect("write day 1");

        let export = SlackExport::load(&dir).expect("load");
        assert_eq!(export.channels.len(), 1);
        assert_eq!(export.users["U1"].best_name(), "Alice");

        let messages = export.channel_messages("general").expect("messages");
        // join filtered, duplicate ts collapsed, sorted oldest-first
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].ts, "100.000100");
        assert_eq!(messages[1].ts, "200.000100");
        assert_eq!(messages[0].reactions.len(), 1);

        // Missing channel directory is an empty channel, not an error.
        let empty = export.channel_messages("nonexistent").expect("empty ok");
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
}
