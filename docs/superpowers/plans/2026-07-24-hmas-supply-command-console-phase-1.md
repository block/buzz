# HMAS Supply Command Console Phase 1 Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task with fresh implementer and reviewer agents.

**Goal:** Establish a tested macOS Command Console foundation on the Buzz fork with default-`OFFICIAL` contracts, truthful local-service status, a recoverable local workspace, and an offline launch path.

**Architecture:** Keep Buzz's Rust/Tauri/React architecture and signed Nostr workspace intact. Add the Command Console as a lazy-loaded desktop feature, place shared advisory contracts in a dependency-light TypeScript domain module, and use existing relay/Tauri health interfaces rather than adding a parallel backend. Phase 1 prepares—but does not yet implement—the LM Studio runtime, RAG replication, adviser orchestration, Apple data access, or workspace mutations.

**Tech Stack:** Rust, Tauri 2, React, TypeScript, TanStack Router, Vitest/Node tests, Playwright, Docker Compose, Bash, Just, Hermit.

---

## Scope and acceptance boundary

Phase 1 includes:

- A working downstream fork, isolated `codex/phase-1-foundation` worktree, and draft PR.
- Reliable local service bootstrap, including a correct Keycloak 26 readiness check.
- Versioned TypeScript command contracts with `OFFICIAL` as the default classification.
- A discoverable macOS Command Console route and shell.
- A truthful status view using existing relay/local-compute interfaces, with explicit unavailable and offline states.
- Scripted, manifest-based PostgreSQL and MinIO backup/restore foundations.
- Tests, offline-launch evidence, and operator documentation.

Phase 1 excludes:

- Adviser execution, model routing, RAG retrieval, Memory MCP replication, Apple Calendar/Reminders/Notes access, scheduled briefs, and approval-gated workspace mutations.
- Any ship-control, navigation-control, communications, combat, logistics, or personnel-system integration.

### Task 1: Make local service bootstrap deterministic

**Files:**

- Modify: `docker-compose.yml`
- Modify: `Justfile`
- Modify: `scripts/dev-setup.sh`
- Add: `scripts/check-local-services.sh`
- Add: `scripts/tests/check-local-services-test.sh`

**Steps:**

1. Add a failing test that validates required/optional service-state handling and confirms the Keycloak health configuration uses its enabled management endpoint.
2. Run the new test and capture the expected failure.
3. Enable Keycloak health and replace the invalid shell/port probe with the documented Bash TCP readiness probe on port 9000.
4. Add bounded, diagnostic service-state checks; required services must fail setup, while optional development services remain visible without blocking relay/desktop startup unless explicitly required.
5. Remove masked Compose startup failures or immediately convert them into explicit diagnostics.
6. Run the script test, `docker compose config -q`, Keycloak health probe, and `just setup`.
7. Commit only Task 1 changes.

### Task 2: Add command-domain contracts and classification invariants

**Files:**

- Add: `desktop/src/features/command-console/domain/contracts.ts`
- Add: `desktop/src/features/command-console/domain/contracts.test.mjs`
- Add: `desktop/src/features/command-console/domain/classification.ts`
- Add: `desktop/src/features/command-console/domain/classification.test.mjs`
- Modify: `desktop/package.json` only if a test registration is required

**Steps:**

1. Write failing tests for `Classification`, `SourceReference`, `AdviserContribution`, `CommandBrief`, `ProposedWorkspaceAction`, `ModelRoute`, `KnowledgeSnapshotManifest`, `MemoryRevision`, and `ReplicationEnvelope`.
2. Require creation helpers to default to `OFFICIAL`; no helper may silently downgrade an artefact.
3. Implement immutable, serialisable TypeScript contracts and narrow validation helpers for untrusted persisted data.
4. Keep runtime dependencies minimal and avoid wiring later-phase services.
5. Run focused tests and the desktop type check.
6. Commit only Task 2 changes.

### Task 3: Add the macOS Command Console route and navigation

**Files:**

- Add: `desktop/src/app/routes/console.tsx`
- Add: `desktop/src/features/command-console/ui/CommandConsoleScreen.tsx`
- Add: `desktop/src/features/command-console/ui/CommandConsoleScreen.test.mjs`
- Modify: `desktop/src/app/AppShell.helpers.ts`
- Modify: `desktop/src/app/AppShell.helpers.test.mjs`
- Modify: `desktop/src/app/navigation/useAppNavigation.ts`
- Modify: `desktop/src/app/AppShell.tsx`
- Modify: `desktop/src/features/sidebar/ui/AppSidebarPinnedHeader.tsx`
- Modify other sidebar prop files only as required by the existing prop chain
- Regenerate: `desktop/src/routeTree.gen.ts`

