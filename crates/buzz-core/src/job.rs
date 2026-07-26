//! Structured payloads for the signed agent job lifecycle.

use serde::{Deserialize, Serialize};
use url::Url;

/// Current schema version for [`JobResultPayload`].
pub const JOB_RESULT_SCHEMA_VERSION: u8 = 1;

/// Maximum serialized size of a job result payload.
pub const MAX_JOB_RESULT_BYTES: usize = 64 * 1024;

const MAX_OUTCOME_BYTES: usize = 8 * 1024;
const MAX_DETAIL_BYTES: usize = 4 * 1024;
const MAX_LABEL_BYTES: usize = 512;
const MAX_REFERENCE_BYTES: usize = 2 * 1024;
const MAX_SOURCE_STATE_BYTES: usize = 512;
const MAX_ITEMS: usize = 50;

/// A complete, inspectable handoff for a finished agent job (`kind:43004`).
///
/// The payload deliberately carries references and provenance, not embedded
/// local file contents. Producers should upload files separately or point at a
/// repository, canvas, workflow run, build, or deployment that readers can
/// inspect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobResultPayload {
    /// Payload contract version. Currently [`JOB_RESULT_SCHEMA_VERSION`].
    pub schema_version: u8,
    /// Hex event id of the originating `kind:43001` job request.
    pub job_request: String,
    /// The outcome the requester asked for.
    pub requested_outcome: String,
    /// Concise final outcome.
    pub outcome: String,
    /// Last meaningful progress or phase reached before the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_progress: Option<String>,
    /// Final state of the work.
    pub disposition: JobDisposition,
    /// Inspectable artifacts or proof produced by the job.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<JobArtifact>,
    /// Verification performed against the result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verification: Vec<JobVerification>,
    /// Concrete blocker when the result is blocked or partial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

impl JobResultPayload {
    /// Validate the payload contract, including cross-field invariants.
    pub fn validate(&self) -> Result<(), JobResultError> {
        if self.schema_version != JOB_RESULT_SCHEMA_VERSION {
            return Err(JobResultError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        validate_event_id(&self.job_request)?;
        validate_required_text(
            "requestedOutcome",
            &self.requested_outcome,
            MAX_OUTCOME_BYTES,
        )?;
        validate_required_text("outcome", &self.outcome, MAX_OUTCOME_BYTES)?;
        validate_optional_text(
            "lastProgress",
            self.last_progress.as_deref(),
            MAX_DETAIL_BYTES,
        )?;
        validate_optional_text("blocker", self.blocker.as_deref(), MAX_DETAIL_BYTES)?;

        if self.artifacts.len() > MAX_ITEMS {
            return Err(JobResultError::TooManyItems {
                field: "artifacts",
                max: MAX_ITEMS,
                got: self.artifacts.len(),
            });
        }
        if self.verification.len() > MAX_ITEMS {
            return Err(JobResultError::TooManyItems {
                field: "verification",
                max: MAX_ITEMS,
                got: self.verification.len(),
            });
        }
        for artifact in &self.artifacts {
            artifact.validate()?;
        }
        for verification in &self.verification {
            verification.validate()?;
        }

        match self.disposition {
            JobDisposition::Completed if self.artifacts.is_empty() => {
                return Err(JobResultError::InvalidCombination(
                    "completed results require an artifact; use no_artifact for analytical work"
                        .into(),
                ));
            }
            JobDisposition::NoArtifact if !self.artifacts.is_empty() => {
                return Err(JobResultError::InvalidCombination(
                    "no_artifact results cannot include artifacts".into(),
                ));
            }
            JobDisposition::Blocked
                if self
                    .blocker
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or_default()
                    .is_empty() =>
            {
                return Err(JobResultError::InvalidCombination(
                    "blocked results require a blocker".into(),
                ));
            }
            _ => {}
        }

        let size = serde_json::to_vec(self)
            .map_err(|error| JobResultError::Serialization(error.to_string()))?
            .len();
        if size > MAX_JOB_RESULT_BYTES {
            return Err(JobResultError::PayloadTooLarge {
                max: MAX_JOB_RESULT_BYTES,
                got: size,
            });
        }
        Ok(())
    }

    /// Validate and serialize the payload to its canonical JSON content.
    pub fn to_json(&self) -> Result<String, JobResultError> {
        self.validate()?;
        serde_json::to_string(self)
            .map_err(|error| JobResultError::Serialization(error.to_string()))
    }
}

/// Final disposition of an agent job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobDisposition {
    /// Requested work and its verification completed.
    Completed,
    /// Useful work exists, but the requested outcome is not fully complete.
    Partial,
    /// Work cannot continue until the named blocker is resolved.
    Blocked,
    /// The requested work failed.
    Failed,
    /// The work completed without a durable artifact, such as an analysis-only result.
    NoArtifact,
}

impl JobDisposition {
    /// Stable wire value used in event tags and UI labels.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::NoArtifact => "no_artifact",
        }
    }
}

