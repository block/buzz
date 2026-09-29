# Feature 08 — Desktop Agent Integration & 1-Click Harness Hub

> **Priority**: P1 — Seamless desktop-first agent configuration (Zero CLI required).  
> **Sprint**: Sprint 4 (Week 7)  
> **Dependencies**: Feature 07 (MCP Server Tools)  
> **Components**: `desktop/src/features/harness/` (Desktop UI), `desktop/src-tauri/src/harness.rs` (Tauri IPC), `buzz-cli` (optional CLI fallback)  
> **Environment Variables**: `BUZZ_DATA_DIR`

---

## Overview

Feature 08 delivers a **Desktop-First Agent Integration & 1-Click Harness Hub** inside the Orbit/Buzz desktop application. Rather than requiring developers to run terminal commands, the desktop application automatically detects installed AI coding agent harnesses and allows developers to connect them to the centralized Orbit Brain with a single click.

---

## Desktop User Flow: The 1-Click Harness Hub

```
Developer signs in or opens Desktop App
                    │
                    ▼
Desktop Scans Workstation for Installed Agent Harnesses
(Antigravity, Claude Code, Cursor, Codex, Goose, OpenCode, ZCode, AGY, Kimi)
                    │
                    ▼
Harness Hub Displayed in Settings / Onboarding
┌─────────────────────────────────────────────────────────────┐
│  🧠 Orbit Centralized Brain — Agent Harnesses               │
│                                                             │
│  [⚡] Antigravity IDE     [Detected]  [✓ Connected]          │
│  [⚡] Claude Code         [Detected]  [ Connect to Brain ]   │
│  [⚡] Cursor              [Detected]  [✓ Connected]          │
│  [⚡] OpenCode            [Detected]  [ Connect to Brain ]   │
│  [⚡] Kimi                [Detected]  [ Connect to Brain ]   │
│  [⚡] Goose               [Not Found] [ Install Guide ]      │
└─────────────────────────────────────────────────────────────┘
                    │ (User clicks "Connect to Brain")
                    ▼
Tauri IPC Writes Agent MCP Config & Injects Shared Skill
                    │
                    ▼
Agent Immediately Shares Centralized `orbit_brain/`
```

---

## Supported Harness Auto-Configurations

| Harness | Configuration File Modified | Skill File Injected |
|---------|-----------------------------|---------------------|
| **Antigravity** | `~/.gemini/config/mcp_config.json` | Built-in via AGY skill system |
| **Claude Code** | `~/.claude/mcp_config.json` | `~/.claude/skills/orbit-memory/SKILL.md` |
| **Cursor** | `.cursor/mcp.json` | `.cursor/rules/orbit-memory.md` |
| **Codex** | `~/.codex/config.json` | `~/.codex/instructions.md` |
| **Goose** | `~/.config/goose/config.yaml` | `~/.goose/skills/orbit.yaml` |
| **OpenCode** | `~/.opencode/mcp.json` | `~/.opencode/skills/orbit.md` |
| **ZCode** | `~/.zcode/mcp_config.json` | `~/.zcode/instructions/orbit.md` |
| **AGY CLI** | `~/.gemini/config/mcp_config.json` | AGY CLI shared skill |
| **Kimi** | `~/.kimi/mcp.json` | `~/.kimi/rules/orbit.md` |

---

## Tauri IPC Bridge: `desktop/src-tauri/src/harness.rs`

### Commands Exposed to Desktop UI

```rust
#[tauri::command]
pub async fn detect_agent_harnesses() -> Result<Vec<HarnessInfo>, String> {
    // Scans ~/.gemini, ~/.claude, ~/.cursor, etc.
    // Returns status: Detected, Connected, or Missing
}

#[tauri::command]
pub async fn connect_harness(harness_id: String) -> Result<bool, String> {
    // 1. Injects buzz-mcp (Orbit MCP server) into the harness's config file
    // 2. Copies the canonical Orbit Skill definition
    // 3. Connects harness to ~/.orbit/brain/
}

#[tauri::command]
pub async fn disconnect_harness(harness_id: String) -> Result<bool, String> {
    // Safely removes Orbit MCP registration
}
```

---

## Shared Skill Definition (`SKILL.md`)

When a harness connects, Orbit injects an identical universal skill instructing the agent how to leverage the centralized brain:
1. **At Session Start**: Call `orbit.get_project_context(".")` to load architectural principles.
2. **Before Modifying Modules**: Call `orbit.get_file_history("path")` to see previous decisions.
3. **Before Big Architectural Choices**: Call `orbit.search_context("query")` to verify constraints.
4. **After Completing Tasks**: Call `orbit.store_memory("decision...")` to update the centralized brain for other agents.

---

## Verification & Quality Gates

- Desktop UI test: open Settings → Agent Harnesses, verify detected harnesses match local system.
- 1-Click test: click "Connect" on Claude / Cursor / Antigravity, verify config file correctly updated.
- Zero-CLI verification: user connects harnesses without ever opening a terminal.
- Run `just ci`.

## Cloud account connection is separate from agent wiring

Agent harness configuration and ORBIT account authentication must remain separate concerns. Connecting Claude Code/Cursor/Codex/etc. grants the harness access to the local ORBIT MCP endpoint; it does not implicitly upload memory to the cloud. Cloud synchronization is an explicit user action in ORBIT settings.

The onboarding surface should expose:

1. `Use Local Brain` — no account required.
2. `Sign In` — opens the ORBIT website in the browser.
3. `Enable Cloud Sync` — shown only after successful account authentication and entitlement checks.
4. `Connected Devices` — shows registered devices and last sync state.

The MCP server always enforces the currently active local workspace and permission scope. A cloud-authenticated user is not automatically granted access to another local workspace.
