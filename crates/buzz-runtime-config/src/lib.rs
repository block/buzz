//! Runtime-agnostic MCP/Skill configuration model.
//!
//! Buzz users run agents on multiple external frameworks ("runtimes") — e.g.
//! Hermes (`~/.hermes/config.yaml`) and Kimi Code (`~/.kimi-code/mcp.json`).
//! Each runtime keeps its own MCP server configuration in its own format.
//! This crate defines a unified, runtime-agnostic model for that
//! configuration plus read-only adapters that load and validate each
//! runtime's native config file.
//!
//! Read + validate only: adapters never write back to the runtime's config
//! file, and secret values (API tokens in `env`) are never logged or
//! persisted by this crate — see [`McpServerConfig::redacted`] for display.

pub mod error;
pub mod hermes;
pub mod kimi;
pub mod model;

pub use error::ConfigError;
pub use model::{McpServerConfig, RuntimeKind, RuntimeMcpConfig, ValidationIssue};
