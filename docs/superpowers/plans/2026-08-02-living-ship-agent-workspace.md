# Living Ship Agent Workspace Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a production desktop Ship screen that shows the eight Command Adviser agents in a pixel-art HMAS Supply side elevation, moves them between their home rooms, the Wardroom, collaboration workspaces, and the off-ship personnel strip using truthful live state, and opens existing activity details from the visualization.

**Architecture:** Add a self-contained `living-ship` desktop feature. Pure domain functions map command personas and explicit observer collaboration context to ship rooms; a React adapter combines managed-agent lifecycle data, the existing unified working signal, channels, and observer snapshots. The screen renders one generated raster ship scene plus semantic HTML room/agent overlays on a fixed 1920x981 logical coordinate layer that scales responsively with the native 1754x896 full-outline asset. No new server, polling loop, model call, game engine, or HTTP endpoint is introduced.

**Tech Stack:** React 19, TypeScript, TanStack Router, TanStack Query, Tailwind CSS, existing Tauri managed-agent and encrypted observer APIs, Node test runner, Playwright E2E, generated PNG/WebP assets.

**Global Constraints:** Preserve the full ship silhouette; use named rem text tokens only; use existing Lucide icons for UI controls; support reduced motion and hidden-tab animation pause; do not infer collaboration from channel co-presence; reset any new module-level community state; keep the PR draft until installed-app user acceptance.

---

## Task 1: Define ship rooms and truthful placement rules

**Files:**
- Create: `desktop/src/features/living-ship/domain/shipLayout.ts`
- Create: `desktop/src/features/living-ship/domain/shipLayout.test.mjs`
- Modify: `desktop/package.json` only if the existing test glob does not discover the new test

- [x] Write a failing domain test for all eight persona-to-home-room assignments, the aft/forward room geometry, online-idle Wardroom placement, stopped/offline personnel-strip placement, and working home-room placement.
- [x] Run `cd desktop && pnpm test -- src/features/living-ship/domain/shipLayout.test.mjs` and confirm the failure is caused by the missing module.
- [x] Implement typed `ShipRoomId`, room definitions, adviser visual definitions, lifecycle categories, and `resolveAgentLocation` with the minimum logic needed for the test.
- [x] Re-run the focused test and confirm it passes.
- [x] Add failing cases for explicit collaboration workspace precedence and deterministic context fallback (`operations/intelligence`, `navigation`, `command/planning`, `reporting/routine`, `logistics`).
- [x] Implement the collaboration resolver without using same-channel presence as evidence.
- [x] Re-run the focused test, then commit with `feat(desktop): define living ship placement rules`.

## Task 2: Project live desktop state into ship occupants

**Files:**
- Create: `desktop/src/features/living-ship/domain/shipProjection.ts`
- Create: `desktop/src/features/living-ship/domain/shipProjection.test.mjs`
- Create: `desktop/src/features/living-ship/hooks/useLivingShipAgents.ts`
- Modify: `desktop/src/features/agents/ui/agentSessionTypes.ts`
- Modify: `desktop/src/features/agents/observerRelayStore.ts` only if a stable latest-event selector is needed
- Modify: `desktop/src/features/communities/useCommunityInit.ts` only if Task 2 introduces module-level state

- [x] Write a failing projection test that joins managed agents to the eight command personas, ignores non-command agents, and emits stable presentation records with lifecycle state, channel, task summary, room, reason, and collaborators.
- [x] Confirm the focused test fails for the missing projection module.
- [x] Add optional collaboration metadata types read from observer frame payloads: `collaborationId`, `workspace`, `leadPubkey`, and `participantPubkeys`.
- [x] Implement pure projection helpers that accept managed agents, working states, channels, and recent observer events; group collaborators only by an explicit shared collaboration ID.
- [x] Add the React hook adapter using `useManagedAgentsQuery`, `useChannelsQuery`, `useAgentWorkingState`, and `useAgentObserverSnapshot`; avoid timers or polling beyond existing stores.
- [x] Add/reset module-level state only if required and test reset behavior.
- [x] Re-run focused tests and commit with `feat(desktop): project agent state into living ship`.