/// A durable artifact or proof reference produced by a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobArtifact {
    /// Artifact category.
    pub kind: JobArtifactKind,
    /// Reader-facing label.
    pub label: String,
    /// URL, Buzz reference, repository-relative path, ref, or object id.
    pub reference: String,
    /// Optional source commit, branch, workflow run, build id, or equivalent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_state: Option<String>,
}

impl JobArtifact {
    fn validate(&self) -> Result<(), JobResultError> {
        validate_required_single_line_text("artifact.label", &self.label, MAX_LABEL_BYTES)?;
        validate_required_single_line_text(
            "artifact.reference",
            &self.reference,
            MAX_REFERENCE_BYTES,
        )?;
        validate_optional_single_line_text(
            "artifact.sourceState",
            self.source_state.as_deref(),
            MAX_SOURCE_STATE_BYTES,
        )?;

        let is_url = Url::parse(&self.reference).is_ok();
        if is_url {
            validate_reference_url(&self.reference)?;
        }
        match self.kind {
            JobArtifactKind::PullRequest
            | JobArtifactKind::Build
            | JobArtifactKind::Deployment
            | JobArtifactKind::Link
            | JobArtifactKind::Media
                if !is_url =>
            {
                validate_reference_url(&self.reference)?
            }
            JobArtifactKind::File if !is_url => validate_file_reference(&self.reference)?,
            JobArtifactKind::Commit if !is_url => validate_commit_reference(&self.reference)?,
            JobArtifactKind::PullRequest
            | JobArtifactKind::Build
            | JobArtifactKind::Deployment
            | JobArtifactKind::Link
            | JobArtifactKind::Media
            | JobArtifactKind::File
            | JobArtifactKind::Commit => {}
            JobArtifactKind::Branch
            | JobArtifactKind::Canvas
            | JobArtifactKind::WorkflowOutput
            | JobArtifactKind::Other => {}
        }
        Ok(())
    }
}

/// Supported artifact reference categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobArtifactKind {
    /// Uploaded file or repository-relative file.
    File,
    /// Uploaded media.
    Media,
    /// Repository branch.
    Branch,
    /// Full commit object id or commit URL.
    Commit,
    /// Pull request URL.
    PullRequest,
    /// Buzz channel canvas reference.
    Canvas,
    /// Workflow output or run.
    WorkflowOutput,
    /// Build result.
    Build,
    /// Deployment proof.
    Deployment,
    /// Provenance-bearing link.
    Link,
    /// Explicitly labeled artifact outside the predefined categories.
    Other,
}

impl JobArtifactKind {
    /// Stable wire value used by clients.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Media => "media",
            Self::Branch => "branch",
            Self::Commit => "commit",
            Self::PullRequest => "pull_request",
            Self::Canvas => "canvas",
            Self::WorkflowOutput => "workflow_output",
            Self::Build => "build",
            Self::Deployment => "deployment",
            Self::Link => "link",
            Self::Other => "other",
        }
    }
}

/// One verification result attached to a job handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobVerification {
    /// Check or review performed.
    pub label: String,
    /// Verification status.
    pub status: JobVerificationStatus,
    /// Optional command, result URL, or concise evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

impl JobVerification {
    fn validate(&self) -> Result<(), JobResultError> {
        validate_required_single_line_text("verification.label", &self.label, MAX_LABEL_BYTES)?;
        validate_optional_text(
            "verification.evidence",
            self.evidence.as_deref(),
            MAX_REFERENCE_BYTES,
        )
    }
}

/// Status of one verification check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobVerificationStatus {
    /// Verification ran and passed.
    Passed,
    /// Verification ran and failed.
    Failed,
    /// Verification was intentionally or unavoidably not run.
    NotRun,
}

impl JobVerificationStatus {
    /// Stable wire value used by clients.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::NotRun => "not_run",
        }
    }
}

