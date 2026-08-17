# Disconnected Command Adviser Acceptance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove that the installed Command Adviser, qualified Gemma runtime, Mac-local RAG, Mac-local Memory, and autonomous skills form a recoverable sea-going product when Internet and home-LAN services are unavailable.

**Architecture:** Phase 5 adds no new runtime service. It inventories and checks the components delivered by Phases 1–4, produces a checksum-addressed deployment manifest without duplicating large local payloads, and runs bounded readiness, recovery, queue, and soak checks. A portable materialisation is optional and must target an operator-selected destination with sufficient free space. Physical network isolation and installed-app journeys remain explicit acceptance actions rather than being inferred from unit tests.

**Tech Stack:** Bash, Python 3.12 standard library, JSON/JQ, macOS `launchctl`/`codesign`, LM Studio REST API, existing Command Adviser shell canaries, Rust/Tauri tests.

## Global Constraints

- Reuse the qualified runtime: `gemma4-26b-official`, 65,536-token context, reasoning off, parallelism one.
- Reuse the existing loopback RAG and Memory services and the signed Buzz event history. Do not add another daemon, database, snapshot format, or credential store.
- Never print, copy into Git, or hash the contents of Keychain secrets. Protected configuration is represented by file identity, permissions, and an encrypted backup artefact only.
- Do not duplicate model weights on the MacBook during ordinary readiness checks. The manifest records their canonical path, size, and SHA-256. Portable materialisation requires an explicit destination.
- Automated checks may observe network state but do not disable interfaces. The owner controls the physical offline window after the candidate is installed.
- A health endpoint is supporting evidence only. RAG acceptance requires a substantive semantic result with source metadata and `point_id`; model acceptance requires a real generation from the qualified instance.
- Preserve the installed application, application data, relay history, Memory vault, RAG snapshot, and the last working rollback application throughout the phase.
- Activate Hermit before repository tests or Git hooks, and create signed commits with `git commit -s`.

## Task 1: Define the Checksum-Addressed Sea-Going Manifest

**Files:**
- Create: `scripts/build-seagoing-manifest.py`
- Create: `scripts/tests/build-seagoing-manifest-test.py`

- [ ] Write failing tests for deterministic path ordering, regular-file hashing, recursive directory inventory, symlink rejection, missing required components, permission-only protected-config representation, duplicate logical component names, and insufficient free-space rejection for portable materialisation.
- [ ] Accept named component inputs for the installed app, qualified model, embedding model, local RAG snapshot/manifest, Memory runtime/vault backup, relay binary, and recovery/operations material.
- [ ] Emit canonical JSON with schema version, creation time, host architecture, component role, canonical path, byte size, SHA-256, and whether the payload is present in place or materialised.
- [ ] Derive the bundle ID from canonical component metadata excluding the creation time and output path.
- [ ] Add an explicit `--materialize DESTINATION` mode that copies payloads only after calculating required space plus a 20% remaining-free-space reserve. The normal mode writes metadata only.
- [ ] Ensure protected configuration inputs must be mode `0600`, are represented by metadata and an encrypted backup artefact, and are never emitted as plaintext content.
- [ ] Run `python3 scripts/tests/build-seagoing-manifest-test.py`.

## Task 2: Add One Truthful Disconnected-Readiness Check

**Files:**
- Create: `scripts/check-disconnected-readiness.sh`
- Create: `scripts/tests/check-disconnected-readiness-test.sh`

- [ ] Write failing fixture tests for missing or unsigned app, wrong LM Studio instance/config, model generation failure, unavailable relay, failed RAG semantic canary, unavailable Memory service, missing active skill projection, low disk headroom, and a complete pass.
- [ ] Require the installed application to exist and pass `codesign --verify --deep --strict`.
- [ ] Reuse `scripts/check-offline-model.sh` for the exact runtime and real generation canary rather than duplicating its contract.
- [ ] Probe the installed relay and Mac-local Memory endpoints on literal loopback.
- [ ] Run a configured collection-specific RAG query and require document identity, quoted location, `point_id`, and active snapshot identity.
- [ ] Verify at least one managed active skill projection when the phase manifest declares skills present; do not fail a clean first install merely because no skill has yet been learned.
- [ ] Check that the bundle manifest resolves every required component and that the data volume retains at least 20% free space after the declared recovery reserve.
- [ ] Emit one redacted JSON report containing `ready: true|false`, component reasons, identities, free-space result, and network observation. Never claim external networking is disabled unless every observed non-loopback default route is absent.
- [ ] Run `bash scripts/tests/check-disconnected-readiness-test.sh`.

## Task 3: Add a Bounded Offline Soak Monitor

**Files:**
- Create: `scripts/monitor-disconnected-soak.py`
- Create: `scripts/tests/monitor-disconnected-soak-test.py`

