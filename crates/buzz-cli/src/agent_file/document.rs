//! The agent document: the JSON contract between `buzz agents` and the
//! desktop app's `agent_read` and `agent_apply` provider methods.
//!
//! It holds one local agent's whole configuration, without runtime state
//! (timer arming times, watch cursors, history) and without secrets.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The only document version this CLI reads and writes.
pub const DOCUMENT_VERSION: u32 = 1;

/// One local agent's configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDocument {
    pub version: u32,
    /// Which agent this is. Absent in a document that creates an agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentRef>,
    /// The agent type (runtime), such as `claude-code`.
    #[serde(rename = "type")]
    pub agent_type: String,
    pub profile: Profile,
    /// The persona prompt.
    pub instructions: String,
    /// Settings that sync for this agent type, such as `model` and `effort`.
    pub settings: Map<String, Value>,
    pub attention: Attention,
    pub channels: Vec<ChannelMembership>,
}

/// The agent a document describes, as the app verified it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRef {
    /// Hex public key.
    pub pubkey: String,
    /// The relay URL of the agent's community.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community: Option<String>,
    /// Hex public key of the owner. Read-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

/// The public profile (kind 0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub about: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
}

/// The agent's Interests, event watches and timers (`agent-attention/v1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attention {
    /// Objects by slug: `interest/<id>` or `watch/<id>`.
    pub objects: BTreeMap<String, AttentionValue>,
}

/// One attention object, without runtime state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AttentionValue {
    Interest(Interest),
    Event(EventWatch),
    Timer(TimerWatch),
    /// A plugin-owned source watch. The file format can't express these.
    Source(Map<String, Value>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
}

/// Which channels an event watch looks at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Channels {
    All(AllChannels),
    Ids(Vec<String>),
}

/// The literal `"all"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AllChannels {
    All,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventWatch {
    pub interest_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifier: Option<Classifier>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Classifier {
    pub questions: BTreeMap<String, Question>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimerWatch {
    pub interest_id: String,
    pub prompt: String,
    pub enabled: bool,
    pub interval_secs: u64,
    /// `None` means the timer fires without limit.
    pub max_occurrences: Option<u64>,
    /// Unix seconds after which the timer stops, if any.
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

/// The agent's membership of one channel.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelMembership {
    pub channel: String,
    pub role: String,
}

/// What `agent_read` returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadResult {
    /// Opaque; changes whenever the agent's configuration changes.
    pub revision: String,
    pub document: AgentDocument,
}
