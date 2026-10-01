# Feature 06 — Recall Agent Checklist

> **Directory**: `orbit_release/features/06_recall_agent/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 3 (Weeks 5–6)

---

## Deliverables & Tasks

### 1. `buzz-plugins` & `buzz-recall` Setup
- [ ] Create `crates/buzz-plugins` with `RecallPlugin` trait
- [ ] Create `crates/buzz-recall` implementing the 9 agent parsers
- [ ] Connect with `buzz-db` and `buzz-ai`

### 2. 9 IDE & Agent Parsers
- [ ] Implement `AntigravityRecallPlugin` (`~/.gemini/antigravity-ide/brain/*/transcript.jsonl`)
- [ ] Implement `ClaudeCodeRecallPlugin` (`~/.claude/projects/**/`)
- [ ] Implement `CodexRecallPlugin` (`~/.codex/conversations/`)
- [ ] Implement `CursorRecallPlugin` (`~/.cursor/User/workspaceStorage/`)
- [ ] Implement `GooseRecallPlugin` (`~/.config/goose/sessions/`)
- [ ] Implement `OpenCodeRecallPlugin` (`~/.opencode/sessions/`)
- [ ] Implement `ZCodeRecallPlugin` (`~/.zcode/conversations/`)
- [ ] Implement `AgyCliRecallPlugin` (`~/.gemini/transcripts/`)
- [ ] Implement `KimiRecallPlugin` (`~/.kimi/chats/`)

### 3. Normalization & Idempotency
- [ ] Extract user prompts, agent reasoning, tool calls, and diffs
- [ ] Pass raw text through Secret Redactor
- [ ] Compute SHA-256 `content_hash` for session deduplication
- [ ] Track watermark / last ingested timestamp per session

### 4. Background Sync & Orchestration
- [ ] Implement periodic background poll (default every 300s)
- [ ] Cache ingested transcripts in `orbit_brain/transcripts/`

---

## Verification & Sign-off

- [ ] `cargo test -p buzz-recall` passes
- [ ] All 9 parsers correctly extract structured data from sample fixtures
- [ ] Re-running ingestion is fully idempotent (0 new chunks created)
- [ ] `just ci` passes cleanly
