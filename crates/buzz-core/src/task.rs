//! Buzz Tasks v1 wire contract.
//!
//! Task identity is carried in signed Nostr tags rather than duplicated in JSON:
//! exactly one `d` task UUID, `p` owner pubkey, `agent` author pubkey, `h`
//! channel UUID, and `e` source event with marker `source`. The relay performs
//! the database-backed owner, channel, and source-message checks at ingest.

use chrono::{DateTime, Utc};
use nostr::{Event, EventId, PublicKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::kind::{event_kind_u32, KIND_TASK_REQUESTED, KIND_TASK_RESOLVED, KIND_TASK_UPDATED};
use crate::CommunityId;

/// Maximum UTF-8 byte length of a task title.
pub const TASK_TITLE_MAX_BYTES: usize = 200;
/// Maximum UTF-8 byte length of a short task context.
pub const TASK_CONTEXT_MAX_BYTES: usize = 500;
/// Maximum UTF-8 byte length of an agent display name snapshot.
pub const TASK_AGENT_NAME_MAX_BYTES: usize = 100;
/// Maximum accepted lead of sourceUpdatedAt over the signed event timestamp.
pub const TASK_SOURCE_CLOCK_LEAD_SECS: i64 = 900;

/// Errors returned while parsing or building the Buzz Tasks v1 contract.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TaskContractError {
    /// The event kind is not one of the three Buzz Tasks v1 kinds.
    #[error("unsupported Buzz Task kind {0}")]
    UnsupportedKind(u32),
    /// A required signed tag is missing, duplicated, or malformed.
    #[error("invalid Buzz Task envelope: {0}")]
    InvalidEnvelope(String),
    /// The JSON content does not match the event kind's v1 payload.
    #[error("invalid Buzz Task payload: {0}")]
    InvalidPayload(String),
    /// The stored source event ID is not exactly 32 bytes.
    #[error("source event id must be exactly 32 bytes")]
    InvalidSourceEventId,
}

/// The action an owner must perform in the source Buzz thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    /// Reply to the agent.
    Reply,
    /// Approve or reject in the source thread.
    Approval,
    /// Select among choices in the source thread.
    Choice,
    /// Review material in the source thread.
    Review,
}

/// Owner-facing task priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    /// Low priority.
    Low,
    /// Normal priority.
    Medium,
    /// High priority.
    High,
}

/// Terminal task outcome selected by the source Buzz agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskResolution {
    /// The requested action happened in Buzz.
    Resolved,
    /// The agent withdrew the request.
    Withdrawn,
}

/// JSON content for kind 44300 (`task.requested.v1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskRequestedV1 {
    /// Action required in the source thread.
    pub task_type: TaskType,
    /// Short action-oriented title.
    pub title: String,
    /// Optional privacy-conscious context snapshot.
    pub context: Option<String>,
    /// Task priority.
    pub priority: TaskPriority,
    /// Optional absolute due timestamp in UTC.
    pub due_at: Option<DateTime<Utc>>,
    /// Agent display name snapshot for presentation.
    pub agent_name: String,
    /// Monotonic task-source version. Requested events must use version 1.
    pub source_version: i64,
    /// Source-side update time used for display and audit.
    pub source_updated_at: DateTime<Utc>,
}

/// JSON content for kind 44301 (`task.updated.v1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskUpdatedV1 {
    /// Action required in the source thread.
    pub task_type: TaskType,
    /// Short action-oriented title.
    pub title: String,
    /// Optional privacy-conscious context snapshot.
    pub context: Option<String>,
    /// Task priority.
    pub priority: TaskPriority,
    /// Optional absolute due timestamp in UTC.
    pub due_at: Option<DateTime<Utc>>,
    /// Agent display name snapshot for presentation.
    pub agent_name: String,
    /// Monotonic task-source version. Updated events must use version 2 or later.
    pub source_version: i64,
    /// Source-side update time used for display and audit.
    pub source_updated_at: DateTime<Utc>,
}

/// JSON content for kind 44302 (`task.resolved.v1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskResolvedV1 {
    /// Whether the source action completed or the request was withdrawn.
    pub resolution: TaskResolution,
    /// Monotonic task-source version. Resolved events must use version 2 or later.
    pub source_version: i64,
    /// Source-side update time used as the terminal time.
    pub source_updated_at: DateTime<Utc>,
}

/// Kind-discriminated Buzz Tasks v1 JSON content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskEventPayloadV1 {
    /// Kind 44300 content.
    Requested(TaskRequestedV1),
    /// Kind 44301 content.
    Updated(TaskUpdatedV1),
    /// Kind 44302 content.
    Resolved(TaskResolvedV1),
}

