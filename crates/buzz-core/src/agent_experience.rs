//! Bounded, owner-encrypted records of meaningful adviser task experience.

use chrono::DateTime;
use nostr::{Event, Keys, PublicKey};
use serde::{Deserialize, Serialize};

use crate::engram::{self, Body};

/// Maximum UTF-8 bytes in the task summary.
pub const MAX_TASK_SUMMARY_BYTES: usize = 4_096;
/// Maximum UTF-8 bytes in any other free-text item.
pub const MAX_TEXT_ITEM_BYTES: usize = 4_096;
/// Maximum number of items in any experience list.
pub const MAX_EXPERIENCE_ITEMS: usize = 64;
/// Maximum UTF-8 bytes in a stable identifier.
pub const MAX_EXPERIENCE_ID_BYTES: usize = 512;
/// Maximum number of skill versions on one experience.
pub const MAX_SKILL_VERSIONS: usize = 32;

/// Terminal result of an adviser task experience.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceOutcome {
    /// The requested work completed successfully.
    Succeeded,
    /// The work ended in a reported failure.
    Failed,
    /// The output was corrected after review.
    Corrected,
    /// A later record replaced this result.
    Superseded,
    /// The task was cancelled before completion.
    Cancelled,
    /// Work stopped without a completed result.
    Abandoned,
}

/// Visibility of an active memory derived from an experience.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryScope {
    /// Visible only to the named specialist for this owner.
    SpecialistPrivate,
    /// Visible to the owner's named Command Team.
    CommandTeamShared,
}

/// Bounded evidence that a tool ran, without raw arguments or credentials.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolEvidenceV1 {
    /// Stable tool name.
    pub tool: String,
    /// Stable bounded result code.
    pub result_code: String,
    /// Short result summary; raw provider payloads are excluded.
    pub summary: String,
}

/// Immutable skill version used for the task.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillVersionV1 {
    /// Stable skill identity.
    pub skill_id: String,
    /// Immutable version identity.
    pub version: String,
}

/// One deterministic validation result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationResultV1 {
    /// Stable validation check identity.
    pub check_id: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Optional bounded, redacted detail.
    pub detail: Option<String>,
}

/// Version-one immutable adviser experience payload.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExperienceRecordV1 {
    /// Globally unique immutable record identity.
    pub record_id: String,
    /// Stable key used to derive an active memory lineage.
    pub memory_key: String,
    /// Recall visibility.
    pub scope: MemoryScope,
    /// Originating specialist, when applicable.
    pub specialist_id: Option<String>,
    /// Command Team identity for shared memories.
    pub team_id: Option<String>,
    /// RFC3339 task occurrence timestamp.
    pub occurred_at: String,
    /// Short description of the task.
    pub task_summary: String,
    /// Decision reached, if any.
    pub decision: Option<String>,
    /// Assumptions used in the work.
    pub assumptions: Vec<String>,
    /// Dissenting views retained from the task.
    pub dissent: Vec<String>,
    /// Known limitations.
    pub limitations: Vec<String>,
    /// Terminal task result.
    pub outcome: ExperienceOutcome,
    /// Bounded tool results, never raw tool inputs or credentials.
    pub tool_evidence: Vec<ToolEvidenceV1>,
    /// Stable source identities rather than copied source payloads.
    pub source_ids: Vec<String>,
    /// Exact provider/model/runtime identity.
    pub model_identity: String,
    /// Prompt-template identity.
    pub prompt_template_id: String,
    /// Active-memory view revision supplied to the task.
    pub memory_view_revision: String,
    /// Frozen RAG snapshot identity supplied to the task.
    pub rag_snapshot_id: String,
    /// Immutable skills used by the task.
    pub skill_versions: Vec<SkillVersionV1>,
    /// Deterministic validation results.
    pub validation_results: Vec<ValidationResultV1>,
    /// Earlier immutable experience record identities replaced by this record.
    pub supersedes: Vec<String>,
    /// Bounded confidence from `0.0` to `1.0`.
    pub confidence: f64,
}

