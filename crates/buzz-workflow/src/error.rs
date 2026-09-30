//! Workflow error types.

use thiserror::Error;

/// Partial execution progress captured when a workflow step fails mid-run.
///
/// This allows callers to persist whatever trace was accumulated before the
/// error, rather than losing it when the in-memory `Vec` is dropped.
#[derive(Debug, Default)]
pub struct PartialProgress {
    /// Index of the step that failed (0-based).
    pub step_index: usize,
    /// Trace entries for steps completed/skipped before the failure.
    pub trace: Vec<serde_json::Value>,
}

/// Errors produced by the workflow engine.
#[derive(Debug, Error)]
pub enum WorkflowError {
    /// The workflow YAML/JSON could not be parsed.
    #[error("invalid YAML: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),

    /// The workflow definition violates a semantic invariant.
    #[error("invalid definition: {0}")]
    InvalidDefinition(String),

    /// An `if:` condition expression could not be evaluated.
    #[error("condition evaluation error: {0}")]
    ConditionError(String),

    /// A template variable substitution failed.
    #[error("template error: {0}")]
    TemplateError(String),

    /// A step exceeded its configured timeout.
    #[error("step '{step_id}' timed out after {timeout_secs}s")]
    StepTimeout {
        /// The ID of the step that timed out.
        step_id: String,
        /// The timeout limit in seconds.
        timeout_secs: u64,
    },

    /// An outbound webhook call failed.
    #[error("webhook error: {0}")]
    WebhookError(String),

    /// The engine's concurrency limit was reached.
    #[error("capacity exceeded")]
    CapacityExceeded,

    /// A database operation failed.
    #[error("database error: {0}")]
    Database(String),

    /// The workflow's owner is not currently authorized to run it (removed
    /// from the channel, insufficient role for the definition's actions, or
    /// the authority lookup failed — all deny, fail-closed).
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// The action is defined but not yet implemented.
    #[error("action not implemented: {0}")]
    NotImplemented(String),
}

impl WorkflowError {
    /// Stable run-level classification. Diagnostics remain in `Display` output.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidYaml(_) => "invalid_yaml",
            Self::InvalidDefinition(_) => "invalid_definition",
            Self::ConditionError(_) => "condition_evaluation_failed",
            Self::TemplateError(_) => "template_resolution_failed",
            Self::StepTimeout { .. } => "step_timeout",
            Self::WebhookError(_) => "webhook_failed",
            Self::CapacityExceeded => "capacity_exceeded",
            Self::Database(_) => "database_error",
            Self::Unauthorized(_) => "owner_unauthorized",
            Self::NotImplemented(_) => "action_not_implemented",
        }
    }
}

impl From<buzz_db::error::DbError> for WorkflowError {
    fn from(e: buzz_db::error::DbError) -> Self {
        WorkflowError::Database(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::WorkflowError;

    #[test]
    fn workflow_error_codes_are_stable_and_separate_from_diagnostics() {
        let timeout = WorkflowError::StepTimeout {
            step_id: "notify".to_owned(),
            timeout_secs: 30,
        };
        assert_eq!(timeout.code(), "step_timeout");
        assert!(timeout.to_string().contains("notify"));

        let webhook = WorkflowError::WebhookError("secret-bearing detail".to_owned());
        assert_eq!(webhook.code(), "webhook_failed");
        assert!(!webhook.code().contains("secret-bearing detail"));

        assert_eq!(
            WorkflowError::NotImplemented("SendDm".to_owned()).code(),
            "action_not_implemented"
        );
    }
}
