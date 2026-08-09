# Command Adviser Upstream Buzz Desktop v0.5.8 Sync Implementation Plan

> Execute this plan on `codex/phase-upstream-v0.5.8-sync` from the isolated
> worktree. Activate Hermit before repository commands.

**Goal:** Upgrade the integrated Command Adviser baseline at `e9b121154` from
upstream Buzz Desktop `v0.5.2` to the pinned stable tag `desktop-v0.5.8`
(`f3de860574bb3119018b4592353e9761635aeb07`) without losing user data or
regressing the accepted Command Adviser journeys.

**Architecture:** Preserve a true two-parent upstream merge. Use upstream
v0.5.8 as the platform/runtime base at shared seams, retain the downstream
Command Adviser product layer, and combine both where source, managed-agent,
desktop-shell, or event-kind contracts overlap. Back up live state before the
merge, validate migrations against copies, and do not replace the installed app
until automated and isolated acceptance gates pass.

## Fixed inputs and non-goals

- Downstream base: `e9b12115446be24c6b31b6e4be17dcea64852f9a`.
- Upstream target: `desktop-v0.5.8` at
  `f3de860574bb3119018b4592353e9761635aeb07`.
- Common ancestor: upstream `v0.5.2` at
  `3e48f1b2365d326ee1c9582448d86a99b44ecd5d`.
- The target tag cannot move during this phase.
- This phase does not redesign RAG, Memory MCP, model routing, Command Brief,
  Keeper, remote access, Battle Rhythm, Plans, or Living Ship.
- New upstream functionality is adopted where compatible, but it does not
  displace a working Command Adviser user journey merely to reduce conflicts.

## Task 1: Open and document the phase

- [x] Merge PR #14 and freeze the integrated downstream baseline.
- [x] Fetch upstream tags and verify `desktop-v0.5.8` and its commit.
- [x] Create isolated worktree and branch
      `codex/phase-upstream-v0.5.8-sync` from the merged baseline.
- [x] Run a dry `git merge-tree` compatibility probe.
- [ ] Commit this plan and the roadmap transition.
- [ ] Push the branch and open a draft PR against
      `codex/project-execution-v1`.

## Task 2: Capture the rollback checkpoint

- [ ] Verify the installed `/Applications/Command Adviser.app` bundle name,
      identifier, version, architecture, executable, signature, and hash.
- [ ] Inventory the local Compose services and persistent volumes without
      printing credentials or Keychain values.
- [ ] Run the repository's bounded local-workspace backup into a private,
      absolute backup directory outside all worktrees.
- [ ] Copy the installed app and `~/Library/Application Support/Command Adviser`
      into the same timestamped rollback checkpoint.
- [ ] Capture non-secret configuration metadata, managed-agent definitions,
      source-routing state, file counts, sizes, and SHA-256 inventories.
- [ ] Validate the generated backup manifest and archive enumeration.
- [ ] Record the rollback path and baseline event/count evidence without
      exposing content or secrets.

## Task 3: Start the real two-parent merge

- [ ] Run `git merge --no-ff --no-commit desktop-v0.5.8`.
- [ ] Confirm the unresolved set still matches or is explained relative to the
      39-path dry-run inventory below.
- [ ] Search for hidden binary, submodule, rename/delete, migration, generated,
      or conflict-marker risks.
- [ ] Resolve and stage conflicts by subsystem; do not make one wholesale
      ours/theirs choice across the tree.

### Dry-run conflict groups

1. **CI, release, and build:** three workflows, `Cargo.toml`, both lockfiles,
   `Justfile`, and the desktop Tauri manifest/config.
2. **Agent/model runtime:** `buzz-agent` catalog/config/LLM/types plus desktop
   discovery, environment, model, snapshot, and agent configuration seams.
3. **Relay/event storage:** event kinds, database migration lint, relay ingest,
   and request handling.
4. **Desktop shell:** app state, initial-window setup, Tauri command wiring,
   `App.tsx`, `AppShell`, and agent UI.
5. **Test infrastructure:** E2E bridge, video attachment, helper bridge, and
   managed-agent reader/snapshot tests.
6. **Structural deletion:** upstream deletes `spawn_hash.rs`; downstream has
   modifications that must be relocated into the v0.5.8 spawn contract rather
   than blindly retaining the obsolete file.

## Task 4: Resolve platform, dependency, and release contracts

- [ ] Adopt upstream CI, release, CSP, Tauri, dependency, and packaging changes
      while retaining Command Adviser branding and sidecars.
- [ ] Keep upstream's patched Nostr dependency graph and regenerate lockfiles
      with Cargo rather than hand-editing them.
- [ ] Preserve Command Adviser bundle name, identifier, app-data derivation,
      icon, permissions, entitlements, and signed sidecar inclusion.
- [ ] Preserve the local workspace backup/restore and Command Adviser-specific
      verification commands that the generic upstream tree does not contain.
- [ ] Run formatting, dependency, security, Tauri configuration, and sidecar
      verification immediately after this group is resolved.

## Task 5: Resolve agents, harnesses, models, and source access

- [ ] Adopt v0.5.8 managed-agent discovery, unified add-agent flow, ACP fixes,
      response truncation recovery, timeout handling, and context summarization.