impl TaskEventPayloadV1 {
    /// Return the monotonic source version shared by all payload variants.
    pub fn source_version(&self) -> i64 {
        match self {
            Self::Requested(payload) => payload.source_version,
            Self::Updated(payload) => payload.source_version,
            Self::Resolved(payload) => payload.source_version,
        }
    }

    /// Return the source-side update timestamp shared by all payload variants.
    pub fn source_updated_at(&self) -> DateTime<Utc> {
        match self {
            Self::Requested(payload) => payload.source_updated_at,
            Self::Updated(payload) => payload.source_updated_at,
            Self::Resolved(payload) => payload.source_updated_at,
        }
    }
}

/// Fully parsed, cryptographically-bound Buzz Tasks v1 event envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEventV1 {
    /// Stable task UUID from the signed `d` tag.
    pub task_id: Uuid,
    /// Owner who must act, from the signed `p` tag.
    pub owner_pubkey: PublicKey,
    /// Agent that authored the event, bound to the signed `agent` tag.
    pub agent_pubkey: PublicKey,
    /// Source channel UUID from the signed `h` tag.
    pub channel_id: Uuid,
    /// Exact source message from the signed `e` tag marked `source`.
    pub source_event_id: EventId,
    /// Kind-discriminated JSON content.
    pub payload: TaskEventPayloadV1,
}

impl TaskEventV1 {
    /// Parse and validate the pure signed-envelope and JSON portions of a task event.
    ///
    /// Database-backed ownership, tenant, membership, and source-author checks
    /// remain relay responsibilities and are intentionally not represented here.
    pub fn parse(event: &Event) -> Result<Self, TaskContractError> {
        let mut task_ids = Vec::new();
        let mut owners = Vec::new();
        let mut agents = Vec::new();
        let mut channels = Vec::new();
        let mut sources = Vec::new();

        for tag in event.tags.iter() {
            let parts = tag.as_slice();
            match parts.first().map(String::as_str) {
                Some("d") if parts.len() == 2 => task_ids.push(parts[1].as_str()),
                Some("p") if parts.len() == 2 => owners.push(parts[1].as_str()),
                Some("agent") if parts.len() == 2 => agents.push(parts[1].as_str()),
                Some("h") if parts.len() == 2 => channels.push(parts[1].as_str()),
                Some("e") if parts.len() == 4 && parts[3] == "source" => {
                    sources.push(parts[1].as_str())
                }
                Some("d" | "p" | "agent" | "h" | "e") => {
                    return Err(TaskContractError::InvalidEnvelope(
                        "malformed identity tag".into(),
                    ));
                }
                _ => {}
            }
        }

        let task_id = parse_exactly_one(&task_ids, "d")?
            .parse::<Uuid>()
            .map_err(|_| TaskContractError::InvalidEnvelope("d tag must be a UUID".into()))?;
        let owner_pubkey = PublicKey::from_hex(parse_exactly_one(&owners, "p")?)
            .map_err(|_| TaskContractError::InvalidEnvelope("p tag must be a pubkey".into()))?;
        let agent_pubkey = PublicKey::from_hex(parse_exactly_one(&agents, "agent")?)
            .map_err(|_| TaskContractError::InvalidEnvelope("agent tag must be a pubkey".into()))?;
        if agent_pubkey != event.pubkey {
            return Err(TaskContractError::InvalidEnvelope(
                "agent tag must equal the event author".into(),
            ));
        }
        if agent_pubkey == owner_pubkey {
            return Err(TaskContractError::InvalidEnvelope(
                "task owner and agent must be different pubkeys".into(),
            ));
        }
        let channel_id = parse_exactly_one(&channels, "h")?
            .parse::<Uuid>()
            .map_err(|_| TaskContractError::InvalidEnvelope("h tag must be a UUID".into()))?;
        let source_event_id =
            EventId::from_hex(parse_exactly_one(&sources, "e/source")?).map_err(|_| {
                TaskContractError::InvalidEnvelope("source e tag must be an event id".into())
            })?;

        let kind = event_kind_u32(event);
        let payload = match kind {
            KIND_TASK_REQUESTED => {
                let payload: TaskRequestedV1 = parse_json(&event.content)?;
                validate_mutable_fields(
                    &payload.title,
                    payload.context.as_deref(),
                    &payload.agent_name,
                )?;
                if payload.source_version != 1 {
                    return Err(TaskContractError::InvalidPayload(
                        "requested sourceVersion must equal 1".into(),
                    ));
                }
                TaskEventPayloadV1::Requested(payload)
            }
            KIND_TASK_UPDATED => {
                let payload: TaskUpdatedV1 = parse_json(&event.content)?;
                validate_mutable_fields(
                    &payload.title,
                    payload.context.as_deref(),
                    &payload.agent_name,
                )?;
                if payload.source_version < 2 {
                    return Err(TaskContractError::InvalidPayload(
                        "updated sourceVersion must be at least 2".into(),
                    ));
                }
                TaskEventPayloadV1::Updated(payload)
            }
            KIND_TASK_RESOLVED => {
                let payload: TaskResolvedV1 = parse_json(&event.content)?;
                if payload.source_version < 2 {
                    return Err(TaskContractError::InvalidPayload(
                        "resolved sourceVersion must be at least 2".into(),
                    ));
                }
                TaskEventPayloadV1::Resolved(payload)
            }
            other => return Err(TaskContractError::UnsupportedKind(other)),
        };

        let event_created_at = DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
            .ok_or_else(|| {
                TaskContractError::InvalidPayload("invalid event creation timestamp".into())
            })?;
        if payload.source_updated_at()
            > event_created_at + chrono::Duration::seconds(TASK_SOURCE_CLOCK_LEAD_SECS)
        {
            return Err(TaskContractError::InvalidPayload(
                "sourceUpdatedAt is too far ahead of the signed event timestamp".into(),
            ));
        }

        Ok(Self {
            task_id,
            owner_pubkey,
            agent_pubkey,
            channel_id,
            source_event_id,
            payload,
        })
    }
}