**Steps:**

1. Write failing route-derivation, navigation, and screen tests.
2. Add the lazy-loaded `/console` route and extend the existing shell route union.
3. Add a pinned-sidebar Command Console entry with `data-testid="open-command-console-view"`.
4. Render an unmistakable `OFFICIAL` banner and six adviser placeholders without implying that advisers are operational.
5. Regenerate the TanStack route tree through the repository-supported command.
6. Run focused tests, desktop checks, and formatting.
7. Commit only Task 3 changes.

### Task 4: Show truthful local readiness and offline state

**Files:**

- Add: `desktop/src/features/command-console/hooks/useCommandConsoleStatus.ts`
- Add: `desktop/src/features/command-console/hooks/useCommandConsoleStatus.test.mjs`
- Add: `desktop/src/features/command-console/ui/CommandSystemStatus.tsx`
- Add: `desktop/src/features/command-console/ui/CommandSystemStatus.test.mjs`
- Modify: `desktop/src/features/command-console/ui/CommandConsoleScreen.tsx`
- Reuse existing relay and mesh APIs; modify shared APIs only if a missing typed read-only operation is proven

**Steps:**

1. Write failing tests for connected, degraded, unavailable, and offline states.
2. Compose existing relay-connection and local-compute status sources into a read-only view model.
3. Make unknown/unavailable states explicit; never label a service healthy without a successful probe.
4. Show later-phase capabilities as `Not configured` rather than simulated data.
5. Run focused tests, desktop checks, and formatting.
6. Commit only Task 4 changes.

### Task 5: Add manifest-based local workspace backup and restore

**Files:**

- Add: `scripts/backup-local-workspace.sh`
- Add: `scripts/restore-local-workspace.sh`
- Add: `scripts/lib/local-workspace-backup.sh`
- Add: `scripts/tests/local-workspace-backup-test.sh`
- Modify: `Justfile`
- Add: `docs/development/local-workspace-backup.md`

**Steps:**

1. Write failing tests against mocked Docker responses for manifest validation, restrictive permissions, confirmation requirements, and failure propagation.
2. Implement timestamped PostgreSQL custom-format dumps and MinIO object mirroring with a checksummed, versioned manifest.
3. Write backups only to an explicit path outside the repository; refuse repository-contained targets.
4. Make restore validate the manifest and archive before requiring explicit confirmation, stop write-producing services, restore data, run migrations, and verify readiness.
5. Add non-destructive Just recipes and operator documentation.
6. Run unit-style script tests; if the harness supports it safely, run an isolated round-trip test.
7. Commit only Task 5 changes.

### Task 6: Integrate, exercise offline launch, and document Phase 1

**Files:**

- Add: `desktop/tests/e2e/command-console.spec.ts`
- Modify: `desktop/playwright.config.ts` or existing E2E registration only as required
- Add: `docs/command-console/phase-1-foundation.md`
- Modify: `README.md` only for a concise discovery link

**Steps:**

1. Add an E2E test that opens the Command Console and verifies the `OFFICIAL` classification, adviser placeholders, and truthful offline/unavailable status.
2. Exercise `just desktop-standalone` with network/service dependencies unavailable and confirm the app reaches its supported offline/community-selection state without attempting to start Docker.
3. Document prerequisites, macOS packaging status, Docker Desktop setup, security boundaries, backup/restore, and deferred phases.
4. Capture a Command Console screenshot through the repository's official screenshot workflow if the E2E harness is available.
5. Run focused tests, full desktop checks, upstream unit tests, and `just ci`.
6. Request code review, address all important findings, and rerun affected tests.
7. Record verified Phase 1 decisions and gotchas in Memory MCP with `agent="CODEX"`.
8. Commit remaining documentation/evidence, push the branch, and update the draft PR.

## Final verification

Run from the activated Hermit environment:

```bash
. ./bin/activate-hermit
docker compose config -q
just test-unit
just desktop-check
just desktop-test
just ci
```

Also verify:

- `docker inspect --format '{{.State.Health.Status}}' buzz-keycloak` returns `healthy`.
- The Command Console is reachable from the desktop sidebar.
- All newly created command artefacts default to `OFFICIAL`.
- Offline desktop launch neither requires nor starts Docker.
- Backup output is outside the repository, permission-restricted, and validated before restore.

