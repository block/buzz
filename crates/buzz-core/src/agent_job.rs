//! Typed payloads and validation for Buzz's native agent-job protocol.
//!
//! Job events are ordinary, durable Nostr events. The relay is the source of
//! truth for recovery; harnesses keep only an in-memory projection while they
//! are running.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Protocol version emitted by current Buzz job producers.
pub const AGENT_JOB_VERSION: u8 = 1;
/// Maximum serialized job-event content accepted by the relay.
pub const MAX_AGENT_JOB_CONTENT_BYTES: usize = 16 * 1024;
/// Maximum bytes in a delegated objective.
pub const MAX_AGENT_JOB_OBJECTIVE_BYTES: usize = 4 * 1024;
/// Maximum evidence references carried inline by one request or result.
pub const MAX_AGENT_JOB_EVIDENCE_REFS: usize = 32;
/// Maximum bytes in one evidence reference.
pub const MAX_AGENT_JOB_EVIDENCE_REF_BYTES: usize = 1024;

/// A compact reference to durable evidence; source bodies do not ride in jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    /// Stable path, URL, or application-defined evidence URI.
    pub uri: String,
    /// Optional revision, commit, or capture date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Optional section or anchor within the referenced artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
}

/// Bounded execution policy attached to a job request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobBudget {
    /// Absolute job deadline as an RFC3339 timestamp.
    pub deadline_at: String,
    /// Advisory maximum model calls for this task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_model_calls: Option<u32>,
    /// Maximum serialized terminal-result size.
    pub max_output_bytes: usize,
}

/// Contract describing the compact terminal result expected from a worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultContract {
    /// Application-defined result type such as `evidence_packet`.
    pub kind: String,
    /// Required result field names.
    #[serde(default)]
    pub required: Vec<String>,
}

/// Payload for kind `43001` — delegate one bounded task to one agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobRequest {
    /// Protocol version.
    pub v: u8,
    /// Host-generated UUID task identifier.
    pub task_id: Uuid,
    /// Owner-message or application root identifier.
    pub root_task_id: String,
    /// Optional parent task for future correlation; Phase 1.2 is sequential.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<Uuid>,
    /// Logical configured role, for example `research` or `builder`.
    pub assigned_role: String,
    /// One bounded objective.
    pub objective: String,
    /// Existing evidence the worker should reuse first.
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceRef>,
    /// Expected terminal result shape.
    pub result_contract: ResultContract,
    /// Task-specific safety and scope restrictions.
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Deadline and output/model budgets.
    pub budget: JobBudget,
    /// One-based attempt number.
    pub attempt: u32,
}

/// Payload for kind `43002` and `43003` host-generated status events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobStatus {
    /// Protocol version.
    pub v: u8,
    /// Task identifier.
    pub task_id: Uuid,
    /// Short host-generated status label.
    pub status: String,
    /// Optional bounded human-readable detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Terminal result status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobTerminalStatus {
    /// The assigned objective was completed.
    Completed,
    /// Work cannot continue without a real external decision or dependency.
    Blocked,
    /// Work failed.
    Failed,
    /// Work was cancelled.
    Cancelled,
}

/// Payload for kind `43004` and `43006` terminal events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobResult {
    /// Protocol version.
    pub v: u8,
    /// Task identifier.
    pub task_id: Uuid,
    /// Terminal status.
    pub status: JobTerminalStatus,
    /// At most five concise result bullets.
    #[serde(default)]
    pub summary: Vec<String>,
    /// Durable artifacts produced by the task.
    #[serde(default)]
    pub artifacts: Vec<String>,
    /// Evidence references supporting the result.
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceRef>,
    /// Validation checks run by the worker.
    #[serde(default)]
    pub checks: Vec<String>,
    /// Unresolved facts or decisions.
    #[serde(default)]
    pub gaps: Vec<String>,
}

/// Payload for kind `43005` cancellation events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobCancel {
    /// Protocol version.
    pub v: u8,
    /// Task identifier.
    pub task_id: Uuid,
    /// Bounded reason supplied by the owner or delegator.
    pub reason: String,
}

/// Validation failure for a native job payload.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AgentJobError {
    /// Serialized content is too large.
    #[error("job content exceeds {MAX_AGENT_JOB_CONTENT_BYTES} bytes")]
    ContentTooLarge,
    /// JSON did not match the event-kind schema.
    #[error("invalid job payload: {0}")]
    InvalidJson(String),
    /// A required or bounded field is invalid.
    #[error("invalid job field: {0}")]
    InvalidField(&'static str),
}

fn validate_common(version: u8, task_id: Uuid) -> Result<(), AgentJobError> {
    if version != AGENT_JOB_VERSION {
        return Err(AgentJobError::InvalidField("v"));
    }
    if task_id.is_nil() {
        return Err(AgentJobError::InvalidField("task_id"));
    }
    Ok(())
}

fn validate_refs(refs: &[EvidenceRef]) -> Result<(), AgentJobError> {
    if refs.len() > MAX_AGENT_JOB_EVIDENCE_REFS {
        return Err(AgentJobError::InvalidField("evidence_refs"));
    }
    if refs.iter().any(|reference| {
        reference.uri.trim().is_empty()
            || reference.uri.len() > MAX_AGENT_JOB_EVIDENCE_REF_BYTES
            || reference
                .revision
                .as_ref()
                .is_some_and(|value| value.len() > 256)
            || reference
                .section
                .as_ref()
                .is_some_and(|value| value.len() > 256)
    }) {
        return Err(AgentJobError::InvalidField("evidence_refs"));
    }
    Ok(())
}

