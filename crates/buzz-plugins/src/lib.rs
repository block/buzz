#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-plugins` — Plugin traits and interfaces for Orbit.
//!
//! Provides the core `RecallPlugin` and `SourcePlugin` abstractions for retroactively
//! indexing past agent sessions and streaming external developer knowledge into Orbit.

/// Error definitions for plugins.
pub mod error;
/// Domain models for raw documents and structured conversation turns.
pub mod models;
/// Recall plugin trait for retroactive transcript ingestion across IDEs and agents.
pub mod recall;
/// Source plugin trait for live and streaming external sources.
pub mod source;

pub use error::{PluginError, Result};
pub use models::{RawDocument, SessionTurn, TurnRole};
pub use recall::RecallPlugin;
pub use source::SourcePlugin;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_document_hashing_and_conversion() {
        let turn = SessionTurn::user("Explain Rust memory model");
        let raw = RawDocument::new(
            "/path/to/transcript.jsonl",
            "/workspace/orbit",
            "User: Explain Rust memory model\nAssistant: Rust has ownership and lifetimes.",
            1_700_000_000_000_000_000,
            serde_json::json!({ "agent": "test" }),
            vec![turn],
        );

        assert_eq!(raw.turns.len(), 1);
        assert_eq!(raw.turns[0].role, TurnRole::User);
        assert!(!raw.content_hash.is_empty());

        let doc = raw.to_document();
        assert_eq!(doc.source_type, "session");
        assert_eq!(doc.source_uri, "/path/to/transcript.jsonl");
        assert_eq!(doc.workspace_path, "/workspace/orbit");
        assert_eq!(doc.content_hash, raw.content_hash);
        assert_eq!(doc.permissions["local_only"], true);
    }
}
