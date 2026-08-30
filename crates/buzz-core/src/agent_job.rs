//! Strict MotifOS agent-job projection wire codec.
//!
//! This module is pure and performs zero I/O. It validates projections only;
//! it never emits an event, starts an agent, or grants approval or execution
//! authority.

use std::collections::HashSet;
use std::fmt;

use chrono::{DateTime, Datelike, Utc};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{value::RawValue, Value};
use thiserror::Error;

use crate::kind::{
    KIND_JOB_ACCEPTED, KIND_JOB_CANCEL, KIND_JOB_ERROR, KIND_JOB_PROGRESS, KIND_JOB_REQUEST,
    KIND_JOB_RESULT,
};

/// Wire-format discriminator for MotifOS agent-job projections.
pub const FORMAT: &str = "motifos-agent-job-projection";
/// Current MotifOS agent-job projection schema version.
pub const VERSION: u16 = 1;
/// Maximum serialized projection content accepted on the wire.
pub const MAX_CONTENT_BYTES: usize = 16_384;
/// Largest integer represented exactly by interoperable JSON implementations.
pub const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;
/// Maximum UTF-8 bytes in a projection identity field.
pub const MAX_ID_BYTES: usize = 128;
/// Maximum UTF-8 bytes in a projection error code.
pub const MAX_ERROR_CODE_BYTES: usize = 64;
/// Maximum number of artifact or evidence references in one projection.
pub const MAX_REFERENCES: usize = 32;

const DUPLICATE_FIELD_SENTINEL: &str = "__buzz_agent_job_duplicate_field__";
const MAX_NESTED_JSON_CONTAINERS: usize = 126;

/// Canonical system that owns the projected task and run state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CanonicalSystem {
    /// MotifOS is the canonical system.
    #[serde(rename = "motifos")]
    MotifOs,
}

/// Buzz's strictly non-authoritative role in the projection contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuzzRole {
    /// Buzz may carry a projection but may not claim action authority.
    #[serde(rename = "projection_only")]
    ProjectionOnly,
}

/// Explicit canonical-system and Buzz-role authority boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionAuthority {
    /// System that owns the canonical task and run state.
    pub canonical_system: CanonicalSystem,
    /// Buzz's non-authoritative role.
    pub buzz_role: BuzzRole,
}

/// Sensitivity classification attached to a projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    /// Projection may be treated as public.
    Public,
    /// Projection is limited to internal consumers.
    Internal,
    /// Projection has restricted distribution.
    Restricted,
}

/// Canonical lifecycle state represented by a projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Work has been requested.
    Requested,
    /// The request has been accepted.
    Accepted,
    /// The accepted work is running.
    Running,
    /// The run completed successfully.
    Succeeded,
    /// Cancellation has been requested.
    CancellationRequested,
    /// The run failed.
    Failed,
}

/// State change asserted by a single projection record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateTransition {
    /// State immediately before this record, or `None` for an initial request.
    pub from: Option<JobState>,
    /// State asserted by this record.
    pub to: JobState,
}

/// Durable actor seat allowed to appear in a projection.
///
/// This closed set excludes capability slots, which must never deserialize as
/// actor identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatId {
    /// Wilson coordination seat.
    Wilson,
    /// Scout research seat.
    Scout,
    /// Bambu implementation seat.
    Bambu,
    /// Critic review seat.
    Critic,
    /// Ledger evidence seat.
    Ledger,
}

/// Durable actor seat and the host that produced the projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorRef {
    /// Durable actor seat identifier.
    pub seat_id: SeatId,
    /// Host identifier observed for the actor seat.
    pub host_id: String,
}

/// Content-minimized pointer to an artifact or evidence record.
///
/// Version 1 carries only an identifier and deliberately has no URI field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionReference {
    /// Source-held reference identifier.
    pub id: String,
}

/// Relationship between this projection and an earlier record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// This record corrects the target record.
    Corrects,
    /// This record supersedes the target record.
    Supersedes,
}

/// Typed relationship to an earlier projection record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordRelation {
    /// Relationship asserted by this record.
    pub kind: RelationKind,
    /// Portable idempotency key of the related record, excluding event-ID shapes.
    pub target_idempotency_key: String,
}

/// Versioned, content-minimized MotifOS agent-job projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentJobProjection {
    /// Wire-format discriminator; must equal [`FORMAT`].
    pub format: String,
    /// Schema version; must equal [`VERSION`].
    pub version: u16,
    /// Explicit projection-only authority boundary.
    pub authority: ProjectionAuthority,
    /// Canonical conversation identifier.
    pub conversation_id: String,
    /// Canonical mission identifier.
    pub mission_id: String,
    /// Canonical workstream identifier.
    pub workstream_id: String,
    /// Canonical attempt identifier.
    pub attempt_id: String,
    /// Canonical run identifier.
    pub run_id: String,
    /// Monotonic record sequence within the run.
    pub sequence: u64,
    /// Canonical task or run revision.
    pub revision: u64,
    /// Portable idempotency key for this record, excluding event-ID shapes.
    pub idempotency_key: String,
    /// Durable actor seat and observed host.
    pub actor: ActorRef,
    /// Canonical state transition carried by this record.
    pub transition: StateTransition,
    /// Canonical occurrence timestamp.
    pub occurred_at: DateTime<Utc>,
    /// Projection expiry timestamp.
    pub expires_at: DateTime<Utc>,
    /// Sensitivity classification.
    pub sensitivity: Sensitivity,
    /// Source-held artifact references.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ProjectionReference>,
    /// Source-held evidence references.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<ProjectionReference>,
    /// Optional correction or supersession relationship.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<RecordRelation>,
    /// Optional content-free outcome error code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

/// Agent-job event kind accepted by this projection codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    /// Job request event (`43001`).
    Request,
    /// Job acceptance event (`43002`).
    Accepted,
    /// Job progress event (`43003`).
    Progress,
    /// Job result event (`43004`).
    Result,
    /// Job cancellation request event (`43005`).
    Cancel,
    /// Job error event (`43006`).
    Error,
}

impl TryFrom<u32> for JobKind {
    type Error = AgentJobError;

    fn try_from(kind: u32) -> Result<Self, AgentJobError> {
        match kind {
            KIND_JOB_REQUEST => Ok(Self::Request),
            KIND_JOB_ACCEPTED => Ok(Self::Accepted),
            KIND_JOB_PROGRESS => Ok(Self::Progress),
            KIND_JOB_RESULT => Ok(Self::Result),
            KIND_JOB_CANCEL => Ok(Self::Cancel),
            KIND_JOB_ERROR => Ok(Self::Error),
            _ => Err(AgentJobError::UnsupportedKind),
        }
    }
}

/// Stable, content-free failures returned by the agent-job projection codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AgentJobError {
    /// The event kind is outside the reserved agent-job range.
    #[error("unsupported agent-job kind")]
    UnsupportedKind,
    /// The serialized content exceeds [`MAX_CONTENT_BYTES`].
    #[error("agent-job content is too large")]
    ContentTooLarge,
    /// The content is malformed JSON, has trailing data, or exceeds the
    /// 126-container nesting limit.
    #[error("invalid agent-job JSON")]
    InvalidJson,
    /// An object contains a duplicate member name.
    #[error("duplicate agent-job field")]
    DuplicateField,
    /// The JSON value does not match the closed wire schema.
    #[error("invalid agent-job schema")]
    InvalidSchema,
    /// The wire-format discriminator is not supported.
    #[error("unsupported agent-job format")]
    UnsupportedFormat,
    /// The wire schema version is not supported.
    #[error("unsupported agent-job version")]
    UnsupportedVersion,
    /// One or more identity fields, including the primary idempotency key, are invalid.
    #[error("invalid agent-job identity")]
    InvalidIdentity,
    /// The sequence or revision is invalid.
    #[error("invalid agent-job sequence")]
    InvalidSequence,
    /// The state transition is invalid.
    #[error("invalid agent-job transition")]
    InvalidTransition,
    /// The projection expiry is invalid.
    #[error("invalid agent-job expiry")]
    InvalidExpiry,
    /// A projection reference is invalid.
    #[error("invalid agent-job reference")]
    InvalidReference,
    /// A reference collection exceeds [`MAX_REFERENCES`].
    #[error("too many agent-job references")]
    TooManyReferences,
    /// The record relationship is invalid.
    #[error("invalid agent-job relation")]
    InvalidRelation,
    /// The projected outcome is invalid.
    #[error("invalid agent-job outcome")]
    InvalidOutcome,
}

impl AgentJobError {
    /// Return the stable machine-readable error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnsupportedKind => "unsupported_kind",
            Self::ContentTooLarge => "content_too_large",
            Self::InvalidJson => "invalid_json",
            Self::DuplicateField => "duplicate_field",
            Self::InvalidSchema => "invalid_schema",
            Self::UnsupportedFormat => "unsupported_format",
            Self::UnsupportedVersion => "unsupported_version",
            Self::InvalidIdentity => "invalid_identity",
            Self::InvalidSequence => "invalid_sequence",
            Self::InvalidTransition => "invalid_transition",
            Self::InvalidExpiry => "invalid_expiry",
            Self::InvalidReference => "invalid_reference",
            Self::TooManyReferences => "too_many_references",
            Self::InvalidRelation => "invalid_relation",
            Self::InvalidOutcome => "invalid_outcome",
        }
    }
}

