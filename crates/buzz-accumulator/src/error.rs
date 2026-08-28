//! Error type shared across the fold engine.

/// Errors surfaced by the fold engine.
///
/// A [`Error::Nonconforming`] means the model's output broke the artifact
/// contract — the run is refused and nothing may be persisted. This is a
/// deliberate design rule: never build a validator that a model can only
/// satisfy by lying; every rule enforced here is one the model can genuinely
/// meet.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The fold spec is invalid (bad name, empty selection, unknown schema, …).
    #[error("invalid fold spec: {0}")]
    InvalidSpec(String),

    /// Model output does not conform to the artifact contract; nothing persists.
    #[error("nonconforming model output: {0}")]
    Nonconforming(String),

    /// The model runner failed (missing binary, timeout, crash, auth sentinel).
    #[error("fold runner failed: {0}")]
    Runner(String),
}
