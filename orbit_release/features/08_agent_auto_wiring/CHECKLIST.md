# Feature 08 — Desktop Agent Integration Checklist

> **Directory**: `orbit_release/features/08_agent_auto_wiring/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 4 (Week 7)

---

## Deliverables & Tasks

### 1. Tauri Backend Detection Engine (`desktop/src-tauri/src/harness.rs`)
- [ ] Implement detection logic for:
  - [ ] Antigravity IDE (`~/.gemini/config/`)
  - [ ] Claude Code (`~/.claude/`)
  - [ ] Cursor (`.cursor/` or `~/.cursor/`)
  - [ ] Codex (`~/.codex/`)
  - [ ] Goose (`~/.config/goose/`)
  - [ ] OpenCode (`~/.opencode/`)
  - [ ] ZCode (`~/.zcode/`)
  - [ ] AGY CLI (`~/.gemini/`)
  - [ ] Kimi (`~/.kimi/`)
- [ ] Implement `detect_agent_harnesses` Tauri command
- [ ] Implement `connect_harness` Tauri command (injects JSON-RPC config)
- [ ] Implement `disconnect_harness` Tauri command

### 2. Desktop UI Component (`desktop/src/features/harness/`)
- [ ] Create `AgentHarnessHub.tsx` component in Desktop settings and onboarding
- [ ] Display grid/list of detected agents with status badges (`Connected`, `Detected`, `Not Installed`)
- [ ] Add 1-click `Connect to Brain` button with animated success indicator
- [ ] Display connection status to centralized `orbit_brain/`
- [ ] Add quick link to launch or view agent instructions

### 3. Universal Orbit Skill Injection
- [ ] Create template `orbit-memory.skill.md`
- [ ] Ingest skill into connected agent config directories on 1-click connect
- [ ] Ensure prompt templates instruct agents on `orbit.*` tool call patterns

### 4. Optional CLI Fallback (`buzz-cli`)
- [ ] Add `buzz memory install` as headless fallback for CI/CD environments

---

## Verification & Sign-off

- [ ] Desktop app opens and detects installed agents automatically
- [ ] Clicking "Connect" updates the target agent's config file without manual terminal usage
- [ ] Connected agent immediately interacts with `orbit_brain/` via MCP
- [ ] `just ci` passes cleanly
