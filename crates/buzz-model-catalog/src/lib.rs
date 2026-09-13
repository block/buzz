#![forbid(unsafe_code)]
//! Databricks model discovery, split out of `buzz-agent`.
//!
//! # Why this crate exists
//!
//! When `buzz-agent`'s loop moved onto the goose library, `desktop/src-tauri`
//! could no longer link it. Goose pulls `sqlx-sqlite` → `libsqlite3-sys 0.30`;
//! the desktop already has `rusqlite 0.37` → `libsqlite3-sys 0.35`. Both
//! declare `links = "sqlite3"`, and Cargo refuses to build two packages that
//! link the same native library. No version pin resolves that.
//!
//! The desktop never needed the agent loop anyway — only the model picker's
//! Databricks discovery (`commands/agent_models.rs:791`). So that code, and
//! the token sources it depends on, live here: no goose, no sqlite, no agent.

pub mod auth;
pub mod catalog;
pub mod config;
pub mod model_capabilities;
pub mod types;

pub use catalog::{
    discover_databricks_models, discover_databricks_models_with_cache_dir, ModelEntry,
};
pub use config::{Config, Provider, ThinkingEffort};
pub use types::AgentError;

/// PKCE configuration for Databricks OAuth against `host`.
///
/// Shared so the desktop model picker and `buzz-agent auth databricks` cache
/// tokens in the same place.
pub fn databricks_pkce_config(host: &str) -> auth::PkceOAuthConfig {
    databricks_pkce_config_with_cache_dir(host, None)
}

/// PKCE configuration with an optional explicit credential cache root.
pub fn databricks_pkce_config_with_cache_dir(
    host: &str,
    cache_dir: Option<&std::path::Path>,
) -> auth::PkceOAuthConfig {
    auth::PkceOAuthConfig {
        discovery_url: format!(
            "{}/oidc/.well-known/oauth-authorization-server",
            host.trim_end_matches('/')
        ),
        client_id: "databricks-cli".into(),
        scopes: vec!["all-apis".into(), "offline_access".into()],
        cache_namespace: "databricks".into(),
        cache_dir_override: cache_dir.map(std::path::Path::to_path_buf),
    }
}

/// Run the interactive Databricks OAuth PKCE login and cache the token.
///
/// Needs a browser on the machine. The desktop model picker calls this when
/// discovery fails with an auth error.
pub async fn authenticate_databricks(host: &str) -> Result<(), AgentError> {
    authenticate_databricks_with_cache_dir(host, None).await
}

/// Run Databricks OAuth using an optional explicit credential cache root.
pub async fn authenticate_databricks_with_cache_dir(
    host: &str,
    cache_dir: Option<&std::path::Path>,
) -> Result<(), AgentError> {
    auth::PkceOAuthTokenSource::new(databricks_pkce_config_with_cache_dir(host, cache_dir))?
        .interactive_login()
        .await
}

/// Environment keys the Windows Git Bash resolver may inspect.
///
/// The MCP child is spawned with an otherwise-cleared environment, so every
/// key here must be forwarded or a ready agent cannot start its shell tool.
/// Doctor checks the same contract, which is why this is one shared constant
/// rather than two lists that can drift
/// (`desktop/src-tauri/src/managed_agents/git_bash.rs:136`).
///
/// Parked in this crate because it is the only agent-side crate the desktop
/// can still link.
#[cfg(windows)]
pub const WINDOWS_SHELL_RESOLUTION_ENV: &[&str] = &[
    "PATH",
    "BUZZ_SHELL",
    "GIT_BASH",
    "SystemRoot",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "LOCALAPPDATA",
];
