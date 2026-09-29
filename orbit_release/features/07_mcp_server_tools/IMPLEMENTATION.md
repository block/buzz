# Feature 07 — MCP Server Tools (orbit.* Tools for AI Agents)

> **Priority**: P0 — Integration point. Exposes Orbit intelligence to AI agents.  
> **Sprint**: Sprint 4 (Week 7)  
> **Dependencies**: Feature 01, 02, 04, 05, 06  
> **Crates**: `orbit-mcp` (implementation), `buzz-dev-mcp` (registration)  
> **Environment Variables**: `BUZZ_DATABASE_URL`, `BUZZ_DATA_DIR`

---

## Overview

Feature 07 exposes the 8 standardized `orbit.*` MCP tools to AI coding agents (Antigravity, Claude Code, Cursor, Codex, Goose, OpenCode, ZCode, AGY, Kimi). The tools are registered directly inside the existing `buzz-dev-mcp` server, allowing a single lightweight process to serve both developer tools (`shell`, `read_file`) and Orbit memory tools.

---

## The 8 Standardized Orbit MCP Tools

| Tool Name | Type | Signature | Description | Backend Feature |
|-----------|------|-----------|-------------|-----------------|
| `orbit.search_context` | Read | `(query: string, limit?: number, workspace?: string)` | Hybrid Multi-RAG search over code, docs, and memories | Feature 04 |
| `orbit.store_memory` | Write | `(content: string, tags?: string[], scope?: string)` | Explicitly writes a new architectural decision or fact | Feature 01, 02 |
| `orbit.get_project_context` | Read | `(path: string, max_tokens?: number)` | Generates full architectural summary for a workspace | Feature 03, 04 |
| `orbit.recall_session` | Read | `(agent?: string, query?: string, days?: number)` | Recalls past conversations and decisions from agents | Feature 06 |
| `orbit.get_file_history` | Read | `(file_path: string)` | Retrieves decisions and changes affecting a specific file | Feature 05 |
| `orbit.mark_decision` | Write | `(id: string, state: string, note?: string)` | Updates or invalidates a decision in the knowledge graph | Feature 05 |
| `orbit.get_index_stats` | Read | `()` | Returns chunk counts, index health, and sync status | Feature 01 |
| `orbit.delete_memory` | Write | `(id: string)` | Deletes a chunk/memory (GDPR compliance) | Feature 01 |

---

## Security & Context Arbiter

To protect AI agents from prompt injections and hallucinated data:
1. **Delimiter Fencing**: All outbound context is wrapped in `<orbit_untrusted_context source="...">`.
2. **Secret Redaction**: Any accidental secrets in retrieved text are stripped before returning to the agent.
3. **Audit Logging**: Every tool invocation is recorded in `buzz-audit`.

---

## Verification & Quality Gates

- Integration test: invoke `orbit.store_memory` over JSON-RPC stdio → verify chunk stored.
- Integration test: invoke `orbit.search_context` over JSON-RPC stdio → verify ranked results.
- Run `cargo test -p orbit-mcp`.
- Run `just ci`.

## Local versus hosted MCP policy

The MCP surface is local-first in both product editions. A local agent call should normally execute against the local ORBIT engine and local indexes, even when the user is signed in.

When cloud sync is enabled, the MCP server may expose sync state tools, but memory retrieval should not silently redirect to a remote database. Remote retrieval is an explicit capability controlled by workspace policy and user settings.

Recommended additional tools:

- `orbit.sync_status` — pending changes, last successful sync, device identifier.
- `orbit.sync_now` — request an immediate logical-state sync.
- `orbit.list_devices` — show registered devices for the signed-in account/workspace.
- `orbit.cloud_policy` — report whether the current workspace permits cloud storage or hosted processing.

No MCP tool should ever return access tokens, subscription secrets, raw credentials or another device's local-only data.
