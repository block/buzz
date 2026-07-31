//! Minimal provider config for model discovery.
//!
//! This is the surviving slice of the pre-goose `buzz-agent/src/config.rs`
//! (2,709 lines). Everything else in that file existed to *implement* provider
//! configuration for the agent loop — provider enums, base URLs, model-name
//! normalization, OpenAI upgrade rules — and goose owns all of it now.
//!
//! What remains is only what Databricks model discovery needs, with the exact
//! names `desktop/src-tauri` matches on
//! (`commands/agent_models.rs:755-785`).

/// Provider families the desktop model picker can discover models for.
///
/// Names preserved verbatim: the desktop constructs these variants directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Provider {
    Anthropic,
    #[default]
    OpenAi,
    /// Databricks model serving (`api/2.0/serving-endpoints`).
    Databricks,
    /// Databricks AI Gateway v2.
    DatabricksV2,
}

/// Credentials and host for a discovery call.
#[derive(Debug, Clone)]
pub struct Config {
    pub provider: Provider,
    /// Static bearer. Empty means "try the PKCE cache, but never open a
    /// browser" — see [`crate::catalog::discover_databricks_models`].
    pub api_key: String,
    /// Provider host, e.g. `DATABRICKS_HOST`.
    pub base_url: String,
}

impl Config {
    /// Signature preserved from the pre-goose crate — the desktop calls this
    /// directly (`commands/agent_models.rs:785`).
    pub fn for_discovery(provider: Provider, api_key: String, base_url: String) -> Self {
        Self {
            provider,
            api_key,
            base_url,
        }
    }
}
