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

use std::{fs::File, io::Read as _, path::Path};

pub mod error;
pub mod hermes;
pub mod kimi;
pub mod launch;
pub mod model;

pub use error::ConfigError;
pub use launch::{to_launch_json, LaunchMapError, McpLaunchConfigDocument};
pub use model::{
    McpServerConfig, McpServerInventoryEntry, RuntimeKind, RuntimeMcpConfig, RuntimeMcpInventory,
    ValidationIssue,
};

/// Maximum native runtime configuration size accepted by an adapter.
pub const RUNTIME_CONFIG_MAX_BYTES: u64 = 1024 * 1024;

fn read_config(path: &Path) -> Result<String, ConfigError> {
    let file = File::open(path)?;
    let mut content = String::new();
    file.take(RUNTIME_CONFIG_MAX_BYTES + 1)
        .read_to_string(&mut content)?;
    validate_config_size(content.len())?;
    Ok(content)
}

fn validate_config_size(size: usize) -> Result<(), ConfigError> {
    if size as u64 > RUNTIME_CONFIG_MAX_BYTES {
        return Err(ConfigError::TooLarge {
            limit: RUNTIME_CONFIG_MAX_BYTES,
        });
    }
    Ok(())
}
