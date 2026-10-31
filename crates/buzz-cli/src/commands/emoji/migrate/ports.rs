//! Network boundaries of the migration, behind traits so tests fake only
//! HTTP fetch, the uploader and the relay.

use std::time::Duration;

/// Why a port call failed. The class decides what the pipeline does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    /// Worth retrying; `retry_after` is a server hint that also pauses the stage's rate bucket.
    Transient {
        message: String,
        retry_after: Option<Duration>,
    },
    /// This item cannot succeed; reject it.
    Permanent(String),
    /// Every item would fail the same way (bad credentials); stop the run.
    Fatal(String),
}

impl PortError {
    pub fn transient(message: impl Into<String>) -> Self {
        Self::Transient {
            message: message.into(),
            retry_after: None,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Transient { message, .. } => message,
            Self::Permanent(m) | Self::Fatal(m) => m,
        }
    }
}

/// A blob the relay stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedBlob {
    pub url: String,
    pub sha256: String,
}

/// One catalog entry (kind 30180) as the relay returned it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogRow {
    pub name: String,
    pub url: String,
    pub author: String,
    pub event_id: String,
    pub other_authors: Vec<String>,
}

/// Downloads one image source (`http(s)://` or `file://`) with a size cap.
pub trait Fetcher {
    async fn fetch(&self, source: &str) -> Result<Vec<u8>, PortError>;
}

/// Stores sanitized bytes as a Blossom blob. Idempotent by content hash.
pub trait Uploader {
    async fn upload(&self, bytes: Vec<u8>) -> Result<UploadedBlob, PortError>;
}

/// Relay reads and writes of catalog entries.
pub trait Relay {
    /// Hex public key entries are published under.
    fn pubkey(&self) -> String;
    /// Publish one entry. `Ok` only when the relay answered `accepted: true`.
    async fn publish(&self, name: &str, url: &str) -> Result<String, PortError>;
    /// Exact `#d` lookup for at most 50 normalized names. Newest event wins per name.
    async fn lookup(&self, names: &[String]) -> Result<Vec<CatalogRow>, PortError>;
    /// Prefix search over entry search text.
    async fn search(&self, prefix: &str, limit: u32) -> Result<Vec<CatalogRow>, PortError>;
    /// Delete the caller's entry for `name` (kind 5, `a` tag).
    async fn delete(&self, name: &str) -> Result<(), PortError>;
    /// Whether the blob answers a `HEAD`.
    async fn blob_exists(&self, url: &str) -> Result<bool, PortError>;
}
