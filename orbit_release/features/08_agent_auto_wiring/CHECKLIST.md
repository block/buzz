# Feature 08 — Desktop Agent Integration Checklist

> **Directory**: `orbit_release/features/08_agent_auto_wiring/`  
> **Status**: `[x] Complete / Verified`  
> **Estimated Effort**: Sprint 4 (Week 7)

---

## Deliverables & Tasks

### 1. Tauri Backend Detection Engine (`desktop/src-tauri/src/harness.rs`)
- [x] Implement detection logic for:
  - [x] Antigravity IDE (`~/.gemini/config/`)
  - [x] Claude Code (`~/.claude/`)
  - [x] Cursor (`.cursor/` or `~/.cursor/`)
  - [x] Codex (`~/.codex/`)
  - [x] Goose (`~/.config/goose/`)
  - [x] OpenCode (`~/.opencode/`)
  - [x] ZCode (`~/.zcode/`)
  - [x] AGY CLI (`~/.gemini/`)
  - [x] Kimi (`~/.kimi/`)
- [x] Implement `detect_agent_harnesses` Tauri command
- [x] Implement `connect_harness` Tauri command (injects JSON-RPC config)
- [x] Implement `disconnect_harness` Tauri command

### 2. Desktop UI Component (`desktop/src/features/harness/`)
- [x] Create `AgentHarnessHub.tsx` component in Desktop settings and onboarding
- [x] Display grid/list of detected agents with status badges (`Connected`, `Detected`, `Not Installed`)
- [x] Add 1-click `Connect to Brain` button with animated success indicator
- [x] Display connection status to centralized `orbit_brain/`
- [x] Add quick link to launch or view agent instructions

### 3. Universal Orbit Skill Injection
- [x] Create template `orbit-memory.skill.md`
- [x] Ingest skill into connected agent config directories on 1-click connect
- [x] Ensure prompt templates instruct agents on `orbit.*` tool call patterns

### 4. Optional CLI Fallback (`buzz-cli`)
- [x] Add `buzz memory install` as headless fallback for CI/CD environments

---

## Verification & Sign-off

- [x] Desktop app opens and detects installed agents automatically
- [x] Clicking "Connect" updates the target agent's config file without manual terminal usage
- [x] Connected agent immediately interacts with `orbit_brain/` via MCP
- [x] `just ci` passes cleanly
