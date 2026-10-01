# Feature 06 — Recall Agent Checklist

> **Directory**: `orbit_release/features/06_recall_agent/`  
> **Status**: `[x] Complete`  
> **Estimated Effort**: Sprint 3 (Weeks 5–6)

---

## Deliverables & Tasks

### 1. `buzz-plugins` & `buzz-recall` Setup
- [x] Create `crates/buzz-plugins` with `RecallPlugin` trait
- [x] Create `crates/buzz-recall` implementing the 9 agent parsers
- [x] Connect with `buzz-db` and `buzz-ai`

### 2. 9 IDE & Agent Parsers
- [x] Implement `AntigravityRecallPlugin` (`~/.gemini/antigravity-ide/brain/*/transcript.jsonl`)
- [x] Implement `ClaudeCodeRecallPlugin` (`~/.claude/projects/**/`)
- [x] Implement `CodexRecallPlugin` (`~/.codex/conversations/`)
- [x] Implement `CursorRecallPlugin` (`~/.cursor/User/workspaceStorage/`)
- [x] Implement `GooseRecallPlugin` (`~/.config/goose/sessions/`)
- [x] Implement `OpenCodeRecallPlugin` (`~/.opencode/sessions/`)
- [x] Implement `ZCodeRecallPlugin` (`~/.zcode/conversations/`)
- [x] Implement `AgyCliRecallPlugin` (`~/.gemini/transcripts/`)
- [x] Implement `KimiRecallPlugin` (`~/.kimi/chats/`)

### 3. Normalization & Idempotency
- [x] Extract user prompts, agent reasoning, tool calls, and diffs
- [x] Pass raw text through Secret Redactor
- [x] Compute SHA-256 `content_hash` for session deduplication
- [x] Track watermark / last ingested timestamp per session

### 4. Background Sync & Orchestration
- [x] Implement periodic background poll (default every 300s)
- [x] Cache ingested transcripts in `orbit_brain/transcripts/`

---

## Verification & Sign-off

- [x] `cargo test -p buzz-recall` passes
- [x] All 9 parsers correctly extract structured data from sample fixtures
- [x] Re-running ingestion is fully idempotent (0 new chunks created)
- [ ] `just ci` passes cleanly
