# Command Adviser Default Harness Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make existing Command Team agents inherit the saved Codex default, apply harness edits to active agents immediately, and remove the inherited 24-worker setting.

**Architecture:** Extend the pure managed-agent runtime resolver with the global preferred runtime and use it at spawn/status/summary boundaries. Restart only active local agents after a successful harness edit. Apply a narrow idempotent migration from the known legacy Command Team parallelism value of 24 to 1.

**Tech Stack:** Rust, Tauri 2, React 19, TypeScript, Node test runner.

## Global Constraints

- Preserve explicit instance and persona runtime choices.
- Do not require a provider for the Codex harness.
- Do not start stopped agents automatically.
- Preserve unrelated dirty-worktree changes.

---

### Task 1: Runtime inheritance

**Files:**
- Modify: `desktop/src-tauri/src/managed_agents/discovery.rs`
- Modify: managed-agent resolver call sites
- Test: existing Rust tests beside the resolver

**Interfaces:**
- Consumes: `GlobalAgentConfig.preferred_runtime`
- Produces: a pure resolver implementing override → record → persona → global → Buzz

- [ ] Add a failing test where a runtime-null record and runtime-null persona resolve to `codex-acp` when the preferred runtime is `codex`.
- [ ] Run the focused test and confirm it fails by returning `buzz-agent`.
- [ ] Add the preferred-runtime argument and update application call sites.
- [ ] Run the focused tests and confirm they pass.

### Task 2: Active harness-edit restart

**Files:**
- Modify: `desktop/src/features/agents/ui/AgentInstanceEditDialog.tsx`
- Test: the nearest existing Edit Agent test module

**Interfaces:**
- Consumes: successful `updateManagedAgent` result and active-agent state
- Produces: one restart request only when an active agent's harness changed

- [ ] Add failing tests for active restart and stopped-agent preservation.
- [ ] Run them and confirm the active restart assertion fails.
- [ ] Implement the minimal post-save restart.
- [ ] Run the focused tests and confirm both pass.

### Task 3: Command Team worker migration

**Files:**
- Modify: the existing Command Adviser managed-agent migration module
- Test: its adjacent Rust test module

**Interfaces:**
- Consumes: managed Command Team identity and `parallelism == 24`
- Produces: idempotent `parallelism = 1`

- [ ] Add a failing migration test proving Command Team 24 becomes 1 while other values remain unchanged.
- [ ] Run it and confirm the expected failure.
- [ ] Implement the narrow migration.
- [ ] Run the migration tests and confirm they pass.

### Task 4: Verification and live handoff

**Files:**
- No new production files

- [ ] Activate Hermit and run focused Rust and desktop tests.
- [ ] Run formatting and the relevant lint/type checks.
- [ ] Restart the packaged Command Adviser only if a newly built application is available.
- [ ] Verify managed records and runtime logs show Codex inheritance with one worker.
- [ ] Record the correction in Memory MCP with agent `CODEX`.

