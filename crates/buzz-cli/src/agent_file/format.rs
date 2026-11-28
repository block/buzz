//! The `AGENT.md` front matter, as YAML.
//!
//! Field order here is the canonical order export writes. Every struct is
//! closed, so a misspelled field is an error, not a silent no-op.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::document::Channels;

pub const KIND: &str = "Agent";
/// The version export writes.
pub const VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontMatter {
    pub kind: String,
    pub version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub description: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<Identity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buzz: Option<BuzzBlock>,
    #[serde(default, skip_serializing_if = "Wake::is_empty")]
    pub wake: Wake,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    /// The agent's npub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buzz: Option<String>,
}

/// Settings that only Buzz reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuzzBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community: Option<String>,
    /// The agent type.
    pub runtime: String,
    /// Other synced settings of the agent type.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub settings: BTreeMap<String, serde_yaml::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<ChannelEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interests: Vec<InterestEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelEntry {
    pub channel: String,
    pub role: String,
}

/// Interest details that a `## When` section can't carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterestEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
    /// Only for an Interest that no wake feeds, so it has no section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wake {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timers: Vec<Timer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Event>,
}

impl Wake {
    fn is_empty(&self) -> bool {
        self.timers.is_empty() && self.events.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timer {
    pub id: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// An interval such as `30m` or `24h`.
    pub every: String,
    #[serde(default = "Max::once")]
    pub max: Max,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interest: Option<String>,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

/// How many times a timer fires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Max {
    Count(u64),
    Unbounded(UnboundedWord),
}

impl Max {
    fn once() -> Self {
        Max::Count(1)
    }
}

/// The literal `unbounded`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnboundedWord {
    Unbounded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub id: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub channels: Channels,
    pub kinds: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_authors: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<BTreeMap<String, Vec<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_reply: Option<bool>,
    /// Questions by name: a question's text, or the full question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifier: Option<BTreeMap<String, QuestionSpec>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QuestionSpec {
    Text(String),
    Full(FullQuestion),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FullQuestion {
    pub question: String,
    #[serde(rename = "true", default, skip_serializing_if = "Option::is_none")]
    pub if_true: Option<String>,
    #[serde(rename = "false", default, skip_serializing_if = "Option::is_none")]
    pub if_false: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
}