/// Decode and validate one complete agent-job projection document.
///
/// This function performs no I/O and does not emit an event or grant authority.
pub fn decode(
    kind: u32,
    content: &[u8],
    now: DateTime<Utc>,
) -> Result<AgentJobProjection, AgentJobError> {
    let kind = JobKind::try_from(kind)?;
    if content.len() > MAX_CONTENT_BYTES {
        return Err(AgentJobError::ContentTooLarge);
    }

    let (value, raw_numeric_semantics) = parse_strict_agent_job_json(content)?;
    if raw_numeric_semantics.invalid_schema {
        return Err(AgentJobError::InvalidSchema);
    }
    let projection = serde_json::from_value(value).map_err(|_| AgentJobError::InvalidSchema)?;
    validate(&projection, kind, now, raw_numeric_semantics)?;
    Ok(projection)
}

/// Encode and validate an agent-job projection document.
///
/// This function performs no I/O and does not emit an event or grant authority.
pub fn encode(
    kind: u32,
    projection: &AgentJobProjection,
    now: DateTime<Utc>,
) -> Result<Vec<u8>, AgentJobError> {
    let kind = JobKind::try_from(kind)?;
    validate(projection, kind, now, RawNumericSemantics::default())?;
    if !has_portable_encoded_year(&projection.occurred_at)
        || !has_portable_encoded_year(&projection.expires_at)
    {
        return Err(AgentJobError::InvalidSchema);
    }
    let content = serde_json::to_vec(projection).map_err(|_| AgentJobError::InvalidSchema)?;
    if content.len() > MAX_CONTENT_BYTES {
        return Err(AgentJobError::ContentTooLarge);
    }
    Ok(content)
}

fn has_portable_encoded_year(timestamp: &DateTime<Utc>) -> bool {
    (0..=9999).contains(&timestamp.year())
}

#[derive(Clone, Copy, Debug, Default)]
struct RawNumericSemantics {
    unsupported_version: bool,
    invalid_sequence: bool,
    invalid_schema: bool,
}

struct ParsedRawValue {
    value: Value,
    invalid_schema: bool,
}

fn raw_integer_token(value: &RawValue) -> Option<&str> {
    let token = value.get();
    let digits = token.strip_prefix('-').unwrap_or(token);

    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())).then_some(token)
}

fn classify_root_numeric_value(
    field: &str,
    raw: &RawValue,
    semantics: &mut RawNumericSemantics,
    depth: usize,
) -> Result<ParsedRawValue, AgentJobError> {
    if let Some(token) = raw_integer_token(raw) {
        if field == "version" {
            semantics.unsupported_version = token != "1";
            return Ok(ParsedRawValue {
                value: Value::from(VERSION),
                invalid_schema: false,
            });
        }

        let number = token
            .parse::<u64>()
            .ok()
            .filter(|number| (1..=MAX_SAFE_INTEGER).contains(number));
        return Ok(ParsedRawValue {
            value: match number {
                Some(number) => Value::from(number),
                None => {
                    semantics.invalid_sequence = true;
                    Value::from(1_u64)
                }
            },
            invalid_schema: false,
        });
    }

    if raw
        .get()
        .as_bytes()
        .first()
        .is_some_and(|byte| matches!(byte, b'-' | b'0'..=b'9'))
    {
        // Non-integer JSON numbers are never schema-valid for these fields.
        // The flag rejects them before typed deserialization; Null is only a
        // range-insensitive placeholder while duplicate scanning completes.
        return Ok(ParsedRawValue {
            value: Value::Null,
            invalid_schema: true,
        });
    }

    parse_strict_raw_value(raw, depth)
}

fn validate(
    projection: &AgentJobProjection,
    kind: JobKind,
    now: DateTime<Utc>,
    raw_numeric_semantics: RawNumericSemantics,
) -> Result<(), AgentJobError> {
    if projection.format != FORMAT {
        return Err(AgentJobError::UnsupportedFormat);
    }
    if raw_numeric_semantics.unsupported_version || projection.version != VERSION {
        return Err(AgentJobError::UnsupportedVersion);
    }

    if !valid_identifier(&projection.conversation_id, MAX_ID_BYTES)
        || !valid_identifier(&projection.mission_id, MAX_ID_BYTES)
        || !valid_identifier(&projection.workstream_id, MAX_ID_BYTES)
        || !valid_identifier(&projection.attempt_id, MAX_ID_BYTES)
        || !valid_identifier(&projection.run_id, MAX_ID_BYTES)
        || !valid_portable_idempotency_key(&projection.idempotency_key)
        || !valid_identifier(&projection.actor.host_id, MAX_ID_BYTES)
    {
        return Err(AgentJobError::InvalidIdentity);
    }

    if raw_numeric_semantics.invalid_sequence
        || projection.sequence == 0
        || projection.sequence > MAX_SAFE_INTEGER
        || projection.revision == 0
        || projection.revision > MAX_SAFE_INTEGER
    {
        return Err(AgentJobError::InvalidSequence);
    }

    if !valid_transition(kind, &projection.transition) {
        return Err(AgentJobError::InvalidTransition);
    }

    if is_active(projection.transition.to)
        && (projection.expires_at <= projection.occurred_at || projection.expires_at <= now)
    {
        return Err(AgentJobError::InvalidExpiry);
    }

    let total_references = projection
        .artifacts
        .len()
        .saturating_add(projection.evidence.len());
    if total_references > MAX_REFERENCES {
        return Err(AgentJobError::TooManyReferences);
    }

    let mut reference_ids = HashSet::with_capacity(total_references);
    for reference in projection.artifacts.iter().chain(&projection.evidence) {
        if !validate_reference(reference) || !reference_ids.insert(reference.id.as_str()) {
            return Err(AgentJobError::InvalidReference);
        }
    }

    if let Some(relation) = &projection.relation {
        if !valid_portable_idempotency_key(&relation.target_idempotency_key)
            || relation.target_idempotency_key == projection.idempotency_key
        {
            return Err(AgentJobError::InvalidRelation);
        }
    }

    match kind {
        JobKind::Result => {
            if total_references == 0 || projection.error_code.is_some() {
                return Err(AgentJobError::InvalidOutcome);
            }
        }
        JobKind::Error => match projection.error_code.as_deref() {
            Some(code) if valid_identifier(code, MAX_ERROR_CODE_BYTES) => {}
            _ => return Err(AgentJobError::InvalidOutcome),
        },
        _ => {
            if projection.error_code.is_some() {
                return Err(AgentJobError::InvalidOutcome);
            }
        }
    }

    Ok(())
}

fn valid_identifier(value: &str, max_bytes: usize) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > max_bytes {
        return false;
    }

    matches!(bytes[0], b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9')
        && bytes[1..].iter().all(|byte| {
            matches!(
                *byte,
                b'A'..=b'Z'
                    | b'a'..=b'z'
                    | b'0'..=b'9'
                    | b'-'
                    | b'_'
                    | b'.'
                    | b':'
            )
        })
}

fn valid_portable_idempotency_key(value: &str) -> bool {
    valid_identifier(value, MAX_ID_BYTES)
        && !(value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn valid_transition(kind: JobKind, transition: &StateTransition) -> bool {
    matches!(
        (kind, transition.from, transition.to),
        (JobKind::Request, None, JobState::Requested)
            | (
                JobKind::Accepted,
                Some(JobState::Requested),
                JobState::Accepted
            )
            | (
                JobKind::Progress,
                Some(JobState::Accepted),
                JobState::Running
            )
            | (
                JobKind::Progress,
                Some(JobState::Running),
                JobState::Running
            )
            | (
                JobKind::Result,
                Some(JobState::Accepted),
                JobState::Succeeded
            )
            | (
                JobKind::Result,
                Some(JobState::Running),
                JobState::Succeeded
            )
            | (
                JobKind::Result,
                Some(JobState::CancellationRequested),
                JobState::Succeeded
            )
            | (
                JobKind::Cancel,
                Some(JobState::Requested),
                JobState::CancellationRequested
            )
            | (
                JobKind::Cancel,
                Some(JobState::Accepted),
                JobState::CancellationRequested
            )
            | (
                JobKind::Cancel,
                Some(JobState::Running),
                JobState::CancellationRequested
            )
            | (JobKind::Error, Some(JobState::Requested), JobState::Failed)
            | (JobKind::Error, Some(JobState::Accepted), JobState::Failed)
            | (JobKind::Error, Some(JobState::Running), JobState::Failed)
            | (
                JobKind::Error,
                Some(JobState::CancellationRequested),
                JobState::Failed
            )
    )
}

fn is_active(state: JobState) -> bool {
    matches!(
        state,
        JobState::Requested
            | JobState::Accepted
            | JobState::Running
            | JobState::CancellationRequested
    )
}

fn validate_reference(reference: &ProjectionReference) -> bool {
    valid_identifier(&reference.id, MAX_ID_BYTES)
}

#[derive(Clone, Copy)]
struct StrictRawObject {
    depth: usize,
}

impl<'de> Visitor<'de> for StrictRawObject {
    type Value = ParsedRawValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object with unique member names")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = serde_json::Map::new();
        let mut invalid_schema = false;
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(<A::Error as de::Error>::custom(DUPLICATE_FIELD_SENTINEL));
            }
            let raw = map.next_value::<&RawValue>()?;
            let parsed = parse_strict_raw_value(raw, self.depth + 1)
                .map_err(agent_job_json_error::<A::Error>)?;
            invalid_schema |= parsed.invalid_schema;
            values.insert(key, parsed.value);
        }
        Ok(ParsedRawValue {
            value: Value::Object(values),
            invalid_schema,
        })
    }
}

#[derive(Clone, Copy)]
struct StrictRawArray {
    depth: usize,
}