fn parse_exactly_one<'a>(values: &'a [&str], tag: &str) -> Result<&'a str, TaskContractError> {
    match values {
        [value] => Ok(*value),
        _ => Err(TaskContractError::InvalidEnvelope(format!(
            "expected exactly one {tag} tag"
        ))),
    }
}

fn parse_json<T: for<'de> Deserialize<'de>>(content: &str) -> Result<T, TaskContractError> {
    serde_json::from_str(content)
        .map_err(|error| TaskContractError::InvalidPayload(error.to_string()))
}

fn validate_mutable_fields(
    title: &str,
    context: Option<&str>,
    agent_name: &str,
) -> Result<(), TaskContractError> {
    validate_display_text("title", title, TASK_TITLE_MAX_BYTES, false)?;
    validate_display_text("agentName", agent_name, TASK_AGENT_NAME_MAX_BYTES, false)?;
    if let Some(context) = context {
        validate_display_text("context", context, TASK_CONTEXT_MAX_BYTES, true)?;
        if context.lines().count() > 2 {
            return Err(TaskContractError::InvalidPayload(
                "context must contain at most two lines".into(),
            ));
        }
    }
    Ok(())
}

fn validate_display_text(
    field: &str,
    value: &str,
    max_bytes: usize,
    allow_newline: bool,
) -> Result<(), TaskContractError> {
    if value.is_empty() || value.trim() != value || value.len() > max_bytes {
        return Err(TaskContractError::InvalidPayload(format!(
            "{field} must be non-empty, trimmed, and at most {max_bytes} bytes"
        )));
    }
    if value
        .chars()
        .any(|character| character.is_control() && !(allow_newline && character == '\n'))
    {
        return Err(TaskContractError::InvalidPayload(format!(
            "{field} contains a control character"
        )));
    }
    Ok(())
}

/// Validated canonical target identity for an exact Buzz source message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskTarget {
    community_id: CommunityId,
    channel_id: Uuid,
    source_event_id: EventId,
}

impl TaskTarget {
    /// Build a target from database bytes, rejecting any non-Nostr event ID.
    pub fn from_bytes(
        community_id: CommunityId,
        channel_id: Uuid,
        source_event_id: &[u8],
    ) -> Result<Self, TaskContractError> {
        let bytes: [u8; 32] = source_event_id
            .try_into()
            .map_err(|_| TaskContractError::InvalidSourceEventId)?;
        Ok(Self {
            community_id,
            channel_id,
            source_event_id: EventId::from_byte_array(bytes),
        })
    }

    /// Server-resolved community identity. It is deliberately absent from the URL.
    pub fn community_id(&self) -> CommunityId {
        self.community_id
    }

    /// Source channel UUID.
    pub fn channel_id(&self) -> Uuid {
        self.channel_id
    }

    /// Exact source Nostr event ID.
    pub fn source_event_id(&self) -> EventId {
        self.source_event_id
    }

    /// Build the existing Buzz native navigation URL.
    ///
    /// This is navigation metadata only. Callers must obtain the target through
    /// an authenticated, owner- and channel-authorized Nostr read before using it.
    pub fn navigation_url(&self) -> String {
        format!(
            "buzz://message?channel={}&id={}",
            self.channel_id,
            self.source_event_id.to_hex()
        )
    }
}
