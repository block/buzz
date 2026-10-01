#![deny(unsafe_code)]
//! Source plugin trait definition for live and streaming external sources.

use async_trait::async_trait;
use std::path::Path;
use crate::error::Result;
use crate::models::RawDocument;

/// Source plugin trait for live, streaming, or external system ingestion.
#[async_trait]
pub trait SourcePlugin: Send + Sync {
    /// Identifier name of the source plugin.
    fn name(&self) -> &'static str;

    /// Connect and verify credentials/permissions for this source.
    async fn connect(&self) -> Result<()>;

    /// Ingest available documents from the source for a given workspace path.
    async fn pull_updates(&self, workspace_path: &Path) -> Result<Vec<RawDocument>>;
}