impl<'de> Visitor<'de> for StrictRawArray {
    type Value = ParsedRawValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON array")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
        let mut invalid_schema = false;
        while let Some(raw) = sequence.next_element::<&RawValue>()? {
            let parsed = parse_strict_raw_value(raw, self.depth + 1)
                .map_err(agent_job_json_error::<A::Error>)?;
            invalid_schema |= parsed.invalid_schema;
            values.push(parsed.value);
        }
        Ok(ParsedRawValue {
            value: Value::Array(values),
            invalid_schema,
        })
    }
}

struct StrictAgentJobObject;

impl<'de> Visitor<'de> for StrictAgentJobObject {
    type Value = (Value, RawNumericSemantics);

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("valid agent-job JSON with unique object member names")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = serde_json::Map::new();
        let mut semantics = RawNumericSemantics::default();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(<A::Error as de::Error>::custom(DUPLICATE_FIELD_SENTINEL));
            }
            let parsed = if matches!(key.as_str(), "version" | "sequence" | "revision") {
                let raw = map.next_value::<&RawValue>()?;
                classify_root_numeric_value(&key, raw, &mut semantics, 1)
                    .map_err(agent_job_json_error::<A::Error>)?
            } else {
                let raw = map.next_value::<&RawValue>()?;
                parse_strict_raw_value(raw, 1).map_err(agent_job_json_error::<A::Error>)?
            };
            semantics.invalid_schema |= parsed.invalid_schema;
            values.insert(key, parsed.value);
        }
        Ok((Value::Object(values), semantics))
    }
}

fn agent_job_json_error<E: de::Error>(error: AgentJobError) -> E {
    let message = match error {
        AgentJobError::DuplicateField => DUPLICATE_FIELD_SENTINEL,
        _ => "invalid JSON value",
    };
    E::custom(message)
}

fn parse_strict_raw_value(raw: &RawValue, depth: usize) -> Result<ParsedRawValue, AgentJobError> {
    match raw.get().as_bytes().first().copied() {
        Some(b'{') => {
            if depth > MAX_NESTED_JSON_CONTAINERS {
                return Err(AgentJobError::InvalidJson);
            }
            let mut deserializer = serde_json::Deserializer::from_str(raw.get());
            let value = deserializer
                .deserialize_map(StrictRawObject { depth })
                .map_err(classify_json_error)?;
            deserializer.end().map_err(|_| AgentJobError::InvalidJson)?;
            Ok(value)
        }
        Some(b'[') => {
            if depth > MAX_NESTED_JSON_CONTAINERS {
                return Err(AgentJobError::InvalidJson);
            }
            let mut deserializer = serde_json::Deserializer::from_str(raw.get());
            let value = deserializer
                .deserialize_seq(StrictRawArray { depth })
                .map_err(classify_json_error)?;
            deserializer.end().map_err(|_| AgentJobError::InvalidJson)?;
            Ok(value)
        }
        Some(b'-' | b'0'..=b'9') => {
            // RawValue already proved this is syntactically valid JSON. Keep
            // scanning for duplicate keys, but flag numbers Value cannot hold
            // so they are rejected before bounded typed deserialization.
            Ok(match serde_json::from_str(raw.get()) {
                Ok(value) => ParsedRawValue {
                    value,
                    invalid_schema: false,
                },
                Err(_) => ParsedRawValue {
                    value: Value::Null,
                    invalid_schema: true,
                },
            })
        }
        Some(_) => serde_json::from_str(raw.get())
            .map(|value| ParsedRawValue {
                value,
                invalid_schema: false,
            })
            .map_err(|_| AgentJobError::InvalidJson),
        None => Err(AgentJobError::InvalidJson),
    }
}

fn parse_strict_agent_job_json(
    content: &[u8],
) -> Result<(Value, RawNumericSemantics), AgentJobError> {
    let raw =
        serde_json::from_slice::<&RawValue>(content).map_err(|_| AgentJobError::InvalidJson)?;
    if !raw.get().starts_with('{') {
        return parse_strict_raw_value(raw, 0).map(|parsed| {
            (
                parsed.value,
                RawNumericSemantics {
                    invalid_schema: parsed.invalid_schema,
                    ..RawNumericSemantics::default()
                },
            )
        });
    }

    let mut deserializer = serde_json::Deserializer::from_str(raw.get());
    let value = deserializer
        .deserialize_map(StrictAgentJobObject)
        .map_err(classify_json_error)?;
    deserializer.end().map_err(|_| AgentJobError::InvalidJson)?;
    Ok(value)
}

