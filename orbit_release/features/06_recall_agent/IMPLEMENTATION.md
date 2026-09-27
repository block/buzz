# Feature 06 — Recall Agent (9 IDE Transcript Parsers & Retroactive Ingestion)

> **Priority**: P1 — Day-one knowledge bootstrapping from existing agent transcripts.  
> **Sprint**: Sprint 3 (Weeks 5–6)  
> **Dependencies**: Feature 01 (Storage), Feature 02 (Embeddings), Feature 03 (Ingestion)  
> **Crates**: `buzz-plugins` (plugin traits), `buzz-recall` (parser implementations)  
> **Environment Variables**: `BUZZ_DATA_DIR`, `BUZZ_RECALL_SYNC_INTERVAL_SEC`

---

## Overview

Feature 06 retroactively parses, normalizes, and ingests past session transcripts from AI coding agents so Orbit starts with a rich memory from day one. Ingested raw transcripts are cached under `orbit_brain/transcripts/` and vectorized into `buzz_chunks`.

---

## Supported IDE & Agent Parsers

| # | Agent / IDE | Parser Plugin | Default Storage Location | Format |
|---|-------------|---------------|--------------------------|--------|
| 1 | **Antigravity IDE** | `AntigravityRecallPlugin` | `~/.gemini/antigravity-ide/brain/*/transcript.jsonl` | JSONL with step objects |
| 2 | **Claude Code** | `ClaudeCodeRecallPlugin` | `~/.claude/projects/**/` | Session JSON logs |
| 3 | **Codex** | `CodexRecallPlugin` | `~/.codex/conversations/` | Conversation JSON |
| 4 | **Cursor** | `CursorRecallPlugin` | `~/.cursor/User/workspaceStorage/` | Workspace state JSON |
| 5 | **Goose** | `GooseRecallPlugin` | `~/.config/goose/sessions/` | Session files |
| 6 | **OpenCode** | `OpenCodeRecallPlugin` | `~/.opencode/sessions/` | Markdown / JSON session logs |
| 7 | **ZCode** | `ZCodeRecallPlugin` | `~/.zcode/conversations/` | Conversation records |
| 8 | **AGY CLI** | `AgyCliRecallPlugin` | `~/.gemini/transcripts/` | AGY CLI execution transcripts |
| 9 | **Kimi** | `KimiRecallPlugin` | `~/.kimi/chats/` | Kimi chat & session logs |

---

## Architecture & Parser Contract

### 1. `RecallPlugin` Trait (`crates/buzz-plugins/src/recall.rs`)

```rust
use async_trait::async_trait;
use std::path::Path;
use crate::models::RawDocument;

#[async_trait]
pub trait RecallPlugin: Send + Sync {
    /// Name of the agent harness (e.g. "antigravity", "claude_code", "opencode")
    fn name(&self) -> &'static str;

    /// Detect if the agent's data directory exists on the workstation
    fn detect(&self) -> bool;

    /// Discover and parse sessions for the specified workspace path
    async fn ingest_sessions(&self, workspace_path: &Path) -> Vec<RawDocument>;
}
```

### 2. Session Normalization & Redaction

For each detected session:
1. Extract user queries, agent explanations, tool invocations, and code diffs.
2. Run through `SecretRedactor` to strip credentials.
3. Compute `content_hash` to guarantee idempotency (prevent re-ingesting identical sessions).
4. Chunk and embed using `orbit-ingest` and `orbit-ai`.
5. Store in `buzz_documents` (`source_type = 'session'`) and `buzz_chunks` (`agent_name = plugin.name()`).

---

## Verification & Quality Gates

- Unit tests for all 9 parser plugins with sample synthetic session files.
- Verify deduplication: re-running ingestion does not create duplicate chunks.
- Run `cargo test -p buzz-recall`.
- Run `just ci`.