/// Validation and serialization errors for a structured job result.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobResultError {
    /// Only the current schema version is accepted by this producer.
    #[error("unsupported job result schema version {0}")]
    UnsupportedSchemaVersion(u8),
    /// The job request reference is not a Nostr event id.
    #[error("jobRequest must be a 64-character hex event id")]
    InvalidJobRequest,
    /// A required field was empty.
    #[error("{0} must not be empty")]
    EmptyField(&'static str),
    /// A text field exceeded its limit.
    #[error("{field} exceeds {max} bytes (got {got})")]
    FieldTooLarge {
        /// Field name.
        field: &'static str,
        /// Maximum allowed bytes.
        max: usize,
        /// Actual byte count.
        got: usize,
    },
    /// A text field contains control characters.
    #[error("{0} must not contain control characters")]
    ControlCharacters(&'static str),
    /// An array exceeded its item limit.
    #[error("{field} exceeds {max} items (got {got})")]
    TooManyItems {
        /// Field name.
        field: &'static str,
        /// Maximum allowed items.
        max: usize,
        /// Actual item count.
        got: usize,
    },
    /// A URL-like reference was invalid or used an unsupported scheme.
    #[error("invalid artifact reference: {0}")]
    InvalidReference(String),
    /// Two otherwise valid fields conflict.
    #[error("invalid job result combination: {0}")]
    InvalidCombination(String),
    /// The serialized payload exceeded the event content limit.
    #[error("job result payload exceeds {max} bytes (got {got})")]
    PayloadTooLarge {
        /// Maximum allowed bytes.
        max: usize,
        /// Actual byte count.
        got: usize,
    },
    /// JSON serialization failed.
    #[error("failed to serialize job result: {0}")]
    Serialization(String),
}

fn validate_event_id(value: &str) -> Result<(), JobResultError> {
    if value.len() != 64 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err(JobResultError::InvalidJobRequest);
    }
    Ok(())
}

fn validate_required_text(
    field: &'static str,
    value: &str,
    max: usize,
) -> Result<(), JobResultError> {
    if value.trim().is_empty() {
        return Err(JobResultError::EmptyField(field));
    }
    validate_text(field, value, max)
}

fn validate_optional_text(
    field: &'static str,
    value: Option<&str>,
    max: usize,
) -> Result<(), JobResultError> {
    if let Some(value) = value {
        validate_required_text(field, value, max)?;
    }
    Ok(())
}

fn validate_required_single_line_text(
    field: &'static str,
    value: &str,
    max: usize,
) -> Result<(), JobResultError> {
    validate_required_text(field, value, max)?;
    if value.chars().any(char::is_control) {
        return Err(JobResultError::ControlCharacters(field));
    }
    Ok(())
}

fn validate_optional_single_line_text(
    field: &'static str,
    value: Option<&str>,
    max: usize,
) -> Result<(), JobResultError> {
    if let Some(value) = value {
        validate_required_single_line_text(field, value, max)?;
    }
    Ok(())
}

fn validate_text(field: &'static str, value: &str, max: usize) -> Result<(), JobResultError> {
    if value.len() > max {
        return Err(JobResultError::FieldTooLarge {
            field,
            max,
            got: value.len(),
        });
    }
    if value
        .chars()
        .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(JobResultError::ControlCharacters(field));
    }
    Ok(())
}