## Task 3: Produce the cohesive pixel-art ship assets

**Files:**
- Create: `desktop/src/features/living-ship/assets/hmas-supply-living-ship.png`
- Create: `desktop/src/features/living-ship/assets/agent-sprites.png`
- Create as needed: `desktop/src/features/living-ship/assets/*.webp`
- Reference: `/Users/matthewwarren/Library/Mobile Documents/com~apple~CloudDocs/Documents/Navy/HMAS Stalwart/Incident plotting boards.pdf`
- Reference: `/var/folders/nc/7ggx382n68z9pfms6sg20r7h0000gn/T/TemporaryItems/NSIRD_screencaptureui_BI5g8l/Screenshot 2026-08-02 at 11.35.50 am.png`

- [x] Use the image-generation skill to create a wide pixel-art HMAS Supply side elevation retaining the recognizable hull, flight deck, masts, replenishment rigs, superstructures, and bow/stern silhouette, with the approved aft 2-stack and forward 2x3 compartment openings.
- [x] Generate a matching eight-character sprite sheet with distinct naval-workspace silhouettes and no embedded labels.
- [x] Inspect both assets at original resolution; reject distorted hull proportions, illegible compartment openings, unwanted text, or inconsistent pixel scale.
- [x] Copy selected project-bound outputs into the feature assets directory and optimize losslessly or to visually equivalent WebP where appropriate.
- [x] Record final prompts and source paths in `desktop/src/features/living-ship/assets/README.md`.
- [x] Commit with `feat(desktop): add living ship pixel art assets`.

## Task 4: Build the interactive Ship screen

**Files:**
- Create: `desktop/src/features/living-ship/ui/LivingShipScreen.tsx`
- Create: `desktop/src/features/living-ship/ui/LivingShipCanvas.tsx`
- Create: `desktop/src/features/living-ship/ui/LivingShipAgent.tsx`
- Create: `desktop/src/features/living-ship/ui/LivingShipDetails.tsx`
- Create: `desktop/src/features/living-ship/ui/LivingShipScreen.test.mjs`
- Create: `desktop/src/features/living-ship/livingShip.css`

- [x] Write a failing UI contract test for the ship title, eight named advisers, all room labels, personnel strip, room/agent selection, collaboration labels, and activity button.
- [x] Confirm the test fails because the screen does not exist.
- [x] Implement the responsive 1920x981 logical coordinate layer, raster layers, semantic room hit targets, agent markers, selection details, state legend, and off-ship strip.
- [x] Use CSS positional transitions for movement with `prefers-reduced-motion`; rely on browser-managed hidden-tab suspension and do not add a render loop.
- [x] Wire `useOpenAgentActivity` so the details action opens existing scoped agent activity and preserves access checks.
- [x] Ensure loading, no-command-team, offline, working, collaborating, and selected states are legible.
- [x] Re-run the UI contract and focused desktop tests, then commit with `feat(desktop): build interactive living ship screen`.

## Task 5: Wire navigation and community lifecycle

**Files:**
- Create: `desktop/src/app/routes/ship.tsx`
- Modify: `desktop/src/app/routes.ts`
- Modify: `desktop/src/app/routeTree.gen.ts` via the repository route generator
- Modify: `desktop/src/app/AppShell.helpers.ts`
- Modify: `desktop/src/app/AppShell.helpers.test.mjs`
- Modify: `desktop/src/app/navigation/useAppNavigation.ts`
- Modify: `desktop/src/app/navigation/useAppNavigation.test.mjs`
- Modify: `desktop/src/app/AppShell.tsx`
- Modify: `desktop/src/features/sidebar/ui/AppSidebar.tsx`
- Modify: `desktop/src/features/sidebar/ui/AppSidebarPinnedHeader.tsx`

