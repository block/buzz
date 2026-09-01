//! Error type shared across the fold engine.

/// Errors surfaced by the fold engine.
///
/// There is deliberately no "model output rejected" variant: the paid-for
/// response persists verbatim as the artifact, and provenance is
/// engine-computed from the plan, never parsed out of the output.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The fold spec is invalid (bad name, empty selection, blank field, …).
    #[error("invalid fold spec: {0}")]
    InvalidSpec(String),

    /// The model runner failed (missing binary, timeout, crash, auth sentinel).
    #[error("fold runner failed: {0}")]
    Runner(String),
}
