# Command Adviser Upstream Buzz v0.5.2 Sync Design

**Date:** 31 July 2026

## Purpose

Bring the maintained Buzz foundation in `block/buzz` up to the stable `v0.5.2`
release while preserving Command Adviser as the product the user already
operates. The upgrade must retain the user's existing conversations, agents,
Battle Rhythm events, planning data, settings, identity, and trusted-source
configuration.

This is an upstream maintenance phase, not a redesign. Post-`v0.5.2` upstream
work and new Command Adviser features are outside this phase.

## Fixed inputs

- Product branch: `codex/project-execution-v1`
- Product baseline: `15d0279ffaca3fc2b8c92cd6466fc6472bf313cd`
- Common ancestor with upstream: `76aeae703664a6a6741b82771df67c546886aafd`
- Upstream release: tag `v0.5.2`
- Upstream release commit: `3e48f1b2365d326ee1c9582448d86a99b44ecd5d`
- Upgrade branch: `codex/upstream-v0.5.2-sync`

The pre-merge `just ci` gate passed from a clean isolated worktree.

## Product boundary

Upstream is authoritative for the general Buzz platform:

- ACP and managed-agent lifecycle
- Codex, Claude Code, and generic agent harness integration
- provider/model discovery and upstream model configuration
- relay, core protocol, database, authentication, media, search, and audit
- shared desktop/mobile platform behaviour and dependency maintenance

Command Adviser remains authoritative for the commissioned naval product:

- Command Adviser name, application identity, crest, visual system, and role icons
- Command Team personas and Chief of Staff orchestration
- Battle Rhythm calendar, ship routine, programme colours, imports, Apple Calendar
  publication, and timezone behaviour
- Plans, Kanban, playbooks, critical path, assignment, exports, and HOD task lists
- cloud-first/local-first routing policy and its user-facing selector
- RAG, Memory MCP, World Monitor, Apple productivity, and planning-evidence
  integrations
- Daily Command Brief presentation and evidence disclosure

Where both sides changed the same integration seam, the result will preserve the
Command Adviser feature contract while adopting the upstream implementation
shape. Whole-file `ours` or `theirs` resolution is permitted only where the file
has a single clear owner; shared seams require a deliberate combined resolution.

## History and merge strategy

Use a real, non-fast-forward merge of the signed stable release tag. Do not
rebase the product history and do not recreate the upstream release through
cherry-picks. The merge commit will retain both histories and make future
upstream comparisons intelligible.

The work runs in an isolated worktree so the installed application and current
product checkout remain available throughout the upgrade.

## Data-preservation contract

No step in this phase may reset, seed, clear, migrate destructively, or replace
the live Command Adviser state.

Before the upstream merge:

1. Resolve the installed bundle identifier and every live data/config path from
   the application and source code.
2. Inventory the installed app, Application Support state, preferences,
   workspace identity/configuration, and any local relay persistence used by
   this installation.
3. Make a timestamped, read-only rollback copy of the installed application.
4. Make a timestamped backup of user-controlled application state without
   exporting Keychain secret values.
5. Record file counts, sizes, and checksums sufficient to validate the backup.
6. Verify that the backup can be enumerated before changing the installed app.

Keychain items remain in Keychain. The backup records only the service/account
references needed to diagnose access; it never prints or archives credential
values.

Any database migrations introduced by upstream must be additive or otherwise
proven compatible against a copy or temporary test state before the upgraded
application opens the live store.

## Conflict-resolution sequence

1. Merge `v0.5.2` without committing.
2. Inventory all unresolved paths and classify them by ownership.
3. Resolve build metadata and lockfiles after source conflicts, not before.
4. Resolve relay/core and ACP/runtime files using upstream as the base.
5. Reapply Command Adviser provider routing and managed-persona expectations
   over the upstream harness interfaces.
6. Resolve shared Tauri commands, app state, permissions, and configuration as
   combined seams.
7. Preserve Command Adviser features and branding while accepting unrelated
   upstream desktop improvements.
8. Regenerate mechanical artifacts only with repository tools.
9. Commit one auditable merge once the tree builds and targeted tests pass.

## Verification gates

The upgrade is not ready for installation until all applicable gates pass:

- no unresolved merge markers or unmerged paths
- focused tests for ACP/provider/model/persona integration
- focused tests for Command Brief, RAG/Memory/World Monitor, Battle Rhythm,
  Plans, Apple Calendar, programme colours, and user-data storage
- application identifier and storage paths remain stable
- migration compatibility is proven without touching live state
- `just ci` passes from the merged tree
- the macOS application builds successfully
- the produced app retains Command Adviser identity and entitlements

After backing up the existing installation, install the new build and perform a
read-only live acceptance:

- app launches as Command Adviser
- existing workspace identity and configured agents are visible
- existing conversations remain present
- previously entered Battle Rhythm and Plans data is visible
- cloud/local routing and trusted-source configuration remain selected
- no write, import, reschedule, republish, or user-data mutation is performed
  during acceptance

The user performs the first write-oriented test after handoff.

## Rollback

If any gate fails:

- keep the current installed application and live data untouched, or
- if installation has occurred, quit the new app, restore the timestamped prior
  app bundle, and retain the untouched data backup for diagnosis.

Do not roll back by deleting live state. Data restoration is a separate,
explicit recovery action and is unnecessary when the upgraded build has only
performed read-only acceptance.

## Completion

The phase is complete when the branch and draft PR contain the merged release,
all required gates pass, an upgraded Command Adviser build is installed with a
verified rollback copy, the existing user data is visible in read-only
acceptance, and the user is invited to begin testing.