- [x] Add failing navigation tests for `/ship`, selected view `ship`, and `goShip()`.
- [x] Confirm failures are due to the missing route/navigation behavior.
- [x] Add the lazy Ship route, sidebar item, selected-view plumbing, and route tree generation.
- [x] Verify community switching cannot retain a selected agent or collaboration from the prior relay; prefer component-local state reset by route remount and add a reset hook only if a singleton is introduced.
- [x] Run focused navigation tests and TypeScript/lint checks, then commit with `feat(desktop): add living ship navigation`.

## Task 6: Add seeded E2E coverage and visual QA

**Files:**
- Modify: `desktop/src/testing/e2eBridge.ts`
- Create: `desktop/tests/e2e/living-ship.spec.ts`
- Modify: `desktop/playwright.config.ts`
- Create: `desktop/test-results/living-ship/` at test runtime only

- [x] Add a failing Playwright spec that seeds the eight command personas and observer states, opens Ship from the sidebar, selects a working agent, selects a collaboration room, opens agent activity, and verifies the personnel strip.
- [x] Confirm the spec initially fails on absent mock observer helpers or selectors.
- [x] Reuse the existing narrow mock bridge lifecycle/observer fixtures and implement the required selectors.
- [x] Build E2E assets, kill any stale port 4173 server, and run the focused spec.
- [x] Capture distinct agent, collaboration, and offline/personnel-strip screenshots after `waitForAnimations(page)`; confirm SHA-256 hashes differ.
- [x] Compare the rendered ship with the approved side-elevation target at the same viewport and fix material silhouette, scale, overlap, contrast, focus, or reduced-motion problems.
- [x] Commit with `test(desktop): cover living ship user journey`.

## Task 7: Verify, document, publish draft, and leave UI acceptance open

**Files:**
- Modify: `docs/superpowers/plans/2026-08-02-living-ship-agent-workspace.md` (check off completed steps)
- Modify: project documentation only if the route requires a user-facing note

- [x] Run `cd desktop && pnpm test`.
- [x] Run `cd desktop && pnpm lint` and `pnpm check:px-text`.
- [x] Run `cd desktop && pnpm build`.
- [x] Activate Hermit and run the proportionate repository gate (`just desktop-check` and relevant Rust checks; use `just ci` if observer/Rust code changed).
- [x] Run the installed/mock desktop journey from sidebar to Ship to agent details to activity, recording anything that still requires the user's real-agent acceptance.
- [x] Review the full diff for unrelated files, generated artifacts, secrets, and missing reset wiring.
- [x] Record the implementation decisions and verification result to Memory MCP with `agent: CODEX`.
- [x] Push the branch and open/update a draft PR with screenshots and a clear acceptance checklist; do not mark ready or merge.
- [x] Hand back the local route/build and exact UI acceptance steps for the user.

Verification note (2026-08-02): the mock journey and all feature-owned checks
pass. Real installed-app testing with live agents remains the user's acceptance
gate. `just desktop-check` was run and reaches the repository file-size ratchet;
it reports nine files already oversized in the required `641f5ac4` base. The
Living Ship navigation change was reduced to 999 lines in `AppShell.tsx` and no
longer appears in that failure list.

User UI acceptance route: start the desktop app from this preserved worktree,
open **Living Ship** in the sidebar (`/ship`), then exercise one online-idle
agent, one working agent, one explicit collaboration involving two agents, one
stopped agent, compartment selection, adviser selection, and **Open activity**.
Confirm normal-window label scale and approximate compartment placement before
marking PR #15 ready.

## Plan Self-Review

- [x] Every approved room, agent, state, collaboration rule, interaction, accessibility requirement, and ship-outline constraint is covered by a task.
- [x] Every production behavior begins with a failing test and a witnessed RED state.
- [x] Generated assets are project-local and visually inspected, not placeholders.
- [x] Exact collaboration remains explicit-data-only; no same-channel inference is introduced.
- [x] Verification distinguishes automated/mock acceptance from the user's pending real-agent UI acceptance.
