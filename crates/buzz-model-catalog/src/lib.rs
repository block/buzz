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
pub mod types;

pub use catalog::{discover_databricks_models, ModelEntry, DATABRICKS_V2_KNOWN_MODELS};
pub use config::{Config, Provider};
pub use types::AgentError;

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
