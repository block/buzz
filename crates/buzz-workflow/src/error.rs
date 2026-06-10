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

    /// The action is defined but not yet implemented.
    #[error("action not implemented: {0}")]
    NotImplemented(String),
}

impl From<sprout_db::error::DbError> for WorkflowError {
    fn from(e: sprout_db::error::DbError) -> Self {
        WorkflowError::Database(e.to_string())
    }
}