fn validate_reference_url(value: &str) -> Result<(), JobResultError> {
    let url =
        Url::parse(value).map_err(|error| JobResultError::InvalidReference(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https" | "buzz" | "nostr") {
        return Err(JobResultError::InvalidReference(format!(
            "unsupported URL scheme {:?}",
            url.scheme()
        )));
    }
    if matches!(url.scheme(), "http" | "https") && url.host_str().is_none() {
        return Err(JobResultError::InvalidReference(
            "HTTP(S) references require a host".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(JobResultError::InvalidReference(
            "artifact references must not contain URL credentials".into(),
        ));
    }
    Ok(())
}

fn validate_file_reference(value: &str) -> Result<(), JobResultError> {
    if Url::parse(value).is_ok() {
        return validate_reference_url(value);
    }
    if value.starts_with('/')
        || value.starts_with('\\')
        || value.starts_with("~/")
        || value.starts_with("~\\")
        || value.as_bytes().get(1) == Some(&b':')
    {
        return Err(JobResultError::InvalidReference(
            "file references must be uploaded URLs or repository-relative paths".into(),
        ));
    }
    if value.split(['/', '\\']).any(|component| component == "..") {
        return Err(JobResultError::InvalidReference(
            "file references must not traverse outside the repository".into(),
        ));
    }
    Ok(())
}

fn validate_commit_reference(value: &str) -> Result<(), JobResultError> {
    if Url::parse(value).is_ok() {
        return validate_reference_url(value);
    }
    if matches!(value.len(), 40 | 64)
        && value.chars().all(|character| character.is_ascii_hexdigit())
    {
        return Ok(());
    }
    Err(JobResultError::InvalidReference(
        "commit references must be a full 40/64-character object id or URL".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(kind: JobArtifactKind, reference: &str) -> JobArtifact {
        JobArtifact {
            kind,
            label: "Primary artifact".into(),
            reference: reference.into(),
            source_state: Some("abc1234".into()),
        }
    }

    fn payload(disposition: JobDisposition) -> JobResultPayload {
        JobResultPayload {
            schema_version: JOB_RESULT_SCHEMA_VERSION,
            job_request: "a".repeat(64),
            requested_outcome: "Ship an inspectable result".into(),
            outcome: "The result is ready for review.".into(),
            last_progress: Some("Verification completed.".into()),
            disposition,
            artifacts: vec![artifact(
                JobArtifactKind::PullRequest,
                "https://github.com/block/buzz/pull/1",
            )],
            verification: vec![JobVerification {
                label: "just ci".into(),
                status: JobVerificationStatus::Passed,
                evidence: Some("exit 0".into()),
            }],
            blocker: None,
        }
    }

    #[test]
    fn result_payload_round_trips() {
        let original = payload(JobDisposition::Completed);
        let json = original.to_json().expect("serialize");
        let decoded: JobResultPayload = serde_json::from_str(&json).expect("parse");
        assert_eq!(decoded, original);
        assert!(json.contains(r#""schemaVersion":1"#));
        assert!(json.contains(r#""pull_request""#));
    }

    #[test]
    fn additive_fields_are_ignored_within_the_current_schema() {
        let mut value = serde_json::to_value(payload(JobDisposition::Completed)).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("futureField".into(), serde_json::json!("ignored"));
        value["artifacts"][0]
            .as_object_mut()
            .unwrap()
            .insert("futureArtifactField".into(), serde_json::json!(true));

        let decoded: JobResultPayload = serde_json::from_value(value).expect("forward compatible");
        decoded.validate().expect("known fields remain valid");
    }

    #[test]
    fn all_artifact_kinds_validate() {
        let cases = [
            (JobArtifactKind::File, "src/lib.rs"),
            (JobArtifactKind::Media, "https://example.com/result.png"),
            (JobArtifactKind::Branch, "agent/job-result-handoff"),
            (
                JobArtifactKind::Commit,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            (
                JobArtifactKind::PullRequest,
                "https://github.com/block/buzz/pull/1",
            ),
            (
                JobArtifactKind::Canvas,
                "buzz://canvas?channel=550e8400-e29b-41d4-a716-446655440000",
            ),
            (JobArtifactKind::WorkflowOutput, "run-123"),
            (JobArtifactKind::Build, "https://example.com/build/123"),
            (
                JobArtifactKind::Deployment,
                "https://example.com/deploy/123",
            ),
            (JobArtifactKind::Link, "https://example.com/report"),
            (JobArtifactKind::Other, "signed-note-123"),
        ];

        for (kind, reference) in cases {
            let mut candidate = payload(JobDisposition::Completed);
            candidate.artifacts = vec![artifact(kind, reference)];
            candidate.validate().unwrap_or_else(|error| {
                panic!("{kind:?} reference {reference:?} should validate: {error}")
            });
        }
    }

    #[test]
    fn analytical_result_requires_explicit_no_artifact_disposition() {
        let mut completed = payload(JobDisposition::Completed);
        completed.artifacts.clear();
        assert!(matches!(
            completed.validate(),
            Err(JobResultError::InvalidCombination(_))
        ));

        completed.disposition = JobDisposition::NoArtifact;
        completed.validate().expect("explicit no-artifact result");
    }

    #[test]
    fn no_artifact_rejects_artifact_list() {
        let candidate = payload(JobDisposition::NoArtifact);
        assert!(matches!(
            candidate.validate(),
            Err(JobResultError::InvalidCombination(_))
        ));
    }

    #[test]
    fn blocked_result_requires_blocker() {
        let mut candidate = payload(JobDisposition::Blocked);
        candidate.artifacts.clear();
        assert!(matches!(
            candidate.validate(),
            Err(JobResultError::InvalidCombination(_))
        ));
        candidate.blocker = Some("Maintainer decision required.".into());
        candidate.validate().expect("blocked with blocker");
    }

    #[test]
    fn absolute_local_file_paths_are_rejected() {
        for reference in [
            "/Users/alice/private.txt",
            r"\Users\alice\private.txt",
            r"\\server\share\private.txt",
            r"C:\Users\alice\private.txt",
        ] {
            let mut candidate = payload(JobDisposition::Completed);
            candidate.artifacts = vec![artifact(JobArtifactKind::File, reference)];
            assert!(
                matches!(
                    candidate.validate(),
                    Err(JobResultError::InvalidReference(_))
                ),
                "{reference:?} should be rejected"
            );
        }
    }

    #[test]
    fn unsupported_link_schemes_are_rejected() {
        let mut candidate = payload(JobDisposition::Completed);
        candidate.artifacts = vec![artifact(JobArtifactKind::Link, "file:///tmp/result")];
        assert!(matches!(
            candidate.validate(),
            Err(JobResultError::InvalidReference(_))
        ));
    }

    #[test]
    fn link_references_with_credentials_or_missing_hosts_are_rejected() {
        for reference in ["https://user:secret@example.com/report", "https://"] {
            let mut candidate = payload(JobDisposition::Completed);
            candidate.artifacts = vec![artifact(JobArtifactKind::Link, reference)];
            assert!(
                matches!(
                    candidate.validate(),
                    Err(JobResultError::InvalidReference(_))
                ),
                "{reference:?} should be rejected"
            );
        }
    }

    #[test]
    fn credential_bearing_urls_are_rejected_for_non_url_artifact_kinds() {
        for kind in [
            JobArtifactKind::Branch,
            JobArtifactKind::Canvas,
            JobArtifactKind::WorkflowOutput,
            JobArtifactKind::Other,
        ] {
            let mut candidate = payload(JobDisposition::Completed);
            candidate.artifacts = vec![artifact(kind, "https://user:secret@example.com/artifact")];
            assert!(
                matches!(
                    candidate.validate(),
                    Err(JobResultError::InvalidReference(_))
                ),
                "{kind:?} should reject URL credentials"
            );
        }
    }

    #[test]
    fn repository_file_references_cannot_traverse_upward() {
        for reference in [
            "../private.txt",
            "docs/../../private.txt",
            r"..\private.txt",
        ] {
            let mut candidate = payload(JobDisposition::Completed);
            candidate.artifacts = vec![artifact(JobArtifactKind::File, reference)];
            assert!(
                matches!(
                    candidate.validate(),
                    Err(JobResultError::InvalidReference(_))
                ),
                "{reference:?} should be rejected"
            );
        }
    }

    #[test]
    fn multiline_artifact_references_are_rejected() {
        let mut candidate = payload(JobDisposition::Completed);
        candidate.artifacts = vec![artifact(JobArtifactKind::Other, "run-123\nprivate-note")];
        assert!(matches!(
            candidate.validate(),
            Err(JobResultError::ControlCharacters("artifact.reference"))
        ));
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let mut candidate = payload(JobDisposition::Completed);
        candidate.schema_version = 2;
        assert_eq!(
            candidate.validate(),
            Err(JobResultError::UnsupportedSchemaVersion(2))
        );
    }

    #[test]
    fn invalid_job_request_is_rejected() {
        let mut candidate = payload(JobDisposition::Completed);
        candidate.job_request = "not-an-event".into();
        assert_eq!(candidate.validate(), Err(JobResultError::InvalidJobRequest));
    }

    #[test]
    fn too_many_artifacts_are_rejected() {
        let mut candidate = payload(JobDisposition::Completed);
        candidate.artifacts = vec![artifact(JobArtifactKind::File, "src/lib.rs"); MAX_ITEMS + 1];
        assert!(matches!(
            candidate.validate(),
            Err(JobResultError::TooManyItems {
                field: "artifacts",
                ..
            })
        ));
    }

    #[test]
    fn payload_size_limit_is_enforced() {
        let mut candidate = payload(JobDisposition::Completed);
        candidate.artifacts = (0..MAX_ITEMS)
            .map(|index| JobArtifact {
                kind: JobArtifactKind::Other,
                label: format!("artifact-{index}"),
                reference: "x".repeat(MAX_REFERENCE_BYTES),
                source_state: None,
            })
            .collect();
        assert!(matches!(
            candidate.validate(),
            Err(JobResultError::PayloadTooLarge { .. })
        ));
    }
}
