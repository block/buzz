# Actionable Brief Decisions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every supported Daily Command Brief decision two short courses of action, a user-direction field, and simple live execution tracking through the existing Chief of Staff conversation.

**Architecture:** Add one backwards-compatible proposal field, one pure decision projection, one small local execution store/hook, and one focused decision-card component. Dispatch reuses the existing managed-agent and Buzz DM paths; progress reuses live relay messages and active-turn state.

**Tech Stack:** Rust/Serde, Tauri 2, React 19, TypeScript, Buzz relay events, node:test, Playwright.

## Global Constraints

- Selecting a COA or issuing user direction is the approval and starts work immediately.
- Sources stay out of the main decision card.
- Use device dictation through the normal text field; add no speech service.
- Add no workflow engine, monitoring service, or receipt subsystem.
- Historical briefs remain readable.
- Use TDD for every behaviour change.

---

### Task 1: Two-course decision contract

**Files:**
- Modify: `desktop/src-tauri/src/command_brief/types/proposals.rs`
- Modify: `desktop/src-tauri/src/command_brief/personas.rs`
- Modify: `desktop/src-tauri/src/command_brief/types_tests.rs`
- Modify: `desktop/src-tauri/src/command_brief/personas_tests.rs`
- Modify: `desktop/src/features/command-console/domain/briefContracts.ts`
- Modify: `desktop/src/features/command-console/domain/briefContracts.test.mjs`

**Interfaces:**
- Produces: `PendingProposal.alternativeText` on current wire output.
- Preserves: historical proposals without `alternativeText`.

- [ ] Add failing Rust and TypeScript tests proving current proposals preserve a bounded `alternativeText` and historical proposals parse without it.
- [ ] Run the focused tests and confirm they fail because the field is not yet accepted.
- [ ] Add `alternative_text` to `PendingProposal`, accept it optionally on historical input, and require it from newly generated specialist output.
- [ ] Update specialist prompts to provide one concise credible alternative and omit the proposal entirely when no decision is needed.
- [ ] Run the focused Rust and TypeScript tests to green.

### Task 2: Decision projection and durable status store

**Files:**
- Create: `desktop/src/features/command-console/domain/briefDecisions.ts`
- Create: `desktop/src/features/command-console/domain/briefDecisions.test.mjs`
- Create: `desktop/src/features/command-console/domain/decisionExecutionStore.ts`
- Create: `desktop/src/features/command-console/domain/decisionExecutionStore.test.mjs`

**Interfaces:**
- Produces: `projectBriefDecisions(brief): BriefDecision[]`.
- Produces: versioned `DecisionExecution` records keyed by `runId:actionId`.

- [ ] Add failing tests for deduplicated decision projection, historical no-COA-B behaviour, terminal-state preservation, restart parsing, and five-minute stall detection.
- [ ] Run the tests and confirm failure because the modules do not exist.
- [ ] Implement the smallest immutable projection and local-storage store needed by the tests.
- [ ] Run the focused tests to green.

### Task 3: Immediate Chief of Staff dispatch

**Files:**
- Create: `desktop/src/features/command-console/hooks/useCommandDecisionActions.ts`
- Create: `desktop/src/features/command-console/hooks/useCommandDecisionActions.test.mjs`
- Reuse: `desktop/src/features/agents/openPersonaConversation.ts`
- Reuse: `desktop/src/shared/api/tauri.ts`
- Reuse: `desktop/src/shared/api/relayClient.ts`

**Interfaces:**
- Produces: `issueDecision(decision, direction, source)` and `retryDecision(key)`.
- Consumes: Chief of Staff persona `builtin:command-chief-of-staff`.

- [ ] Add failing tests proving direction dispatch starts/reuses the Chief, opens its DM, sends one stable tagged instruction, records failure, and maps live status messages.
- [ ] Run the hook tests and confirm the expected failures.
- [ ] Implement dispatch with existing persona/DM/message dependencies and subscribe to the direction channel for status updates.
- [ ] Connect existing active-turn state to `In progress` and the five-minute watchdog to `Stalled`.
- [ ] Run the focused tests to green.

### Task 4: Concise interactive decision card

**Files:**
- Create: `desktop/src/features/command-console/ui/BriefDecisionCard.tsx`
- Modify: `desktop/src/features/command-console/ui/DailyCommandBrief.tsx`
- Modify: `desktop/src/features/command-console/ui/DailyCommandBrief.test.mjs`
- Modify: `desktop/tests/e2e/daily-command-brief.spec.ts`

**Interfaces:**
- Consumes: projected decisions and `useCommandDecisionActions`.
- Renders: COA A, optional COA B, spell-checked user direction, explicit issue buttons, compact status, retry, and open-conversation actions.

- [ ] Add failing component tests for concise choices, hidden citations, text/dictation input, immediate dispatch, statuses, retry, and conversation navigation.
- [ ] Run the component tests and confirm the interaction is absent.
- [ ] Implement the decision card and replace the read-only decisions section.
- [ ] Add the minimum Playwright path for issuing a COA and observing status.
- [ ] Run component and Playwright tests to green.

### Task 5: Verification and delivery

**Files:**
- Modify only files needed to resolve verification failures introduced by this phase.

- [ ] Run command-console unit tests, Tauri command-brief tests, TypeScript typecheck, and Biome checks.
- [ ] Run `just ci` and confirm a zero exit code.
- [ ] Build and codesign the release app, preserve the installed app and data, then install the new build.
- [ ] Conduct one live direction canary through the Chief of Staff and verify queued/in-progress plus a terminal or stalled state.
- [ ] Commit with signoff, push, open the phase PR, and record the outcome in Memory MCP with agent `CODEX`.