fn classify_json_error(error: serde_json::Error) -> AgentJobError {
    if error.to_string().contains(DUPLICATE_FIELD_SENTINEL) {
        AgentJobError::DuplicateField
    } else {
        AgentJobError::InvalidJson
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::{
        KIND_JOB_ACCEPTED, KIND_JOB_CANCEL, KIND_JOB_ERROR, KIND_JOB_PROGRESS, KIND_JOB_REQUEST,
        KIND_JOB_RESULT,
    };
    use chrono::{DateTime, Duration, TimeZone, Utc};

    const PACKAGED_NIP: &str = include_str!("../NIP-AJ.md");

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 6, 20, 0, 0)
            .single()
            .expect("fixed UTC time is valid")
    }

    fn fixed_expiry() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 6, 21, 0, 0)
            .single()
            .expect("fixed UTC time is valid")
    }

    fn valid_projection() -> AgentJobProjection {
        AgentJobProjection {
            format: FORMAT.to_owned(),
            version: VERSION,
            authority: ProjectionAuthority {
                canonical_system: CanonicalSystem::MotifOs,
                buzz_role: BuzzRole::ProjectionOnly,
            },
            conversation_id: "conversation:alpha".to_owned(),
            mission_id: "mission:alpha".to_owned(),
            workstream_id: "workstream:contract".to_owned(),
            attempt_id: "attempt:1".to_owned(),
            run_id: "run:1".to_owned(),
            sequence: 1,
            revision: 1,
            idempotency_key: "job-request:attempt:1:1".to_owned(),
            actor: ActorRef {
                seat_id: SeatId::Wilson,
                host_id: "dashboard-local".to_owned(),
            },
            transition: StateTransition {
                from: None,
                to: JobState::Requested,
            },
            occurred_at: fixed_now(),
            expires_at: fixed_expiry(),
            sensitivity: Sensitivity::Internal,
            artifacts: Vec::new(),
            evidence: Vec::new(),
            relation: None,
            error_code: None,
        }
    }

    fn canonical_json() -> String {
        serde_json::to_string(&valid_projection()).expect("projection serializes")
    }

    fn projection_with_raw_value(field: &str, raw_value: &str) -> Vec<u8> {
        let canonical = canonical_json();
        let needle = format!("\"{field}\":1");
        let replacement = format!("\"{field}\":{raw_value}");
        let payload = canonical.replacen(&needle, &replacement, 1);
        assert_ne!(payload, canonical, "raw test field must be replaced");
        payload.into_bytes()
    }

    fn projection_with_raw_nullable(field: &str, raw_value: &str) -> Vec<u8> {
        let canonical = canonical_json();
        let payload = if field == "from" {
            canonical.replacen("\"from\":null", &format!("\"from\":{raw_value}"), 1)
        } else {
            let prefix = canonical
                .strip_suffix('}')
                .expect("projection JSON is an object");
            format!("{prefix},\"{field}\":{raw_value}}}")
        };
        assert_ne!(payload, canonical, "nullable test field must be replaced");
        payload.into_bytes()
    }

    fn projection_with_nested_unknown(depth: usize, object: bool) -> Vec<u8> {
        let mut nested = "null".to_owned();
        for _ in 0..depth {
            nested = if object {
                format!("{{\"nested\":{nested}}}")
            } else {
                format!("[{nested}]")
            };
        }
        let canonical = canonical_json();
        let prefix = canonical
            .strip_suffix('}')
            .expect("projection JSON is an object");
        format!("{prefix},\"nested_unknown\":{nested}}}").into_bytes()
    }

    fn assert_nip_mirror_matches(packaged: &str, canonical: &str) {
        assert!(
            packaged == canonical,
            "packaged NIP-AJ fixture drifted from the normative repository source"
        );
    }

    fn references(prefix: &str, count: usize) -> Vec<ProjectionReference> {
        (0..count)
            .map(|index| ProjectionReference {
                id: format!("{prefix}:{index}"),
            })
            .collect()
    }

    fn assert_encode_error(
        kind: u32,
        projection: &AgentJobProjection,
        expected: AgentJobError,
        expected_code: &str,
    ) {
        let error = encode(kind, projection, fixed_now()).unwrap_err();
        assert_eq!(error, expected);
        assert_eq!(error.code(), expected_code);
    }

    #[test]
    fn job_projection_round_trip() {
        let projection = valid_projection();

        let encoded = encode(KIND_JOB_REQUEST, &projection, fixed_now()).expect("encode");
        let decoded = decode(KIND_JOB_REQUEST, &encoded, fixed_now()).expect("decode");

        assert_eq!(decoded, projection);
    }

    #[test]
    fn rejects_unknown_or_duplicate_fields() {
        let mut root_unknown = serde_json::to_value(valid_projection()).expect("serialize");
        root_unknown
            .as_object_mut()
            .expect("projection is an object")
            .insert(
                "raw_prompt".to_owned(),
                serde_json::Value::String("do not echo this secret".to_owned()),
            );
        let root_unknown = serde_json::to_vec(&root_unknown).expect("serialize");
        let error = decode(KIND_JOB_REQUEST, &root_unknown, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidSchema);
        assert_eq!(error.code(), "invalid_schema");
        assert!(!error.to_string().contains("do not echo this secret"));

        let mut nested_unknown = serde_json::to_value(valid_projection()).expect("serialize");
        nested_unknown["actor"]
            .as_object_mut()
            .expect("actor is an object")
            .insert(
                "provider_token".to_owned(),
                serde_json::Value::String("secret-provider-token".to_owned()),
            );
        let nested_unknown = serde_json::to_vec(&nested_unknown).expect("serialize");
        let error = decode(KIND_JOB_REQUEST, &nested_unknown, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidSchema);
        assert_eq!(error.code(), "invalid_schema");

        let duplicate_root = canonical_json().replacen(
            r#""format":"motifos-agent-job-projection""#,
            r#""format":"motifos-agent-job-projection","format":"motifos-agent-job-projection""#,
            1,
        );
        let error = decode(KIND_JOB_REQUEST, duplicate_root.as_bytes(), fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::DuplicateField);
        assert_eq!(error.code(), "duplicate_field");

        let duplicate_nested = canonical_json().replacen(
            r#""host_id":"dashboard-local""#,
            r#""host_id":"dashboard-local","host_id":"dashboard-local""#,
            1,
        );
        let error = decode(KIND_JOB_REQUEST, duplicate_nested.as_bytes(), fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::DuplicateField);
        assert_eq!(error.code(), "duplicate_field");

        let mut projection = valid_projection();
        projection.artifacts.push(ProjectionReference {
            id: "artifact:one".to_owned(),
        });
        let duplicate_array_object = serde_json::to_string(&projection)
            .expect("projection serializes")
            .replacen(
                r#""id":"artifact:one""#,
                r#""id":"artifact:one","id":"artifact:one""#,
                1,
            );
        let error = decode(
            KIND_JOB_REQUEST,
            duplicate_array_object.as_bytes(),
            fixed_now(),
        )
        .unwrap_err();
        assert_eq!(error, AgentJobError::DuplicateField);
        assert_eq!(error.code(), "duplicate_field");
    }

    #[test]
    fn rejects_buzz_claiming_action_authority() {
        let mut value = serde_json::to_value(valid_projection()).expect("serialize");
        value["authority"]["buzz_role"] = serde_json::Value::String("action_authority".to_owned());
        let payload = serde_json::to_vec(&value).expect("serialize");

        let error = decode(KIND_JOB_REQUEST, &payload, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidSchema);
        assert_eq!(error.code(), "invalid_schema");
    }

    #[test]
    fn rejects_content_over_the_wire_limit() {
        let payload = vec![b' '; MAX_CONTENT_BYTES + 1];

        let error = decode(KIND_JOB_RESULT, &payload, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::ContentTooLarge);
        assert_eq!(error.code(), "content_too_large");
    }

    #[test]
    fn rejects_malformed_trailing_and_unsupported_payloads() {
        let error = decode(KIND_JOB_REQUEST, br#"{"format":"#, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidJson);
        assert_eq!(error.code(), "invalid_json");

        let trailing = format!("{} true", canonical_json());
        let error = decode(KIND_JOB_REQUEST, trailing.as_bytes(), fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidJson);
        assert_eq!(error.code(), "invalid_json");

        let mut unsupported_version = serde_json::to_value(valid_projection()).expect("serialize");
        unsupported_version["version"] = serde_json::Value::from(VERSION + 1);
        let unsupported_version = serde_json::to_vec(&unsupported_version).expect("serialize");
        let error = decode(KIND_JOB_REQUEST, &unsupported_version, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::UnsupportedVersion);
        assert_eq!(error.code(), "unsupported_version");

        let mut unsupported_format = serde_json::to_value(valid_projection()).expect("serialize");
        unsupported_format["format"] = serde_json::Value::String("other-format".to_owned());
        let unsupported_format = serde_json::to_vec(&unsupported_format).expect("serialize");
        let error = decode(KIND_JOB_REQUEST, &unsupported_format, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::UnsupportedFormat);
        assert_eq!(error.code(), "unsupported_format");

        let error = encode(42, &valid_projection(), fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::UnsupportedKind);
        assert_eq!(error.code(), "unsupported_kind");
    }

    #[test]
    fn decode_classifies_out_of_range_integer_versions_as_unsupported() {
        for version in [serde_json::json!(65_536), serde_json::json!(-1)] {
            let mut value = serde_json::to_value(valid_projection()).expect("serialize");
            value["version"] = version;
            let payload = serde_json::to_vec(&value).expect("serialize");

            let error = decode(KIND_JOB_REQUEST, &payload, fixed_now()).unwrap_err();
            assert_eq!(error, AgentJobError::UnsupportedVersion);
            assert_eq!(error.code(), "unsupported_version");
        }
    }

    #[test]
    fn decode_classifies_negative_sequence_and_revision_as_invalid_sequence() {
        for field in ["sequence", "revision"] {
            let mut value = serde_json::to_value(valid_projection()).expect("serialize");
            value[field] = serde_json::json!(-1);
            let payload = serde_json::to_vec(&value).expect("serialize");

            let error = decode(KIND_JOB_REQUEST, &payload, fixed_now()).unwrap_err();
            assert_eq!(error, AgentJobError::InvalidSequence, "field: {field}");
            assert_eq!(error.code(), "invalid_sequence", "field: {field}");
        }
    }

    #[test]
    fn decode_classifies_unrepresentable_integer_version_tokens_as_unsupported() {
        let hundreds_digit_integer = "9".repeat(400);
        for token in [
            "-0",
            "18446744073709551616",
            hundreds_digit_integer.as_str(),
        ] {
            let payload = projection_with_raw_value("version", token);

            let error = decode(KIND_JOB_REQUEST, &payload, fixed_now()).unwrap_err();
            assert_eq!(error, AgentJobError::UnsupportedVersion, "token: {token}");
            assert_eq!(error.code(), "unsupported_version", "token: {token}");
        }
    }

    #[test]
    fn decode_classifies_unrepresentable_sequence_tokens_as_invalid_sequence() {
        let hundreds_digit_integer = "9".repeat(400);
        for field in ["sequence", "revision"] {
            for token in [
                "-0",
                "18446744073709551616",
                hundreds_digit_integer.as_str(),
            ] {
                let payload = projection_with_raw_value(field, token);

                let error = decode(KIND_JOB_REQUEST, &payload, fixed_now()).unwrap_err();
                assert_eq!(
                    error,
                    AgentJobError::InvalidSequence,
                    "field: {field}, token: {token}"
                );
                assert_eq!(
                    error.code(),
                    "invalid_sequence",
                    "field: {field}, token: {token}"
                );
            }
        }
    }

    #[test]
    fn decode_keeps_floating_point_numeric_fields_as_invalid_schema() {
        for field in ["version", "sequence", "revision"] {
            let mut value = serde_json::to_value(valid_projection()).expect("serialize");
            value[field] = serde_json::json!(1.0);
            let payload = serde_json::to_vec(&value).expect("serialize");

            let error = decode(KIND_JOB_REQUEST, &payload, fixed_now()).unwrap_err();
            assert_eq!(error, AgentJobError::InvalidSchema, "field: {field}");
            assert_eq!(error.code(), "invalid_schema", "field: {field}");
        }
    }

    #[test]
    fn decode_keeps_decimal_and_exponent_numeric_tokens_as_invalid_schema() {
        for field in ["version", "sequence", "revision"] {
            for token in ["1.0", "1e0", "1e400", "1.0e400", "-1e400"] {
                let payload = projection_with_raw_value(field, token);

                let error = decode(KIND_JOB_REQUEST, &payload, fixed_now()).unwrap_err();
                assert_eq!(
                    error,
                    AgentJobError::InvalidSchema,
                    "field: {field}, token: {token}"
                );
                assert_eq!(
                    error.code(),
                    "invalid_schema",
                    "field: {field}, token: {token}"
                );
            }
        }
    }

    #[test]
    fn decode_keeps_missing_and_wrong_typed_numeric_fields_as_invalid_schema() {
        for field in ["version", "sequence", "revision"] {
            let mut missing = serde_json::to_value(valid_projection()).expect("serialize");
            missing
                .as_object_mut()
                .expect("projection is an object")
                .remove(field);
            let payload = serde_json::to_vec(&missing).expect("serialize");
            let error = decode(KIND_JOB_REQUEST, &payload, fixed_now()).unwrap_err();
            assert_eq!(
                error,
                AgentJobError::InvalidSchema,
                "missing field: {field}"
            );
            assert_eq!(error.code(), "invalid_schema", "missing field: {field}");

            for wrong_type in [
                serde_json::Value::String("1".to_owned()),
                serde_json::Value::Bool(true),
                serde_json::Value::Null,
            ] {
                let mut value = serde_json::to_value(valid_projection()).expect("serialize");
                value[field] = wrong_type;
                let payload = serde_json::to_vec(&value).expect("serialize");
                let error = decode(KIND_JOB_REQUEST, &payload, fixed_now()).unwrap_err();
                assert_eq!(
                    error,
                    AgentJobError::InvalidSchema,
                    "wrong-typed field: {field}"
                );
                assert_eq!(error.code(), "invalid_schema", "wrong-typed field: {field}");
            }
        }
    }

    #[test]
    fn decode_keeps_overflowing_numbers_in_non_target_positions_as_invalid_schema() {
        let canonical = canonical_json();
        let format = canonical.replacen(&format!("\"format\":\"{FORMAT}\""), "\"format\":1e400", 1);
        let nested_host =
            canonical.replacen("\"host_id\":\"dashboard-local\"", "\"host_id\":1e400", 1);
        let unknown_root = canonical.replacen('{', "{\"overflow\":1e400,", 1);

        for (case, payload) in [
            ("wrong-typed format", format),
            ("wrong-typed nested host", nested_host),
            ("unknown root field", unknown_root),
        ] {
            let error = decode(KIND_JOB_REQUEST, payload.as_bytes(), fixed_now()).unwrap_err();
            assert_eq!(error, AgentJobError::InvalidSchema, "case: {case}");
            assert_eq!(error.code(), "invalid_schema", "case: {case}");
        }
    }

    #[test]
    fn decode_keeps_nested_overflow_in_wrong_typed_numeric_fields_schema_classified() {
        let hundreds_digit_integer = "9".repeat(400);
        for field in ["version", "sequence", "revision"] {
            for raw_value in [
                format!("[{}]", hundreds_digit_integer),
                format!("{{\"nested\":{}}}", hundreds_digit_integer),
            ] {
                let payload = projection_with_raw_value(field, &raw_value);
                let error = decode(KIND_JOB_REQUEST, &payload, fixed_now()).unwrap_err();
                assert_eq!(error, AgentJobError::InvalidSchema, "field: {field}");
                assert_eq!(error.code(), "invalid_schema", "field: {field}");
            }

            let duplicate = format!("{{\"nested\":{},\"nested\":1}}", hundreds_digit_integer);
            let payload = projection_with_raw_value(field, &duplicate);
            let error = decode(KIND_JOB_REQUEST, &payload, fixed_now()).unwrap_err();
            assert_eq!(error, AgentJobError::DuplicateField, "field: {field}");
            assert_eq!(error.code(), "duplicate_field", "field: {field}");
        }
    }

    #[test]
    fn decode_rejects_unrepresentable_numbers_in_nullable_fields() {
        let hundreds_digit_integer = "9".repeat(400);
        let mut unexpectedly_decoded = Vec::new();
        for field in ["from", "relation", "error_code"] {
            for token in ["1e400", hundreds_digit_integer.as_str()] {
                let payload = projection_with_raw_nullable(field, token);
                match decode(KIND_JOB_REQUEST, &payload, fixed_now()) {
                    Ok(_) => unexpectedly_decoded.push((field, token.len())),
                    Err(error) => {
                        assert_eq!(error, AgentJobError::InvalidSchema, "field: {field}");
                        assert_eq!(error.code(), "invalid_schema", "field: {field}");
                    }
                }
            }
        }
        assert!(
            unexpectedly_decoded.is_empty(),
            "unrepresentable numbers decoded through nullable fields: {unexpectedly_decoded:?}"
        );
    }

    #[test]
    fn duplicate_fields_win_over_nullable_overflow_classification() {
        for field in ["from", "relation", "error_code"] {
            let canonical = canonical_json();
            let payload = if field == "from" {
                canonical.replacen("\"from\":null", "\"from\":1e400,\"from\":null", 1)
            } else {
                let prefix = canonical
                    .strip_suffix('}')
                    .expect("projection JSON is an object");
                format!("{prefix},\"{field}\":1e400,\"{field}\":null}}")
            };
            assert_ne!(payload, canonical, "duplicate test field must be replaced");

            let error = decode(KIND_JOB_REQUEST, payload.as_bytes(), fixed_now()).unwrap_err();
            assert_eq!(error, AgentJobError::DuplicateField, "field: {field}");
            assert_eq!(error.code(), "duplicate_field", "field: {field}");
        }

        let canonical = canonical_json();
        let prefix = canonical
            .strip_suffix('}')
            .expect("projection JSON is an object");
        let payload =
            format!("{prefix},\"overflow_then_duplicate\":[1e400,{{\"nested\":1,\"nested\":2}}]}}");
        let error = decode(KIND_JOB_REQUEST, payload.as_bytes(), fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::DuplicateField);
        assert_eq!(error.code(), "duplicate_field");
    }

    #[test]
    fn decode_preserves_strict_json_container_depth_boundary() {
        for object in [false, true] {
            let accepted = projection_with_nested_unknown(126, object);
            let error = decode(KIND_JOB_REQUEST, &accepted, fixed_now()).unwrap_err();
            assert_eq!(error, AgentJobError::InvalidSchema, "object: {object}");
            assert_eq!(error.code(), "invalid_schema", "object: {object}");

            let rejected = projection_with_nested_unknown(127, object);
            let error = decode(KIND_JOB_REQUEST, &rejected, fixed_now()).unwrap_err();
            assert_eq!(error, AgentJobError::InvalidJson, "object: {object}");
            assert_eq!(error.code(), "invalid_json", "object: {object}");
        }
    }

    #[test]
    fn decode_preserves_validation_precedence_for_classified_numbers() {
        let mut payload = canonical_json()
            .replacen(FORMAT, "another-agent-job-format", 1)
            .replacen("\"version\":1", "\"version\":18446744073709551616", 1)
            .replacen(
                "\"conversation_id\":\"conversation:alpha\"",
                "\"conversation_id\":\"\"",
                1,
            )
            .replacen("\"sequence\":1", "\"sequence\":-0", 1);

        let error = decode(KIND_JOB_REQUEST, payload.as_bytes(), fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::UnsupportedFormat);

        payload = payload.replacen("another-agent-job-format", FORMAT, 1);
        let error = decode(KIND_JOB_REQUEST, payload.as_bytes(), fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::UnsupportedVersion);

        payload = payload.replacen("18446744073709551616", "1", 1);
        let error = decode(KIND_JOB_REQUEST, payload.as_bytes(), fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidIdentity);

        payload = payload.replacen(
            "\"conversation_id\":\"\"",
            "\"conversation_id\":\"conversation:alpha\"",
            1,
        );
        let error = decode(KIND_JOB_REQUEST, payload.as_bytes(), fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidSequence);
    }

    #[test]
    fn rejects_invalid_kind_state_pair() {
        let mut projection = valid_projection();
        projection.transition = StateTransition {
            from: Some(JobState::Running),
            to: JobState::Succeeded,
        };

        let error = encode(KIND_JOB_REQUEST, &projection, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidTransition);
        assert_eq!(error.code(), "invalid_transition");
    }

    #[test]
    fn rejects_missing_or_noncanonical_identity() {
        let mut missing_conversation = valid_projection();
        missing_conversation.conversation_id.clear();
        let error = encode(KIND_JOB_REQUEST, &missing_conversation, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidIdentity);
        assert_eq!(error.code(), "invalid_identity");

        let mut noncanonical_mission = valid_projection();
        noncanonical_mission.mission_id = "/tmp/mission".to_owned();
        let error = encode(KIND_JOB_REQUEST, &noncanonical_mission, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidIdentity);
        assert_eq!(error.code(), "invalid_identity");

        let mut noncanonical_host = valid_projection();
        noncanonical_host.actor.host_id = "host name".to_owned();
        let error = encode(KIND_JOB_REQUEST, &noncanonical_host, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidIdentity);
        assert_eq!(error.code(), "invalid_identity");
    }

    #[test]
    fn rejects_capability_slot_as_actor_seat() {
        let mut value = serde_json::to_value(valid_projection()).expect("serialize");
        value["actor"]["seat_id"] = serde_json::Value::String("kimi-design-chair".to_owned());
        let payload = serde_json::to_vec(&value).expect("serialize");

        let error = decode(KIND_JOB_REQUEST, &payload, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidSchema);
        assert_eq!(error.code(), "invalid_schema");
    }

    #[test]
    fn correlates_attempt_run_and_workstream() {
        let projection = valid_projection();

        let encoded = encode(KIND_JOB_REQUEST, &projection, fixed_now()).expect("encode");
        let decoded = decode(KIND_JOB_REQUEST, &encoded, fixed_now()).expect("decode");

        assert_eq!(decoded.workstream_id, "workstream:contract");
        assert_eq!(decoded.attempt_id, "attempt:1");
        assert_eq!(decoded.run_id, "run:1");
        assert_eq!(decoded.idempotency_key, "job-request:attempt:1:1");
    }

    #[test]
    fn primary_idempotency_key_uses_portable_domain() {
        let mut opaque_sixty_four_byte_key = valid_projection();
        opaque_sixty_four_byte_key.idempotency_key = format!("g{}", "a".repeat(63));
        encode(KIND_JOB_REQUEST, &opaque_sixty_four_byte_key, fixed_now())
            .expect("non-hex opaque primary idempotency key encodes");

        let mut lowercase_event_id_shape = valid_projection();
        lowercase_event_id_shape.idempotency_key = "a".repeat(64);
        let encode_error = encode(KIND_JOB_REQUEST, &lowercase_event_id_shape, fixed_now()).err();

        let mut uppercase_event_id_shape = valid_projection();
        uppercase_event_id_shape.idempotency_key = "F".repeat(64);
        let payload =
            serde_json::to_vec(&uppercase_event_id_shape).expect("serialize raw projection");
        let decode_error = decode(KIND_JOB_REQUEST, &payload, fixed_now()).err();

        let mut mixed_case_event_id_shape = valid_projection();
        mixed_case_event_id_shape.idempotency_key = "aF".repeat(32);
        let mixed_case_error =
            encode(KIND_JOB_REQUEST, &mixed_case_event_id_shape, fixed_now()).err();

        assert_eq!(
            (
                encode_error,
                encode_error.map(AgentJobError::code),
                decode_error,
                decode_error.map(AgentJobError::code),
                mixed_case_error,
                mixed_case_error.map(AgentJobError::code),
            ),
            (
                Some(AgentJobError::InvalidIdentity),
                Some("invalid_identity"),
                Some(AgentJobError::InvalidIdentity),
                Some("invalid_identity"),
                Some(AgentJobError::InvalidIdentity),
                Some("invalid_identity"),
            )
        );
    }

    #[test]
    fn relation_targets_a_distinct_portable_idempotency_key() {
        let mut self_relation = valid_projection();
        self_relation.relation = Some(RecordRelation {
            kind: RelationKind::Corrects,
            target_idempotency_key: self_relation.idempotency_key.clone(),
        });
        let error = encode(KIND_JOB_REQUEST, &self_relation, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidRelation);
        assert_eq!(error.code(), "invalid_relation");

        let mut path_relation = valid_projection();
        path_relation.relation = Some(RecordRelation {
            kind: RelationKind::Supersedes,
            target_idempotency_key: "/tmp/prior".to_owned(),
        });
        let error = encode(KIND_JOB_REQUEST, &path_relation, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidRelation);
        assert_eq!(error.code(), "invalid_relation");

        let mut valid_relation = valid_projection();
        valid_relation.relation = Some(RecordRelation {
            kind: RelationKind::Corrects,
            target_idempotency_key: "job-request:attempt:1:0".to_owned(),
        });
        encode(KIND_JOB_REQUEST, &valid_relation, fixed_now()).expect("valid relation encodes");

        let mut opaque_sixty_four_byte_relation = valid_projection();
        opaque_sixty_four_byte_relation.relation = Some(RecordRelation {
            kind: RelationKind::Supersedes,
            target_idempotency_key: format!("g{}", "a".repeat(63)),
        });
        encode(
            KIND_JOB_REQUEST,
            &opaque_sixty_four_byte_relation,
            fixed_now(),
        )
        .expect("non-hex opaque relation target encodes");
    }

    #[test]
    fn rejects_nostr_event_id_relation_target_on_encode() {
        let mut projection = valid_projection();
        projection.relation = Some(RecordRelation {
            kind: RelationKind::Corrects,
            target_idempotency_key: "a".repeat(64),
        });

        let Err(error) = encode(KIND_JOB_REQUEST, &projection, fixed_now()) else {
            panic!("64-character lowercase hex relation target must be rejected");
        };
        assert_eq!(error, AgentJobError::InvalidRelation);
        assert_eq!(error.code(), "invalid_relation");
    }

    #[test]
    fn rejects_nostr_event_id_relation_target_on_decode() {
        let mut projection = valid_projection();
        projection.relation = Some(RecordRelation {
            kind: RelationKind::Supersedes,
            target_idempotency_key: "F".repeat(64),
        });
        let payload = serde_json::to_vec(&projection).expect("serialize");

        let Err(error) = decode(KIND_JOB_REQUEST, &payload, fixed_now()) else {
            panic!("64-character uppercase hex relation target must be rejected");
        };
        assert_eq!(error, AgentJobError::InvalidRelation);
        assert_eq!(error.code(), "invalid_relation");
    }

    #[test]
    fn content_and_reference_limits_are_enforced() {
        let mut too_many = valid_projection();
        too_many.evidence = (0..=MAX_REFERENCES)
            .map(|index| ProjectionReference {
                id: format!("evidence:{index}"),
            })
            .collect();
        let error = encode(KIND_JOB_REQUEST, &too_many, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::TooManyReferences);
        assert_eq!(error.code(), "too_many_references");

        let mut private_path = valid_projection();
        private_path.evidence.push(ProjectionReference {
            id: "/home/example/private.txt".to_owned(),
        });
        let error = encode(KIND_JOB_REQUEST, &private_path, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidReference);
        assert_eq!(error.code(), "invalid_reference");
        assert!(!error.to_string().contains("private.txt"));

        let mut duplicate = valid_projection();
        duplicate.artifacts.push(ProjectionReference {
            id: "evidence:one".to_owned(),
        });
        duplicate.evidence.push(ProjectionReference {
            id: "evidence:one".to_owned(),
        });
        let error = encode(KIND_JOB_REQUEST, &duplicate, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidReference);
        assert_eq!(error.code(), "invalid_reference");
    }

    #[test]
    fn active_records_fail_closed_when_expired() {
        let mut projection = valid_projection();
        projection.occurred_at = fixed_now() - Duration::minutes(1);
        projection.expires_at = fixed_now();

        let error = encode(KIND_JOB_REQUEST, &projection, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidExpiry);
        assert_eq!(error.code(), "invalid_expiry");
    }

    #[test]
    fn terminal_records_may_arrive_after_the_active_lease() {
        let mut projection = valid_projection();
        projection.transition = StateTransition {
            from: Some(JobState::Running),
            to: JobState::Succeeded,
        };
        projection.occurred_at = fixed_now() - Duration::minutes(2);
        projection.expires_at = fixed_now() - Duration::minutes(1);
        projection.evidence.push(ProjectionReference {
            id: "receipt:late-result".to_owned(),
        });

        encode(KIND_JOB_RESULT, &projection, fixed_now()).expect("late terminal result encodes");
    }

    #[test]
    fn portable_integer_bounds_are_enforced() {
        let mut maximum = valid_projection();
        maximum.sequence = MAX_SAFE_INTEGER;
        maximum.revision = MAX_SAFE_INTEGER;
        encode(KIND_JOB_REQUEST, &maximum, fixed_now()).expect("safe integer bounds encode");

        let mut zero_sequence = maximum.clone();
        zero_sequence.sequence = 0;
        let error = encode(KIND_JOB_REQUEST, &zero_sequence, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidSequence);
        assert_eq!(error.code(), "invalid_sequence");

        let mut oversized_sequence = maximum.clone();
        oversized_sequence.sequence = MAX_SAFE_INTEGER + 1;
        let error = encode(KIND_JOB_REQUEST, &oversized_sequence, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidSequence);
        assert_eq!(error.code(), "invalid_sequence");

        let mut zero_revision = maximum.clone();
        zero_revision.revision = 0;
        let error = encode(KIND_JOB_REQUEST, &zero_revision, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidSequence);
        assert_eq!(error.code(), "invalid_sequence");

        let mut oversized_revision = maximum;
        oversized_revision.revision = MAX_SAFE_INTEGER + 1;
        let error = encode(KIND_JOB_REQUEST, &oversized_revision, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidSequence);
        assert_eq!(error.code(), "invalid_sequence");
    }

    #[test]
    fn result_and_error_require_content_minimized_outcomes() {
        let mut result = valid_projection();
        result.transition = StateTransition {
            from: Some(JobState::Running),
            to: JobState::Succeeded,
        };
        let error = encode(KIND_JOB_RESULT, &result, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidOutcome);
        assert_eq!(error.code(), "invalid_outcome");

        result.evidence.push(ProjectionReference {
            id: "receipt:result:1".to_owned(),
        });
        encode(KIND_JOB_RESULT, &result, fixed_now()).expect("result with evidence encodes");

        let mut failed = valid_projection();
        failed.transition = StateTransition {
            from: Some(JobState::Running),
            to: JobState::Failed,
        };
        let error = encode(KIND_JOB_ERROR, &failed, fixed_now()).unwrap_err();
        assert_eq!(error, AgentJobError::InvalidOutcome);
        assert_eq!(error.code(), "invalid_outcome");

        failed.error_code = Some("provider_unavailable".to_owned());
        encode(KIND_JOB_ERROR, &failed, fixed_now()).expect("error with code encodes");
    }

    #[test]
    fn every_permitted_transition_edge_is_accepted() {
        let transitions = [
            (KIND_JOB_REQUEST, None, JobState::Requested),
            (
                KIND_JOB_ACCEPTED,
                Some(JobState::Requested),
                JobState::Accepted,
            ),
            (
                KIND_JOB_PROGRESS,
                Some(JobState::Accepted),
                JobState::Running,
            ),
            (
                KIND_JOB_PROGRESS,
                Some(JobState::Running),
                JobState::Running,
            ),
            (
                KIND_JOB_RESULT,
                Some(JobState::Accepted),
                JobState::Succeeded,
            ),
            (
                KIND_JOB_RESULT,
                Some(JobState::Running),
                JobState::Succeeded,
            ),
            (
                KIND_JOB_RESULT,
                Some(JobState::CancellationRequested),
                JobState::Succeeded,
            ),
            (
                KIND_JOB_CANCEL,
                Some(JobState::Requested),
                JobState::CancellationRequested,
            ),
            (
                KIND_JOB_CANCEL,
                Some(JobState::Accepted),
                JobState::CancellationRequested,
            ),
            (
                KIND_JOB_CANCEL,
                Some(JobState::Running),
                JobState::CancellationRequested,
            ),
            (KIND_JOB_ERROR, Some(JobState::Requested), JobState::Failed),
            (KIND_JOB_ERROR, Some(JobState::Accepted), JobState::Failed),
            (KIND_JOB_ERROR, Some(JobState::Running), JobState::Failed),
            (
                KIND_JOB_ERROR,
                Some(JobState::CancellationRequested),
                JobState::Failed,
            ),
        ];

        for (kind, from, to) in transitions {
            let mut projection = valid_projection();
            projection.transition = StateTransition { from, to };
            if kind == KIND_JOB_RESULT {
                projection.evidence.push(ProjectionReference {
                    id: "receipt:result:1".to_owned(),
                });
            }
            if kind == KIND_JOB_ERROR {
                projection.error_code = Some("provider_unavailable".to_owned());
            }

            encode(kind, &projection, fixed_now()).expect("permitted transition encodes");
        }
    }

    #[test]
    fn exhaustive_transition_matrix_accepts_exactly_the_frozen_edges() {
        let kinds = [
            JobKind::Request,
            JobKind::Accepted,
            JobKind::Progress,
            JobKind::Result,
            JobKind::Cancel,
            JobKind::Error,
        ];
        let from_states = [
            None,
            Some(JobState::Requested),
            Some(JobState::Accepted),
            Some(JobState::Running),
            Some(JobState::Succeeded),
            Some(JobState::CancellationRequested),
            Some(JobState::Failed),
        ];
        let to_states = [
            JobState::Requested,
            JobState::Accepted,
            JobState::Running,
            JobState::Succeeded,
            JobState::CancellationRequested,
            JobState::Failed,
        ];
        let allowed = [
            (JobKind::Request, None, JobState::Requested),
            (
                JobKind::Accepted,
                Some(JobState::Requested),
                JobState::Accepted,
            ),
            (
                JobKind::Progress,
                Some(JobState::Accepted),
                JobState::Running,
            ),
            (
                JobKind::Progress,
                Some(JobState::Running),
                JobState::Running,
            ),
            (
                JobKind::Result,
                Some(JobState::Accepted),
                JobState::Succeeded,
            ),
            (
                JobKind::Result,
                Some(JobState::Running),
                JobState::Succeeded,
            ),
            (
                JobKind::Result,
                Some(JobState::CancellationRequested),
                JobState::Succeeded,
            ),
            (
                JobKind::Cancel,
                Some(JobState::Requested),
                JobState::CancellationRequested,
            ),
            (
                JobKind::Cancel,
                Some(JobState::Accepted),
                JobState::CancellationRequested,
            ),
            (
                JobKind::Cancel,
                Some(JobState::Running),
                JobState::CancellationRequested,
            ),
            (JobKind::Error, Some(JobState::Requested), JobState::Failed),
            (JobKind::Error, Some(JobState::Accepted), JobState::Failed),
            (JobKind::Error, Some(JobState::Running), JobState::Failed),
            (
                JobKind::Error,
                Some(JobState::CancellationRequested),
                JobState::Failed,
            ),
        ];

        let mut accepted = 0;
        for kind in kinds {
            for from in from_states {
                for to in to_states {
                    let transition = StateTransition { from, to };
                    let expected = allowed.contains(&(kind, from, to));
                    let actual = valid_transition(kind, &transition);
                    assert_eq!(
                        actual, expected,
                        "unexpected transition result for {kind:?} {from:?}->{to:?}"
                    );
                    accepted += usize::from(actual);
                }
            }
        }

        assert_eq!(accepted, 14);
    }

    #[test]
    fn validation_precedence_surfaces_one_stable_error_at_a_time() {
        let mut projection = valid_projection();
        projection.format = "other-format".to_owned();
        projection.version = VERSION + 1;
        projection.conversation_id.clear();
        projection.sequence = 0;
        projection.transition = StateTransition {
            from: Some(JobState::Running),
            to: JobState::Succeeded,
        };
        projection.expires_at = fixed_now();
        projection.artifacts = references("artifact", 16);
        projection.evidence = references("evidence", 17);
        projection.artifacts[0].id = "reference:duplicate".to_owned();
        projection.evidence[0].id = "/tmp/private-reference".to_owned();
        projection.evidence[1].id = "reference:duplicate".to_owned();
        projection.relation = Some(RecordRelation {
            kind: RelationKind::Corrects,
            target_idempotency_key: projection.idempotency_key.clone(),
        });
        projection.error_code = Some("forbidden_error".to_owned());

        assert_encode_error(
            KIND_JOB_REQUEST,
            &projection,
            AgentJobError::UnsupportedFormat,
            "unsupported_format",
        );
        projection.format = FORMAT.to_owned();

        assert_encode_error(
            KIND_JOB_REQUEST,
            &projection,
            AgentJobError::UnsupportedVersion,
            "unsupported_version",
        );
        projection.version = VERSION;

        assert_encode_error(
            KIND_JOB_REQUEST,
            &projection,
            AgentJobError::InvalidIdentity,
            "invalid_identity",
        );
        projection.conversation_id = "conversation:alpha".to_owned();

        assert_encode_error(
            KIND_JOB_REQUEST,
            &projection,
            AgentJobError::InvalidSequence,
            "invalid_sequence",
        );
        projection.sequence = 1;

        assert_encode_error(
            KIND_JOB_REQUEST,
            &projection,
            AgentJobError::InvalidTransition,
            "invalid_transition",
        );
        projection.transition = StateTransition {
            from: None,
            to: JobState::Requested,
        };

        assert_encode_error(
            KIND_JOB_REQUEST,
            &projection,
            AgentJobError::InvalidExpiry,
            "invalid_expiry",
        );
        projection.expires_at = fixed_expiry();

        assert_encode_error(
            KIND_JOB_REQUEST,
            &projection,
            AgentJobError::TooManyReferences,
            "too_many_references",
        );
        projection.evidence.truncate(16);

        assert_encode_error(
            KIND_JOB_REQUEST,
            &projection,
            AgentJobError::InvalidReference,
            "invalid_reference",
        );
        projection.evidence[0].id = "evidence:fixed:zero".to_owned();

        assert_encode_error(
            KIND_JOB_REQUEST,
            &projection,
            AgentJobError::InvalidReference,
            "invalid_reference",
        );
        projection.evidence[1].id = "evidence:fixed:one".to_owned();

        assert_encode_error(
            KIND_JOB_REQUEST,
            &projection,
            AgentJobError::InvalidRelation,
            "invalid_relation",
        );
        projection
            .relation
            .as_mut()
            .expect("relation remains present")
            .target_idempotency_key = "prior:attempt:1".to_owned();

        assert_encode_error(
            KIND_JOB_REQUEST,
            &projection,
            AgentJobError::InvalidOutcome,
            "invalid_outcome",
        );
        projection.error_code = None;

        encode(KIND_JOB_REQUEST, &projection, fixed_now())
            .expect("fully repaired projection encodes");
    }

    #[test]
    fn decode_enforces_every_semantic_validation_layer() {
        let mut invalid_identity = valid_projection();
        invalid_identity.mission_id = "/tmp/mission".to_owned();

        let mut invalid_transition = valid_projection();
        invalid_transition.transition = StateTransition {
            from: Some(JobState::Running),
            to: JobState::Succeeded,
        };

        let mut invalid_expiry = valid_projection();
        invalid_expiry.occurred_at = fixed_now() - Duration::minutes(1);
        invalid_expiry.expires_at = fixed_now();

        let mut invalid_reference = valid_projection();
        invalid_reference.artifacts.push(ProjectionReference {
            id: "/tmp/artifact".to_owned(),
        });

        let mut invalid_relation = valid_projection();
        invalid_relation.relation = Some(RecordRelation {
            kind: RelationKind::Corrects,
            target_idempotency_key: invalid_relation.idempotency_key.clone(),
        });

        let mut invalid_outcome = valid_projection();
        invalid_outcome.transition = StateTransition {
            from: Some(JobState::Running),
            to: JobState::Succeeded,
        };

        let cases = [
            (
                "identity",
                KIND_JOB_REQUEST,
                invalid_identity,
                AgentJobError::InvalidIdentity,
                "invalid_identity",
            ),
            (
                "transition",
                KIND_JOB_REQUEST,
                invalid_transition,
                AgentJobError::InvalidTransition,
                "invalid_transition",
            ),
            (
                "expiry",
                KIND_JOB_REQUEST,
                invalid_expiry,
                AgentJobError::InvalidExpiry,
                "invalid_expiry",
            ),
            (
                "reference",
                KIND_JOB_REQUEST,
                invalid_reference,
                AgentJobError::InvalidReference,
                "invalid_reference",
            ),
            (
                "relation",
                KIND_JOB_REQUEST,
                invalid_relation,
                AgentJobError::InvalidRelation,
                "invalid_relation",
            ),
            (
                "outcome",
                KIND_JOB_RESULT,
                invalid_outcome,
                AgentJobError::InvalidOutcome,
                "invalid_outcome",
            ),
        ];

        for (layer, kind, projection, expected, expected_code) in cases {
            let payload = serde_json::to_vec(&projection).expect("serialize raw projection");
            let error = decode(kind, &payload, fixed_now()).unwrap_err();
            assert_eq!(error, expected, "unexpected decode error at {layer} layer");
            assert_eq!(
                error.code(),
                expected_code,
                "unexpected decode code at {layer} layer"
            );
        }
    }

    #[test]
    fn identifier_byte_boundaries_are_enforced() {
        let mut one_byte = valid_projection();
        one_byte.conversation_id = "a".to_owned();
        encode(KIND_JOB_REQUEST, &one_byte, fixed_now()).expect("one-byte identity encodes");

        let mut maximum = valid_projection();
        maximum.conversation_id = "a".repeat(MAX_ID_BYTES);
        encode(KIND_JOB_REQUEST, &maximum, fixed_now()).expect("maximum identity encodes");

        let mut oversized = valid_projection();
        oversized.conversation_id = "a".repeat(MAX_ID_BYTES + 1);
        assert_encode_error(
            KIND_JOB_REQUEST,
            &oversized,
            AgentJobError::InvalidIdentity,
            "invalid_identity",
        );

        let mut invalid_first_byte = valid_projection();
        invalid_first_byte.conversation_id = "-conversation".to_owned();
        assert_encode_error(
            KIND_JOB_REQUEST,
            &invalid_first_byte,
            AgentJobError::InvalidIdentity,
            "invalid_identity",
        );

        let mut non_ascii = valid_projection();
        non_ascii.conversation_id = "conversation:café".to_owned();
        assert_encode_error(
            KIND_JOB_REQUEST,
            &non_ascii,
            AgentJobError::InvalidIdentity,
            "invalid_identity",
        );
    }

    #[test]
    fn error_code_byte_boundaries_are_enforced() {
        let mut projection = valid_projection();
        projection.transition = StateTransition {
            from: Some(JobState::Running),
            to: JobState::Failed,
        };

        projection.error_code = Some("e".to_owned());
        encode(KIND_JOB_ERROR, &projection, fixed_now()).expect("one-byte error code encodes");

        projection.error_code = Some("e".repeat(MAX_ERROR_CODE_BYTES));
        encode(KIND_JOB_ERROR, &projection, fixed_now()).expect("maximum error code encodes");

        projection.error_code = Some("e".repeat(MAX_ERROR_CODE_BYTES + 1));
        assert_encode_error(
            KIND_JOB_ERROR,
            &projection,
            AgentJobError::InvalidOutcome,
            "invalid_outcome",
        );
    }

    #[test]
    fn error_code_is_forbidden_for_every_non_error_kind() {
        let cases = [
            (
                KIND_JOB_REQUEST,
                StateTransition {
                    from: None,
                    to: JobState::Requested,
                },
                false,
            ),
            (
                KIND_JOB_ACCEPTED,
                StateTransition {
                    from: Some(JobState::Requested),
                    to: JobState::Accepted,
                },
                false,
            ),
            (
                KIND_JOB_PROGRESS,
                StateTransition {
                    from: Some(JobState::Accepted),
                    to: JobState::Running,
                },
                false,
            ),
            (
                KIND_JOB_RESULT,
                StateTransition {
                    from: Some(JobState::Running),
                    to: JobState::Succeeded,
                },
                true,
            ),
            (
                KIND_JOB_CANCEL,
                StateTransition {
                    from: Some(JobState::Running),
                    to: JobState::CancellationRequested,
                },
                false,
            ),
        ];

        for (kind, transition, needs_reference) in cases {
            let mut projection = valid_projection();
            projection.transition = transition;
            projection.error_code = Some("forbidden_error".to_owned());
            if needs_reference {
                projection.evidence.push(ProjectionReference {
                    id: "receipt:result:1".to_owned(),
                });
            }

            assert_encode_error(
                kind,
                &projection,
                AgentJobError::InvalidOutcome,
                "invalid_outcome",
            );
        }
    }

    #[test]
    fn active_expiry_is_strict_against_occurrence_and_validation_time() {
        let mut before_occurrence = valid_projection();
        before_occurrence.occurred_at = fixed_now() + Duration::seconds(2);
        before_occurrence.expires_at = fixed_now() + Duration::seconds(1);
        assert_encode_error(
            KIND_JOB_REQUEST,
            &before_occurrence,
            AgentJobError::InvalidExpiry,
            "invalid_expiry",
        );

        let mut equal_to_occurrence = valid_projection();
        equal_to_occurrence.occurred_at = fixed_now() + Duration::seconds(1);
        equal_to_occurrence.expires_at = equal_to_occurrence.occurred_at;
        assert_encode_error(
            KIND_JOB_REQUEST,
            &equal_to_occurrence,
            AgentJobError::InvalidExpiry,
            "invalid_expiry",
        );

        let mut one_second_after_both = valid_projection();
        one_second_after_both.occurred_at = fixed_now();
        one_second_after_both.expires_at = fixed_now() + Duration::seconds(1);
        encode(KIND_JOB_REQUEST, &one_second_after_both, fixed_now())
            .expect("expiry one second after occurrence and validation time encodes");
    }

    #[test]
    fn combined_reference_limit_is_enforced_across_both_lists() {
        let mut at_limit = valid_projection();
        at_limit.artifacts = references("artifact", 16);
        at_limit.evidence = references("evidence", 16);
        encode(KIND_JOB_REQUEST, &at_limit, fixed_now()).expect("combined reference limit encodes");

        let mut over_limit = at_limit;
        over_limit.evidence.push(ProjectionReference {
            id: "evidence:16".to_owned(),
        });
        assert_encode_error(
            KIND_JOB_REQUEST,
            &over_limit,
            AgentJobError::TooManyReferences,
            "too_many_references",
        );
    }

    #[test]
    fn encoded_timestamp_year_profile_is_bounded_but_decode_remains_permissive() {
        fn timestamp(year: i32) -> DateTime<Utc> {
            Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0)
                .single()
                .expect("requested Chrono timestamp is constructible")
        }

        fn terminal_result_projection() -> AgentJobProjection {
            let mut projection = valid_projection();
            projection.transition = StateTransition {
                from: Some(JobState::Running),
                to: JobState::Succeeded,
            };
            projection.evidence.push(ProjectionReference {
                id: "receipt:portable-timestamp".to_owned(),
            });
            projection
        }

        let mut at_portable_boundaries = terminal_result_projection();
        at_portable_boundaries.occurred_at = timestamp(0);
        at_portable_boundaries.expires_at = timestamp(9999);
        encode(KIND_JOB_RESULT, &at_portable_boundaries, fixed_now())
            .expect("inclusive portable year boundaries encode");

        let outside_portable_profile = [
            (
                "occurred_at below year zero",
                timestamp(-1),
                timestamp(9999),
            ),
            (
                "occurred_at above year 9999",
                timestamp(10_000),
                timestamp(9999),
            ),
            ("expires_at below year zero", timestamp(0), timestamp(-1)),
            (
                "expires_at above year 9999",
                timestamp(0),
                timestamp(10_000),
            ),
        ];

        let mut unexpectedly_encoded = Vec::new();
        for (case, occurred_at, expires_at) in outside_portable_profile {
            let mut projection = terminal_result_projection();
            projection.occurred_at = occurred_at;
            projection.expires_at = expires_at;

            match encode(KIND_JOB_RESULT, &projection, fixed_now()) {
                Ok(_) => unexpectedly_encoded.push(case),
                Err(error) => {
                    assert_eq!(error, AgentJobError::InvalidSchema, "{case}");
                    assert_eq!(error.code(), "invalid_schema", "{case}");
                }
            }
        }
        assert!(
            unexpectedly_encoded.is_empty(),
            "out-of-profile timestamps encoded successfully: {unexpectedly_encoded:?}"
        );

        let mut extended_year = terminal_result_projection();
        extended_year.occurred_at = timestamp(10_000);
        extended_year.expires_at = timestamp(10_001);
        let raw_json = serde_json::to_vec(&extended_year)
            .expect("Chrono extended-year projection serializes directly");
        let decoded = decode(KIND_JOB_RESULT, &raw_json, fixed_now())
            .expect("decode retains Chrono extended-year compatibility");

        assert_eq!(decoded.occurred_at, extended_year.occurred_at);
        assert_eq!(decoded.expires_at, extended_year.expires_at);
    }

    #[test]
    fn packaged_nip_matches_normative_repository_source_in_workspace() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let repository_root = manifest_dir.join("../..");
        let canonical_path = repository_root.join("docs/nips/NIP-AJ.md");
        let package_metadata = manifest_dir.join(".cargo_vcs_info.json");

        if package_metadata.is_file() {
            // A published crate intentionally has no repository root. Its
            // contract and example tests below still execute against this copy.
            assert!(PACKAGED_NIP
                .starts_with("<!--\nCanonical repository source: docs/nips/NIP-AJ.md\n"));
        } else {
            let canonical_nip = std::fs::read_to_string(&canonical_path).unwrap_or_else(|error| {
                panic!(
                    "repository workspace must contain canonical {}: {error}",
                    canonical_path.display()
                )
            });
            assert_nip_mirror_matches(PACKAGED_NIP, &canonical_nip);
        }
    }

    #[test]
    fn nip_drift_failure_does_not_echo_document_content() {
        let panic = std::panic::catch_unwind(|| {
            assert_nip_mirror_matches("packaged-document-body", "canonical-document-body");
        })
        .expect_err("different NIP documents must fail the drift guard");
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .expect("drift assertion panic must contain a message");

        assert_eq!(
            message,
            "packaged NIP-AJ fixture drifted from the normative repository source"
        );
        assert!(!message.contains("packaged-document-body"));
        assert!(!message.contains("canonical-document-body"));
    }

    #[test]
    fn normative_nip_documents_projection_only_authority() {
        let nip = PACKAGED_NIP;
        assert!(nip.contains("projection_only"));
        assert!(nip.contains("never a launch authorization"));
        assert!(nip.contains("16,384 bytes"));
        assert!(nip.contains("Wilson owns canonical conversation"));
        assert!(nip.contains("only a projection claim"));
        assert!(nip.contains("64 ASCII hexadecimal"));
        assert!(nip.contains("Validation boundary"));
        assert!(nip.contains("record-local"));
    }

    #[test]
    fn normative_nip_request_example_decodes() {
        let nip = PACKAGED_NIP;
        let (_, after_opening_fence) = nip
            .split_once("```json\n")
            .expect("NIP-AJ has a fenced JSON example");
        let (json, _) = after_opening_fence
            .split_once("\n```")
            .expect("NIP-AJ JSON example has a closing fence");

        let projection = decode(KIND_JOB_REQUEST, json.as_bytes(), fixed_now())
            .expect("documented request example decodes");

        assert_eq!(projection.conversation_id, "conversation:alpha");
        assert_eq!(projection.mission_id, "mission:alpha");
        assert_eq!(projection.workstream_id, "workstream:contract");
        assert_eq!(
            projection.authority.canonical_system,
            CanonicalSystem::MotifOs
        );
        assert_eq!(projection.authority.buzz_role, BuzzRole::ProjectionOnly);
        assert_eq!(projection.actor.seat_id, SeatId::Wilson);
        assert_eq!(projection.transition.to, JobState::Requested);
    }
}