- [ ] Preserve the Command Team's Codex harness defaults, source environment,
      cloud-primary/local-fallback routing, and per-agent MCP/tool policy.
- [ ] Preserve RAG, Memory MCP, World Monitor, and Apple source availability to
      the advisers that require them.
- [ ] Map downstream `spawn_hash.rs` behavior into the upstream v0.5.8 spawn
      snapshot/fingerprint mechanism and remove obsolete code only after tests
      prove equivalent Command Team restart behavior.
- [ ] Retain the eight native Command Adviser personas, doctrine-first guidance
      without a doctrine hard-stop, N2 live-intelligence behavior, and the
      25-call daily World Monitor budget.
- [ ] Add or update focused tests for every combined runtime seam before
      proceeding.

## Task 6: Resolve relay, event, and persistence contracts

- [ ] Adopt upstream private managed-agent event support, channel/community
      protections, message/media changes, and migration-lint updates.
- [ ] Preserve Command Adviser event kinds and owner/community visibility rules
      for briefs, Battle Rhythm, Plans, memory, and Living Ship.
- [ ] Reconcile kind-number constants across Rust, desktop TypeScript, mobile,
      migrations, and tests.
- [ ] Verify new migrations are append-only and compatible with the backed-up
      live database; exercise them only against a disposable restored copy.
- [ ] Confirm thread counters, `h`-tag channel scoping, and explicit-kind query
      gates remain correct.

## Task 7: Resolve the desktop product shell

- [ ] Retain Command Adviser name, icon, naval theme, crest, routes, sidebar,
      Command Console, Battle Rhythm, Plans, and Living Ship.
- [ ] Adopt upstream community switching, identity recovery, notification,
      media, terminal, project, and agent-creation improvements at their shared
      seams.
- [ ] Preserve community-reset registration for all downstream module-level
      caches and Living Ship observer stores.
- [ ] Keep readable text on the rem-based type scale and run the px-text guard.
- [ ] Update E2E bridge mocks to cover both upstream v0.5.8 behavior and the
      downstream product routes.

## Task 8: Prove feature preservation

- [ ] Command Team: every adviser is present, messageable, correctly routed,
      and able to persist a conversation outcome.
- [ ] Doctrine RAG: a fixed semantic canary returns document, section/chunk,
      and `point_id`, not merely a collection listing.
- [ ] Memory MCP: write and recall a disposable event through the app path.
- [ ] Maritime N2: World Monitor returns a live result and provenance remains
      distinct from doctrine and planning assumptions.
- [ ] Model routing: one cloud-primary turn and one local-fallback turn complete
      with truthful UI route state.
- [ ] Command Brief: a manual brief includes current conversation outcomes,
      citations, freshness, and useful degraded-source labels.
- [ ] Apple integration: read-only inputs and one-way publication work and fail
      softly on permission denial.
- [ ] Battle Rhythm/Plans: existing data loads and disposable edits preserve
      multiday/all-day, 24-hour, timezone, playbook, dependency, critical-path,
      Kanban, and calendar-link contracts.
- [ ] Living Ship: working, collaborating, idle, unavailable, and activity
      navigation states remain correct at the accepted window size.

## Task 9: Run repository and migration gates

- [ ] Search for unresolved merge entries and conflict markers.
- [ ] Run focused Rust tests for agents, source policies, event kinds,
      persistence, brief orchestration, and app startup.
- [ ] Run focused desktop tests and E2E journeys for navigation, agents,
      Command Adviser branding, Battle Rhythm, Plans, and Living Ship.
- [ ] Run root and desktop Rust format/clippy/check/test gates.
- [ ] Run desktop, web, and mobile checks and tests.
- [ ] Run `cargo deny check` and the full `just ci` gate.
- [ ] Build the release bundle and verify all required sidecars.
- [ ] If a failure appears, reproduce it narrowly, add or retain a regression
      test, repair the smallest shared seam, then rerun focused and full gates.

## Task 10: Rehearse recovery and install

- [ ] Restore the backup into an isolated test profile or equivalent disposable
      environment and validate Postgres, MinIO, Memory, and Command Brief state.
- [ ] Launch the candidate against copied state and verify migrations and
      representative existing records.
- [ ] Reconfirm the rollback application and backup checksums.
- [ ] Gracefully stop the installed app, replace only the application bundle,
      and preserve live user data.
- [ ] Launch the explicit upgraded Command Adviser bundle and confirm it uses
      the established application-data location.

## Task 11: Installed-app acceptance and phase close

- [ ] Confirm identity, communities, conversations, agents, Battle Rhythm,
      Plans, briefs, and Living Ship state survived the upgrade.
- [ ] Run the live acceptance journeys from Task 8 using disposable records
      where writes are required, then remove only those disposable records.
- [ ] Confirm no persistent degraded/error state was introduced.
- [ ] Record exact test, backup, migration, installation, and rollback evidence
      in the draft PR and Memory MCP with agent `CODEX`.
- [ ] Commit and push the verified merge and compatibility repairs.
- [ ] Leave the PR open for user testing unless every required user journey can
      be completed without credentials, permissions, or subjective acceptance.
- [ ] Update the roadmap with the new baseline and the precise next phase.

