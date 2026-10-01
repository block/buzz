#![deny(unsafe_code)]
//! Recall plugin trait definition for retroactive agent transcript ingestion.

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use crate::models::RawDocument;

/// Plugin interface for retroactively parsing past agent transcripts.
#[async_trait]
pub trait RecallPlugin: Send + Sync {
    /// Name of the agent harness (e.g. "antigravity", "claude_code", "opencode").
    fn name(&self) -> &'static str;

    /// Detect if the agent's data directory exists on the workstation.
    fn detect(&self) -> bool;

    /// Default storage location for this agent harness on the current system, if known.
    fn default_location(&self) -> Option<PathBuf> {
        None
    }

    /// Discover and parse sessions for the specified workspace path.
    async fn ingest_sessions(&self, workspace_path: &Path) -> Vec<RawDocument>;
}
