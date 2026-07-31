# Command Adviser Upstream Buzz v0.5.2 Sync Implementation Plan

> Execute this plan on `codex/upstream-v0.5.2-sync` from the isolated worktree.
> Activate Hermit before repository commands.

**Goal:** Merge stable upstream Buzz `v0.5.2` into Command Adviser without losing
or rewriting the user's existing application data, then deliver an installed
build ready for user testing.

**Architecture:** Preserve a true two-parent merge. Adopt upstream platform and
agent-runtime changes, then integrate the Command Adviser product layer at the
shared seams. Protect the live installation with verified app and state backups;
exercise migrations and tests against non-live state; finish with read-only live
acceptance.

## Task 1: Freeze the baseline and open the phase

- [x] Create `.worktrees/codex-upstream-v0.5.2-sync`.
- [x] Create branch `codex/upstream-v0.5.2-sync` from
      `codex/project-execution-v1`.
- [x] Fetch and verify upstream tag `v0.5.2`.
- [x] Run the full pre-merge `just ci` baseline.
- [x] Write the approved design and this executable plan.
- [ ] Commit the phase documents.
- [ ] Push the branch and open a draft PR targeting
      `codex/project-execution-v1`.

## Task 2: Discover and protect live user state

- [ ] Read the installed `Command Adviser.app` metadata and verify its bundle
      identifier, version, signing state, and executable.
- [ ] Trace application, archive, Battle Rhythm, Plans, identity, configuration,
      and relay persistence paths from source and the running installation.
- [ ] Inventory live paths with file counts and sizes, without reading or
      printing secret values.
- [ ] Create a timestamped rollback copy of the installed application.
- [ ] Create a timestamped copy/archive of each live user-data/config path.
- [ ] Record hashes and validate that every backup can be enumerated.
- [ ] Confirm the live state has not been modified.

## Task 3: Merge the stable upstream release

- [ ] Run `git merge --no-ff --no-commit v0.5.2`.
- [ ] Record the complete unresolved-path inventory.
- [ ] Group conflicts into platform/runtime, shared integration seams, product
      features, branding/configuration, tests, and generated artifacts.
- [ ] Confirm no unexpected submodule, binary, or migration conflict is hidden.

## Task 4: Resolve platform and agent-runtime conflicts

- [ ] Adopt upstream relay/core/auth/database/search/media/audit changes except
      where a Command Adviser protocol extension requires a combined resolution.
- [ ] Adopt the upstream ACP and harness lifecycle as the base.
- [ ] Reconcile Codex provider/model discovery with the Command Adviser default
      harness and managed-agent commissioning.
- [ ] Preserve cloud-first/local-first routing and fallback semantics at the
      product routing layer.
- [ ] Add or update focused regression tests before accepting each shared seam.

## Task 5: Resolve the Command Adviser desktop integration

- [ ] Combine upstream Tauri application state, commands, permissions,
      entitlements, and dependencies with Command Adviser native modules.
- [ ] Preserve the bundle identifier and storage-directory derivation.
- [ ] Preserve Command Team personas, Logistics Adviser, N2, doctrine-guided
      advice, and their managed-agent visibility.
- [ ] Preserve RAG, Memory MCP, World Monitor, Apple productivity, and command
      briefing integrations.
- [ ] Preserve Command Adviser branding, crest, symbolic role icons, navigation,
      and decision-first briefing presentation.
- [ ] Accept unrelated upstream desktop improvements and community reset fixes.

## Task 6: Preserve Battle Rhythm and Plans

- [ ] Preserve manual and imported events, recurrence, multiday rendering,
      24-hour time, spellcheck, timezone changes, and Apple Calendar publication.
- [ ] Preserve FAS/Longcast/Shortcast source replacement and ship-routine rules.
- [ ] Preserve all-day programme location colours and the weekly all-day lane.
- [ ] Preserve Plans, Kanban, playbooks, dependencies, critical path, assignment,
      HOD lists, exports, and calendar-linked milestones.
- [ ] Run focused UI/unit tests for these contracts.

## Task 7: Reconcile dependencies and validate data compatibility

- [ ] Resolve Cargo, pnpm, Flutter, and Tauri configuration/lockfiles using the
      repository toolchains.
- [ ] Search for unresolved conflict markers and unmerged paths.
- [ ] Inspect every new database migration and local-store schema change.
- [ ] Exercise migrations only against temporary or copied state.
- [ ] Prove existing application identifiers and live storage paths are stable.
- [ ] Build the desktop frontend and Tauri crate.

## Task 8: Run full verification and repair regressions

- [ ] Run focused ACP/provider/model/persona tests.
- [ ] Run focused Command Brief and trusted-source tests.
- [ ] Run focused Battle Rhythm, Plans, and Apple integration tests.
- [ ] Run desktop E2E tests for the affected user journeys.
- [ ] Run `just ci`.
- [ ] For any failure, identify the root cause, add or retain a regression test,
      fix the smallest affected seam, and rerun the focused then full gate.

## Task 9: Build and install the upgrade safely

- [ ] Produce the release macOS application using the existing Command Adviser
      packaging/signing path.
- [ ] Verify bundle name, identifier, icon, entitlements, architecture, and
      signature before installation.
- [ ] Reconfirm the rollback application and user-data backups.
- [ ] Gracefully quit the installed app if it is running.
- [ ] Install the upgraded bundle without deleting user data.
- [ ] Launch the explicit upgraded bundle.

## Task 10: Perform read-only acceptance

- [ ] Confirm the app opens as Command Adviser.
- [ ] Confirm existing identity, communities, conversations, and agents remain.
- [ ] Confirm previously entered Battle Rhythm and Plans data remains visible.
- [ ] Confirm cloud/local routing preference and trusted-source configuration
      remain available.
- [ ] Confirm no degraded/error state was introduced by the merge.
- [ ] Do not create, edit, publish, import, or reschedule user content.
- [ ] Leave the application at the appropriate screen for user testing.

## Task 11: Complete the phase

- [ ] Commit the resolved merge and any verified compatibility fixes.
- [ ] Push the complete branch.
- [ ] Update the draft PR with test, backup, installation, rollback, and known
      limitation evidence.
- [ ] Record the significant upgrade decisions, paths, test evidence, and
      rollback checkpoint in Memory MCP with agent `CODEX`.
- [ ] Hand off the installed application for user testing with the rollback
      location and the first recommended write-oriented checks.