- [ ] Write failing tests using a fake probe and temporary directories for healthy samples, a cloud-attempt marker, a stuck queue, service loss, duplicate brief publication, and excessive disk growth.
- [ ] Poll the readiness probe and configured audit/status files at a configurable interval and duration; default operational duration is eight hours while tests use seconds.
- [ ] Record only redacted counters and hashes: qualified model instance, queue depth/active age, service readiness, completed brief/run IDs, publication IDs, cloud-attempt count, and monitored-directory byte totals.
- [ ] Fail on any observed cloud attempt, component loss beyond a bounded grace sample, queue item older than the configured limit, duplicate publication ID for one run, or disk growth beyond the configured absolute and percentage bounds.
- [ ] Write progress atomically so interruption preserves the latest complete sample and rerunning with `--resume` continues the same acceptance record.
- [ ] Run `python3 scripts/tests/monitor-disconnected-soak-test.py`.

## Task 4: Prove Existing Recovery and Capacity-One Behaviour

**Files:**
- Modify: `scripts/check-disconnected-readiness.sh`
- Create: `docs/command-console/offline-command-adviser-operations.md`

- [ ] Run the existing model exact-response checker and three-request FIFO queue canary against `gemma4-26b-official`; verify no second loaded instance appears.
- [ ] Run the existing Command Brief audit/recovery tests proving an offline publication stays queued and republishes the same event ID without duplication.
- [ ] Run the existing adaptive-memory and autonomous-skill checks, including rebuild from authoritative history.
- [ ] Document a bounded restart matrix for Command Adviser, LM Studio, local RAG, local Memory, the relay, and the Mac. Each restart has one expected readiness failure, one recovery command/action, and one post-recovery semantic or generation canary.
- [ ] Document backup, restore, RAG rollback, skill rollback, pre-sail refresh, post-deployment reconciliation, and the exact stop conditions that require returning to the last working app.

## Task 5: Build and Install the Phase Candidate Safely

**Files:**
- Modify: `docs/command-console/offline-command-adviser-acceptance-record.md`
- Modify: `docs/command-console/ROADMAP.md`

- [ ] Run focused script tests and targeted Rust recovery/scheduler tests, then `. ./bin/activate-hermit && just ci`.
- [ ] Produce the release application and sidecars using the existing Command Adviser packaging path.
- [ ] Verify the Developer ID signature, retain a timestamped rollback copy of `/Applications/Command Adviser.app`, and preserve existing application data before replacing the installed app.
- [ ] Launch the installed candidate once with networking available and run the readiness check. Any one-time Keychain approval is an owner action; repeat prompts after a successful approval are a product failure.
- [ ] Record exact build commit, app signature, component manifest ID, rollback paths, test results, and the online preflight result in the acceptance record.

## Task 6: Run the Physical Disconnected User Journeys

**Files:**
- Modify: `docs/command-console/offline-command-adviser-acceptance-record.md`

- [ ] With the owner controlling network isolation, verify the readiness report observes no external default route while retaining loopback services.
- [ ] Complete one installed-app request using one adviser, one using two advisers, and one using three advisers. Verify capacity-one generation, cited local RAG evidence, local Memory write/readback, and no cloud attempt.
- [ ] Start a Daily Command Brief, interrupt Command Adviser after at least one durable specialist checkpoint, restart it, and verify the run resumes without losing completed work or publishing twice.
- [ ] Restart LM Studio, local RAG, local Memory, and the relay one at a time, then run the documented post-recovery canary after each restart.
- [ ] Cold restart the Mac while still disconnected. Verify the exact model instance, active RAG snapshot, Memory active view, skill projection, relay, and pending work recover without a second model runtime.
- [ ] Run the eight-hour soak monitor across an overnight Daily Command Brief. Require zero cloud attempts, zero duplicate publications, no stuck queue item, bounded disk growth, and no silent component loss.
- [ ] Record the exact run IDs, publication IDs, snapshot/model/skill identities, restart results, soak report hash, and any truthful limitation. Do not mark Phase 5 complete from automated tests alone.

## Task 7: Close the Phase

**Files:**
- Modify: `docs/command-console/ROADMAP.md`
- Modify: `docs/command-console/offline-command-adviser-acceptance-record.md`

- [ ] Re-run the focused gates and `just ci` after the final change.
- [ ] Update the draft PR with the manifest ID, tests, installed-app result, offline journey evidence, recovery outcomes, soak report, rollback paths, and remaining limitations.
- [ ] Record only the high-value deployment contract and physical acceptance result in Memory MCP with `agent="CODEX"`.
- [ ] Mark the PR ready and merge only after the owner confirms the installed disconnected journey. Keep optional Phase 6 model refinement closed.

## Acceptance Boundary

The phase is complete only when the installed application passes the physical disconnected journeys and eight-hour soak. Repository tests, loopback health probes, an LM Studio catalogue entry, or an online local-model brief are useful prerequisites but cannot establish sea-going readiness by themselves.