/// Fail-closed experience validation, encoding, or engram error.
#[derive(Debug, thiserror::Error)]
pub enum ExperienceError {
    /// The record or decoded envelope did not satisfy the bounded contract.
    #[error("invalid agent experience")]
    Invalid,
    /// JSON encoding or decoding failed.
    #[error("agent experience serialization failed")]
    Serialization,
    /// NIP-AE encryption or envelope construction failed.
    #[error("agent experience engram failed")]
    Engram(#[from] engram::EngramError),
}

impl ExperienceRecordV1 {
    /// Validate the bounded, secret-minimising experience contract.
    pub fn validate(&self) -> Result<(), ExperienceError> {
        if !valid_record_id(&self.record_id)
            || !valid_id(&self.memory_key)
            || DateTime::parse_from_rfc3339(&self.occurred_at).is_err()
            || !valid_text(&self.task_summary, MAX_TASK_SUMMARY_BYTES)
            || !valid_id(&self.model_identity)
            || !valid_id(&self.prompt_template_id)
            || !valid_id(&self.memory_view_revision)
            || !valid_id(&self.rag_snapshot_id)
            || !self.confidence.is_finite()
            || !(0.0..=1.0).contains(&self.confidence)
            || !valid_optional_id(self.specialist_id.as_deref())
            || !valid_optional_id(self.team_id.as_deref())
            || !valid_optional_text(self.decision.as_deref())
            || !valid_text_list(&self.assumptions)
            || !valid_text_list(&self.dissent)
            || !valid_text_list(&self.limitations)
            || !valid_id_list(&self.source_ids, MAX_EXPERIENCE_ITEMS)
            || !valid_id_list(&self.supersedes, MAX_EXPERIENCE_ITEMS)
            || self.supersedes.iter().any(|id| id == &self.record_id)
            || has_duplicates(&self.supersedes)
            || self.tool_evidence.len() > MAX_EXPERIENCE_ITEMS
            || self.validation_results.len() > MAX_EXPERIENCE_ITEMS
            || self.skill_versions.len() > MAX_SKILL_VERSIONS
            || !self.tool_evidence.iter().all(valid_tool_evidence)
            || !self.skill_versions.iter().all(valid_skill_version)
            || !self.validation_results.iter().all(valid_validation_result)
        {
            return Err(ExperienceError::Invalid);
        }
        match self.scope {
            MemoryScope::SpecialistPrivate if self.specialist_id.is_none() => {
                return Err(ExperienceError::Invalid)
            }
            MemoryScope::CommandTeamShared if self.team_id.is_none() => {
                return Err(ExperienceError::Invalid)
            }
            _ => {}
        }
        if self.outcome == ExperienceOutcome::Superseded && self.supersedes.is_empty() {
            return Err(ExperienceError::Invalid);
        }
        Ok(())
    }

    /// Encode a validated, conservatively redacted record as immutable NIP-AE memory.
    pub fn to_engram_body(&self) -> Result<Body, ExperienceError> {
        self.validate()?;
        let redacted = self.redacted();
        let value = serde_json::to_string(&redacted).map_err(|_| ExperienceError::Serialization)?;
        Ok(Body::Memory {
            slug: experience_slug(&self.record_id)?,
            value: Some(value),
        })
    }