/// Parse and validate a request payload.
pub fn parse_job_request(content: &str) -> Result<JobRequest, AgentJobError> {
    check_size(content)?;
    let value: JobRequest = serde_json::from_str(content)
        .map_err(|error| AgentJobError::InvalidJson(error.to_string()))?;
    validate_common(value.v, value.task_id)?;
    if value.root_task_id.trim().is_empty() || value.root_task_id.len() > 256 {
        return Err(AgentJobError::InvalidField("root_task_id"));
    }
    if value.assigned_role.trim().is_empty() || value.assigned_role.len() > 64 {
        return Err(AgentJobError::InvalidField("assigned_role"));
    }
    if value.objective.trim().is_empty() || value.objective.len() > MAX_AGENT_JOB_OBJECTIVE_BYTES {
        return Err(AgentJobError::InvalidField("objective"));
    }
    if value.result_contract.kind.trim().is_empty() || value.result_contract.kind.len() > 64 {
        return Err(AgentJobError::InvalidField("result_contract.kind"));
    }
    if value.attempt == 0 || value.budget.max_output_bytes == 0 {
        return Err(AgentJobError::InvalidField("budget/attempt"));
    }
    chrono::DateTime::parse_from_rfc3339(&value.budget.deadline_at)
        .map_err(|_| AgentJobError::InvalidField("budget.deadline_at"))?;
    validate_refs(&value.evidence_refs)?;
    Ok(value)
}

/// Parse and validate an accepted/progress payload.
pub fn parse_job_status(content: &str) -> Result<JobStatus, AgentJobError> {
    check_size(content)?;
    let value: JobStatus = serde_json::from_str(content)
        .map_err(|error| AgentJobError::InvalidJson(error.to_string()))?;
    validate_common(value.v, value.task_id)?;
    if value.status.trim().is_empty() || value.status.len() > 64 {
        return Err(AgentJobError::InvalidField("status"));
    }
    if value
        .detail
        .as_ref()
        .is_some_and(|detail| detail.len() > 512)
    {
        return Err(AgentJobError::InvalidField("detail"));
    }
    Ok(value)
}

/// Parse and validate a terminal result/error payload.
pub fn parse_job_result(content: &str) -> Result<JobResult, AgentJobError> {
    check_size(content)?;
    let value: JobResult = serde_json::from_str(content)
        .map_err(|error| AgentJobError::InvalidJson(error.to_string()))?;
    validate_common(value.v, value.task_id)?;
    if value.summary.len() > 5 || value.summary.iter().any(|line| line.len() > 1024) {
        return Err(AgentJobError::InvalidField("summary"));
    }
    validate_refs(&value.evidence_refs)?;
    Ok(value)
}

/// Parse and validate a cancellation payload.
pub fn parse_job_cancel(content: &str) -> Result<JobCancel, AgentJobError> {
    check_size(content)?;
    let value: JobCancel = serde_json::from_str(content)
        .map_err(|error| AgentJobError::InvalidJson(error.to_string()))?;
    validate_common(value.v, value.task_id)?;
    if value.reason.trim().is_empty() || value.reason.len() > 512 {
        return Err(AgentJobError::InvalidField("reason"));
    }
    Ok(value)
}

fn check_size(content: &str) -> Result<(), AgentJobError> {
    if content.len() > MAX_AGENT_JOB_CONTENT_BYTES {
        return Err(AgentJobError::ContentTooLarge);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> JobRequest {
        JobRequest {
            v: AGENT_JOB_VERSION,
            task_id: Uuid::new_v4(),
            root_task_id: "owner-event".into(),
            parent_task_id: None,
            assigned_role: "research".into(),
            objective: "Verify one fact".into(),
            evidence_refs: vec![EvidenceRef {
                uri: "evidence://verified/item".into(),
                revision: Some("abc123".into()),
                section: None,
            }],
            result_contract: ResultContract {
                kind: "evidence_packet".into(),
                required: vec!["summary".into()],
            },
            constraints: vec!["read-only".into()],
            budget: JobBudget {
                deadline_at: "2026-08-21T12:00:00Z".into(),
                max_model_calls: Some(3),
                max_output_bytes: 8192,
            },
            attempt: 1,
        }
    }

    #[test]
    fn request_round_trips_and_validates() {
        let request = request();
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(parse_job_request(&json).unwrap(), request);
    }

    #[test]
    fn request_rejects_unknown_fields() {
        let mut value = serde_json::to_value(request()).unwrap();
        value["master_prompt"] = serde_json::json!("too much context");
        assert!(matches!(
            parse_job_request(&value.to_string()),
            Err(AgentJobError::InvalidJson(_))
        ));
    }

    #[test]
    fn request_rejects_unbounded_evidence() {
        let mut request = request();
        request.evidence_refs = (0..=MAX_AGENT_JOB_EVIDENCE_REFS)
            .map(|index| EvidenceRef {
                uri: format!("evidence://{index}"),
                revision: None,
                section: None,
            })
            .collect();
        assert_eq!(
            parse_job_request(&serde_json::to_string(&request).unwrap()),
            Err(AgentJobError::InvalidField("evidence_refs"))
        );
    }
}
