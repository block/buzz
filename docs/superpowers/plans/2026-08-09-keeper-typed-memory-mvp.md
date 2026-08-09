# Keeper Typed-Memory MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a usable owner-private Keeper agent that can record typed relationship debriefs into existing encrypted Buzz engrams and recall them in later direct-message briefs.

**Architecture:** Keeper is a built-in managed persona, not a new service. The active Codex/ACP harness follows a strict schema and uses the already-shipped `buzz mem ls|get|set|rm` commands; signed DMs remain the raw source record and NIP-AE engrams hold only compact outcomes. The Living Ship projection is widened just enough to show Keeper from managed-agent metadata without adding Keeper to Daily Command Brief specialist contracts.

**Tech Stack:** Rust/Tauri managed personas, existing Buzz CLI NIP-AE engrams, React 19/TypeScript Living Ship projection, Node test runner, Playwright.

## Global Constraints

- No new database, memory service, scheduler, model runtime, HTTP endpoint, or cloud copy.
- `builtin:keeper` remains owner-private: `shared: false`, no non-owner respond-to policy, and no Command Adviser RAG/World Monitor environment injection.
- Raw debriefs remain in signed Buzz DMs; only compact outcomes and source event/thread IDs enter `mem/keeper/*`.
- Facts must be explicit owner statements; model inferences are labelled observations with confidence.
- Ambiguous identities are quarantined and never merged automatically.
- A receipt may say “saved” only after every named `buzz mem` write succeeds and is read back.
- The first version is typed text only. Voice, Calendar automation, notifications, contact ingestion, and remote access remain deferred.

---

### Task 1: Built-in Keeper and encrypted-memory operating contract

**Files:**
- Modify: `desktop/src-tauri/src/managed_agents/personas/tests.rs`
- Modify: `desktop/src-tauri/src/managed_agents/personas.rs`
- Modify: `desktop/src/testing/e2eBridge.ts`
- Test: `desktop/src-tauri/src/managed_agents/personas/tests.rs`

**Interfaces:**
- Consumes: existing `AgentDefinition`, on-demand persona conversation provisioning, and `buzz mem ls|get|set|rm`.
- Produces: active `builtin:keeper`, display name `Keeper`, symbolic avatar, one-turn parallelism, and a prompt defining `keeper-index-v1`, `keeper-person-v1`, `keeper-interaction-v1`, and `keeper-unresolved-v1` records.

- [ ] **Step 1: Write the failing built-in behavior test**

  Add a test that finds `builtin:keeper` and asserts it is active, built-in, runtime/model independent, owner-private, single-turn, symbolic, and names the exact encrypted slugs and CLI read/write/tombstone commands. Assert the prompt requires source event/thread IDs, fact/observation separation, ambiguity quarantine, read-back verification, correction, forget, and immediate undo.

- [ ] **Step 2: Run the focused Rust test and verify RED**

  Run: `. ./bin/activate-hermit && cargo test --manifest-path desktop/src-tauri/Cargo.toml managed_agents::personas::tests::keeper_builtin_is_owner_private_and_uses_encrypted_engram_protocol -- --exact`

  Expected: FAIL because no `builtin:keeper` definition exists.

- [ ] **Step 3: Add the minimal Keeper definition**

  Add a compact naval-style SVG data URI and an explicit prompt. The prompt must use opaque lowercase IDs in slugs, resolve identity through `mem/keeper/index`, quarantine multiple matches, retain the triggering Buzz event/thread references, use stdin for JSON writes, verify successful writes with `buzz mem get`, and report partial failure truthfully.

- [ ] **Step 4: Run the focused test and verify GREEN**

  Run the Step 2 command again. Expected: PASS.

- [ ] **Step 5: Add the E2E bridge fixture and run persona regressions**

  Mirror the built-in in `resetMockPersonas`, then run:

  `cd desktop && pnpm test src/features/agents`

  Expected: PASS with Keeper available to ordinary persona-card and Message flows.

### Task 2: Managed Keeper projection in Living Ship

**Files:**
- Modify: `desktop/src/features/living-ship/domain/shipLayout.test.mjs`
- Modify: `desktop/src/features/living-ship/domain/shipProjection.test.mjs`
- Modify: `desktop/src/features/living-ship/domain/shipLayout.ts`
- Modify: `desktop/src/features/living-ship/domain/shipProjection.ts`
- Modify: `desktop/src/features/living-ship/ui/LivingShipScreen.tsx`
- Modify: `desktop/tests/e2e/living-ship.spec.ts`

**Interfaces:**
- Consumes: managed agents identified by `personaId`.
- Produces: `LivingShipAgentId = AdviserId | "keeper"`; Keeper appears only after its managed instance exists, uses Ship's Office as its working home, and remains outside Command Brief contribution contracts.

- [ ] **Step 1: Write failing projection tests**

  Extend the room-map test with `keeper: "ships-office"`. Add a managed Keeper fixture and assert `projectLivingShipAgents` includes it while still excluding unrelated personas.

- [ ] **Step 2: Run focused TypeScript tests and verify RED**

  Run: `cd desktop && pnpm test src/features/living-ship/domain/shipLayout.test.mjs src/features/living-ship/domain/shipProjection.test.mjs`

  Expected: FAIL because the visual registry has no Keeper.

- [ ] **Step 3: Widen the Living Ship-only identity and add Keeper metadata**

  Introduce `LivingShipAgentId` locally, add Keeper with persona ID `builtin:keeper`, Ship's Office home, short label `KEEP`, and a reused compatible sprite column. Update screen copy from only “command team” to “command team and support agents”. Do not change `AdviserId`, `SPECIALISTS`, or Daily Command Brief schemas.

- [ ] **Step 4: Run focused tests and verify GREEN**

  Run the Step 2 command again. Expected: PASS.

- [ ] **Step 5: Extend the real screen journey**

  Add a running Keeper managed-agent seed to `living-ship.spec.ts`, assert its visible state and details, then run:

  `cd desktop && pnpm exec playwright test tests/e2e/living-ship.spec.ts --project=smoke`

  Expected: PASS with Keeper visible from the managed roster.

### Task 3: Phase closure and user-test build

**Files:**
- Modify: `docs/command-console/ROADMAP.md`
- Modify: `docs/superpowers/plans/2026-08-09-keeper-typed-memory-mvp.md`

**Interfaces:**
- Consumes: Tasks 1-2.
- Produces: a verified phase PR and a reproducible installed-app acceptance checklist.

- [ ] **Step 1: Run focused gates**

  Run the Rust persona test, Living Ship domain tests, Living Ship Playwright journey, `git diff --check`, and relevant format/lint checks.

- [ ] **Step 2: Run the repository gate**

  Run: `. ./bin/activate-hermit && just ci`

  Expected: all configured Rust, desktop, Tauri, web, mobile, sidecar, build, and unit gates pass.

- [ ] **Step 3: Update the rollout record**

  Mark the compatibility freeze complete and Keeper MVP implemented/pending installed-app acceptance. Record the exact test commands and final PR/commit evidence once known.

- [ ] **Step 4: Build and launch the macOS app for user acceptance**

  Validate the installed journey: open Agents, select Keeper Message, allow the on-demand managed instance to start with the existing default Codex harness, debrief one unambiguous fictional interaction, request a brief in a later turn, correct one fact, forget it, and immediately undo. Confirm Keeper appears in Living Ship after provisioning.

  Stop only if macOS requires the user to approve Keychain/privacy access or if the live model/relay path needs a user credential action.