    fn redacted(&self) -> Self {
        let mut record = self.clone();
        record.task_summary = redact_free_text(&record.task_summary);
        record.decision = record.decision.map(|value| redact_free_text(&value));
        redact_list(&mut record.assumptions);
        redact_list(&mut record.dissent);
        redact_list(&mut record.limitations);
        for evidence in &mut record.tool_evidence {
            evidence.summary = redact_free_text(&evidence.summary);
        }
        for result in &mut record.validation_results {
            result.detail = result.detail.take().map(|value| redact_free_text(&value));
        }
        record
    }
}

/// Decode and validate an experience from an immutable NIP-AE memory body.
pub fn from_engram_body(body: &Body) -> Result<ExperienceRecordV1, ExperienceError> {
    let Body::Memory {
        slug,
        value: Some(value),
    } = body
    else {
        return Err(ExperienceError::Invalid);
    };
    let record: ExperienceRecordV1 =
        serde_json::from_str(value).map_err(|_| ExperienceError::Serialization)?;
    record.validate()?;
    if experience_slug(&record.record_id)? != *slug {
        return Err(ExperienceError::Invalid);
    }
    Ok(record)
}

/// Build and sign one immutable encrypted experience event.
pub fn build_experience_event(
    agent_keys: &Keys,
    owner_pubkey: &PublicKey,
    record: &ExperienceRecordV1,
    created_at: u64,
) -> Result<Event, ExperienceError> {
    let body = record.to_engram_body()?;
    Ok(engram::build_event(
        agent_keys,
        owner_pubkey,
        &body,
        created_at,
    )?)
}

/// Build the deterministic Memory MCP projection arguments for a signed record.
pub fn experience_projection_payload(
    event: &Event,
    owner_pubkey: &PublicKey,
    record: &ExperienceRecordV1,
) -> Result<serde_json::Value, ExperienceError> {
    record.validate()?;
    let status = if matches!(
        record.outcome,
        ExperienceOutcome::Succeeded | ExperienceOutcome::Corrected
    ) {
        "active"
    } else {
        "inactive"
    };
    Ok(serde_json::json!({
        "source_event_id": event.id.to_hex(),
        "timestamp": record.occurred_at,
        "agent": record.specialist_id,
        "event_type": "command_experience",
        "content": record.task_summary,
        "metadata": {
            "memory_key": record.memory_key,
            "status": status,
            "scope": record.scope,
            "owner_id": owner_pubkey.to_hex(),
            "team_id": record.team_id,
            "specialist_id": record.specialist_id,
            "confidence": record.confidence,
            "supersedes": record.supersedes,
            "source_event_id": event.id.to_hex(),
            "source_created_at": record.occurred_at
        }
    }))
}

fn experience_slug(record_id: &str) -> Result<String, ExperienceError> {
    if !valid_record_id(record_id) {
        return Err(ExperienceError::Invalid);
    }
    Ok(format!("mem/experience/{record_id}"))
}

fn valid_record_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_EXPERIENCE_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'"' && byte != b'\\')
}

fn valid_optional_id(value: Option<&str>) -> bool {
    value.is_none_or(valid_id)
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.contains('\0')
}

fn valid_optional_text(value: Option<&str>) -> bool {
    value.is_none_or(|text| valid_text(text, MAX_TEXT_ITEM_BYTES))
}

fn valid_text_list(values: &[String]) -> bool {
    values.len() <= MAX_EXPERIENCE_ITEMS
        && values
            .iter()
            .all(|value| valid_text(value, MAX_TEXT_ITEM_BYTES))
}

fn valid_id_list(values: &[String], maximum: usize) -> bool {
    values.len() <= maximum && values.iter().all(|value| valid_id(value))
}

fn valid_tool_evidence(value: &ToolEvidenceV1) -> bool {
    valid_id(&value.tool)
        && valid_id(&value.result_code)
        && valid_text(&value.summary, MAX_TEXT_ITEM_BYTES)
}

fn valid_skill_version(value: &SkillVersionV1) -> bool {
    valid_id(&value.skill_id) && valid_id(&value.version)
}

fn valid_validation_result(value: &ValidationResultV1) -> bool {
    valid_id(&value.check_id) && valid_optional_text(value.detail.as_deref())
}

fn has_duplicates(values: &[String]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[index + 1..].contains(value))
}

fn redact_list(values: &mut [String]) {
    for value in values {
        *value = redact_free_text(value);
    }
}

fn redact_free_text(value: &str) -> String {
    const MARKERS: [&str; 6] = [
        "password=",
        "password:",
        "api_key=",
        "api-key=",
        "authorization: bearer ",
        "bearer ",
    ];
    let lowered = value.to_ascii_lowercase();
    let mut ranges = Vec::new();
    for marker in MARKERS {
        let mut offset = 0;
        while let Some(found) = lowered[offset..].find(marker) {
            let mut start = offset + found + marker.len();
            while value
                .as_bytes()
                .get(start)
                .is_some_and(u8::is_ascii_whitespace)
            {
                start += 1;
            }
            let mut end = start;
            while value
                .as_bytes()
                .get(end)
                .is_some_and(|byte| !byte.is_ascii_whitespace())
            {
                end += 1;
            }
            if end > start {
                ranges.push((start, end));
            }
            offset = end.max(offset + found + marker.len());
            if offset >= lowered.len() {
                break;
            }
        }
    }
    ranges.sort_unstable();
    ranges.dedup();
    let mut redacted = value.to_string();
    for (start, end) in ranges.into_iter().rev() {
        redacted.replace_range(start..end, "[REDACTED]");
    }
    redacted
}
