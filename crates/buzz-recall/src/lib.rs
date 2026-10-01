#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-recall` — 9 IDE Transcript Parsers & Retroactive Ingestion for Orbit.
//!
//! Day-one knowledge bootstrapping from existing agent transcripts. Retroactively parses,
//! normalizes, and ingests past session transcripts from 9 AI coding agent harnesses:
//!
//! 1. **Antigravity IDE** (`~/.gemini/antigravity-ide/brain/*/transcript.jsonl`)
//! 2. **Claude Code** (`~/.claude/projects/**/`)
//! 3. **Codex** (`~/.codex/conversations/`)
//! 4. **Cursor** (`~/.cursor/User/workspaceStorage/`)
//! 5. **Goose** (`~/.config/goose/sessions/`)
//! 6. **OpenCode** (`~/.opencode/sessions/`)
//! 7. **ZCode** (`~/.zcode/conversations/`)
//! 8. **AGY CLI** (`~/.gemini/transcripts/`)
//! 9. **Kimi** (`~/.kimi/chats/`)
//!
//! Ingested raw transcripts are cached under `orbit_brain/transcripts/` and vectorized
//! into Orbit's embedded SQLite and LanceDB storage.

/// Semantic session chunker.
pub mod chunker;
/// Error definitions for recall operations.
pub mod error;
/// Normalizer and secret redactor for multi-turn agent transcripts.
pub mod normalizer;
/// Background synchronization and orchestration engine.
pub mod orchestrator;
/// Implementations of the 9 IDE transcript parsers.
pub mod parsers;
/// Policy engine for transcript storage and context filtering.
pub mod policy;

pub use chunker::SessionChunker;
pub use error::{RecallError, Result};
pub use normalizer::{extract_session_decisions, normalize_and_redact_transcript};
pub use orchestrator::{RecallIngestSummary, RecallOrchestrator};
pub use parsers::{
    AgyCliRecallPlugin, AntigravityRecallPlugin, ClaudeCodeRecallPlugin, CodexRecallPlugin,
    CursorRecallPlugin, GooseRecallPlugin, KimiRecallPlugin, OpenCodeRecallPlugin,
    ZCodeRecallPlugin,
};
pub use policy::{
    create_syncable_summary, filter_policy_eligible_chunks, is_chunk_policy_eligible,
    PolicyContextQuery, SessionPolicy,
};
