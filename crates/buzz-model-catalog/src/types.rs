//! Error type shared by the token sources and the discovery calls.
//!
//! Kept structurally identical to `buzz_agent::types::AgentError` because
//! `desktop/src-tauri` matches on the `LlmAuth` variant by name to decide
//! whether to fall back to a subprocess probe
//! (`commands/agent_models.rs:793`).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentError {
    InvalidParams(String),
    Llm(String),
    /// No usable credential. The desktop treats this as "fall through to the
    /// subprocess probe", never as a hard failure — so discovery must return
    /// it rather than hanging or opening a browser.
    LlmAuth(String),
    LlmModelNotFound(String),
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidParams(s) => write!(f, "invalid params: {s}"),
            Self::Llm(s) => write!(f, "llm: {s}"),
            Self::LlmAuth(s) => write!(f, "llm auth: {s}"),
            Self::LlmModelNotFound(s) => write!(f, "llm model not found: {s}"),
        }
    }
}

impl std::error::Error for AgentError {}
