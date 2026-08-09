# HMAS Supply Command Console Phase 3 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task with fresh implementer and reviewer agents.

**Goal:** Make approved knowledge, command memory, Calendar, Reminders, Notes, and allowlisted files available to `OFFICIAL` advisers from authenticated loopback services on the MacBook, including offline operation and conflict-safe home synchronisation.

**Architecture:** Keep the existing home RAG and Memory services as sources of authority, but never admit their current cleartext LAN endpoints into an `OFFICIAL` agent route. Extend `NavigatorRAN/RAG-MCP` with a signed, reproducible snapshot bundle and golden-query validation; extend `NavigatorRAN/AgentMemory` with revision-aware replication; and add Buzz-managed Mac-local authorities, SSH-pinned synchronisation, strict Tauri commands, and a signed Swift Apple-input helper. LM Studio receives only exact allowlisted native MCP integrations on literal loopback.

**Tech Stack:** Rust, TypeScript/React, Tauri 2, Python 3.12, FastMCP, Qdrant 1.17, Docker Compose, Swift 6/Xcode 26, EventKit, Apple Events, Ed25519/SHA-256, SSH, SQLite, Hermit.

## Global constraints

- Every conversation, artefact, workflow, agent run, snapshot, revision, and Apple-input result defaults to `OFFICIAL`.
- `OFFICIAL` agent inference and MCP calls use literal loopback only. There is no LAN or cloud fallback.
- The live home Memory MCP at `192.168.1.26:8006` and RAG MCP at `192.168.1.107:8005` are currently unauthenticated cleartext services and are never direct `OFFICIAL` integrations.
- Home synchronisation uses a pinned SSH host identity, a loopback tunnel, application authentication, bounded payloads, and resumable cursors. No trust is inferred merely because an address is on the home LAN.
- The Mac Memory node is writable and authoritative for the command application. Home RAG remains authoritative for the approved document corpus.
- Retrieved text and Apple/file content are untrusted evidence, never instructions. Source metadata and retrieval timestamps are preserved.
- Snapshot activation is staging-first and atomic. A failed checksum, signature, schema, model, point-count, or golden-query gate leaves the active snapshot unchanged.
- Memory append-only events merge automatically. Divergent stable-entity revisions create visible conflicts; last-write-wins is prohibited and conflicted fields are excluded from unattended briefs.
- Calendar, Reminders, Notes, and file access is read-only. Permission canaries do not prompt; permission requests are separate explicit user actions. Denial degrades one source and never blocks the entire brief.
- Notes access uses a fixed application-owned Apple Event operation because EventKit does not provide Notes. Renderer input can never supply a script.
- File inputs are selected by native picker, canonicalised, allowlisted, bounded, and symlink-safe.
- No ship-control, navigation-control, communications, combat, logistics, personnel, or external operational system is connected.
- Public APIs are documented; production Rust contains no new `unwrap()` or `expect()`; all untrusted counts, strings, payloads, processes, and timeouts are bounded.
- Each repository remains independently buildable and reviewable. Phase 3 uses stacked draft PRs in Buzz, AgentMemory, and RAG-MCP before deployment.

## Verified starting state

- Buzz Phase 2 is complete at `d83ace9f` and allows only explicit literal-loopback `ephemeral_mcp` integrations with per-server tool allowlists.
- The Buzz contract module already defines strict `KnowledgeSnapshotManifest`, `MemoryRevision`, and `ReplicationEnvelope` shapes, but does not yet verify their cryptographic values.
- Home Memory MCP is FastMCP `memory` 3.2.4 at `http://192.168.1.26:8006/mcp`; canonical Markdown is under `/mnt/aishareddrive/family-agents/memory`, and SQLite is a derived cache. No revision, cursor, conflict, tombstone, or replication API exists.
- Home RAG MCP is `rag` 1.27.0 at `http://192.168.1.107:8005/mcp/`; Qdrant 1.17.1 contains collection `documents` with 93,483 points at reconnaissance time. Dense embedding, sparse embedding, and reranking currently depend on separate services at `192.168.1.11`.
- Buzz has no existing Swift source or Xcode project. The macOS app metadata lives in `desktop/src-tauri/Info.plist`, `Entitlements.plist`, and `tauri.conf.json`.

## Repository and branch topology

- Buzz: `NavigatorRAN/buzz`, branch `codex/phase-3-knowledge-productivity`, stacked on `codex/phase-2-local-agent-runtime`.
- AgentMemory: `NavigatorRAN/AgentMemory`, branch `codex/phase-3-memory-replication`, stacked on its `main`.
- RAG-MCP: `NavigatorRAN/RAG-MCP`, branch `codex/phase-3-offline-snapshots`, stacked on its `main`.
- Do not deploy either home service until its branch tests, review, backup, and rollback checks pass.

### Task 1: Add signed RAG snapshot export and exact manifests

**Repository:** `NavigatorRAN/RAG-MCP`

**Files:**

- Add: `rag_snapshot/__init__.py`
- Add: `rag_snapshot/manifest.py`
- Add: `rag_snapshot/export.py`
- Add: `rag_snapshot/crypto.py`
- Add: `tests/test_snapshot_manifest.py`
- Add: `tests/test_snapshot_export.py`
- Modify: `requirements/ingest.freeze.txt`
- Modify: `README.md`

**Interfaces:**

- Produces canonical `manifest.json`, `manifest.sig`, Qdrant snapshot, collection catalogue, service commit, schema/counts, document metadata, dense/sparse/reranker identities, golden-query file, and SHA-256 object checksums.
- Consumes an Ed25519 signing key from a file descriptor or protected path supplied by the operator; the private key is never written into the bundle or logs.

**Steps:**

1. Write failing tests for canonical byte-for-byte manifest serialisation, strict field bounds, path traversal rejection, duplicate object rejection, exact collection schema/counts, required retrieval model roles, and checksum verification.
2. Write failing tests proving the exported manifest identifies `bge-m3`, the sparse implementation/version, reranker implementation/version, RAG commit, Qdrant version, collection names, schema, point counts, snapshot time, and golden-query digest.
3. Write a fake-Qdrant export test that creates a snapshot, streams it with a size limit, records object hashes while writing, and removes partial output after any failure.
4. Implement deterministic canonical JSON, Ed25519 detached signatures, streaming SHA-256, and a staging-only exporter. Reject symlinks, special files, absolute archive paths, control characters, unbounded metadata, and mutable output directories.
5. Add `python -m rag_snapshot.export` with explicit endpoint, output, signing-key, model-manifest, catalogue, and golden-query arguments. It must not discover secrets or endpoints from unrelated ambient environment variables.
6. Document backup-first operation and prove no export endpoint mutates the active Qdrant collection except Qdrant's snapshot operation.
7. Run `pytest -q`, Python compilation, the existing smoke-test fixtures, and formatting/lint available in the repo.
8. Commit only Task 1 changes.

### Task 2: Add staged RAG import, local retrieval, and equivalence gates

**Repository:** `NavigatorRAN/RAG-MCP`

**Files:**

- Add: `rag_snapshot/import_bundle.py`
- Add: `rag_snapshot/activate.py`
- Add: `rag_snapshot/golden.py`
- Add: `tests/test_snapshot_import.py`
- Add: `tests/test_snapshot_activation.py`
- Add: `tests/test_golden_queries.py`
- Add: `compose/docker-compose.macos.yml`
- Add: `compose/macos.env.example`
- Modify: `retrieval/config.py`
- Modify: `retrieval/retrieve.py`
- Modify: `retrieval/mcp_server.py`
- Modify: `README.md`

**Interfaces:**

- Produces a loopback RAG MCP URL and a machine-readable readiness record containing active snapshot ID, signature fingerprint, schema/counts, retrieval model versions, golden-query result, and last successful activation.
- Provides read-only MCP tools with an exact allowlist for search, catalogue, source retrieval, and snapshot status.

**Steps:**

1. Write failing import tests for signature mismatch, untrusted signer, checksum mismatch, traversal, symlink, decompression bomb, schema mismatch, model mismatch, point-count mismatch, insufficient free space, interrupted copy, and stale/incomplete staging state.
2. Write failing activation tests proving validation happens against staging, the active symlink/directory switch is atomic, the prior snapshot remains available for rollback, and failed validation never changes active state.
3. Add deterministic golden-query tests comparing expected document IDs/collections/metadata filters and bounded retrieval-quality thresholds rather than exact unstable scores.
4. Implement the importer and activation journal with fsync/rename semantics, explicit capacity calculation for staging plus rollback, and a minimum 20% free-space postcondition.
5. Add a macOS Compose topology for Qdrant, retrieval/MCP, dense `bge-m3`, matching sparse retrieval, and reranking. Bind all published service ports to `127.0.0.1`, disable telemetry where supported, and pin image/model/service versions in the manifest.
6. Isolate retrieved content from system instructions in the MCP response schema. Return source ID, collection, document/chunk IDs, snapshot, timestamp, quoted location, score components, and an explicit `untrusted_evidence` marker.
7. Add prompt-injection fixtures proving retrieved instructions are returned only as quoted evidence and never alter tool policy, server configuration, or result shape.
8. Run unit/integration tests against disposable Qdrant, then run home and Mac mirror golden queries without activating a failed mirror.
9. Commit only Task 2 changes.

### Task 3: Add revision-aware Memory MCP storage and replication APIs

**Repository:** `NavigatorRAN/AgentMemory`

**Files:**

- Add: `MemoryMCPServer/src/memory_mcp/revisions.py`
- Add: `MemoryMCPServer/src/memory_mcp/replication.py`
- Add: `MemoryMCPServer/src/memory_mcp/auth.py`
- Modify: `MemoryMCPServer/src/memory_mcp/storage.py`
- Modify: `MemoryMCPServer/src/memory_mcp/index.py`
- Modify: `MemoryMCPServer/src/memory_mcp/server.py`
- Add: `MemoryMCPServer/tests/test_revisions.py`
- Add: `MemoryMCPServer/tests/test_replication.py`
- Add: `MemoryMCPServer/tests/test_replication_auth.py`
- Modify: `MemoryMCPServer/README.md`

**Interfaces:**

- Produces immutable objects, manifests, `MemoryRevision`, `ReplicationEnvelope`, cursor acknowledgement, conflict listing/resolution, tombstone, readiness, and backup/export operations.
- Uses stable node IDs and globally unique ULID event IDs. Stable entity revisions contain parent revision IDs and content hashes.

**Steps:**

1. Write failing tests for immutable content-addressed objects, canonical revision hashes, unique event IDs, stable node identity, parent validation, duplicate delivery, deterministic cursor order, bounded pagination, and resumable acknowledgement.
2. Write failing merge tests: append-only events auto-merge; identical revisions deduplicate; a descendant advances; divergent entity parents create a conflict; neither branch overwrites the other; conflicted fields are absent from unattended views until explicitly resolved.
3. Write failing tombstone and restore tests proving deletion is replicated, retained for the configured horizon, idempotent, and recoverable from backup.
4. Add application authentication with constant-time bearer verification, request/body/rate bounds, redacted errors, and separate read/replicate/admin capabilities. Authentication is mandatory for replication endpoints even when legacy MCP access remains temporarily compatible.
5. Implement a revision journal beside the canonical vault, with SQLite as a rebuildable index only. Use atomic writes and preserve existing Markdown compatibility.
6. Implement bounded HTTP replication endpoints and matching MCP administration tools. No endpoint accepts a caller-supplied filesystem path.
7. Add conflict projection to existing recall/entity responses so unattended consumers can exclude conflicted fields while reviewers can inspect both branches and provenance.
8. Run the Memory MCP pytest suite, index rebuild tests, Swift transport tests affected by response changes, and smoke fixtures.
9. Commit only Task 3 changes.

### Task 4: Add the Mac-local Memory authority and pinned home synchronisation

**Repositories:** `NavigatorRAN/AgentMemory`, then `NavigatorRAN/buzz`

**Files:**

- Add: `MemoryMCPServer/src/memory_mcp/replicate_cli.py`
- Add: `MemoryMCPServer/tests/test_replicate_cli.py`
- Modify: `MemoryMCPServer/pyproject.toml`
- Add: `desktop/src-tauri/src/command_services/mod.rs`
- Add: `desktop/src-tauri/src/command_services/memory.rs`
- Add: `desktop/src-tauri/src/command_services/ssh.rs`
- Add: `desktop/src/shared/api/tauriCommandServices.ts`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src-tauri/Cargo.toml`
- Modify: `docker-compose.yml`
- Modify: `scripts/check-local-services.sh`
- Modify: `scripts/backup-local-workspace.sh`
- Modify: `scripts/restore-local-workspace.sh`
- Modify corresponding Rust and shell tests

**Interfaces:**

- Produces authenticated loopback Memory MCP readiness and an explicit user-triggered/scheduled sync operation with node IDs, cursors, conflicts, object counts, last success, and pinned-host evidence.
- Writes command-agent memory immediately to the Mac node; home synchronisation is asynchronous and never blocks a local write.

**Steps:**

1. Write CLI tests for pull, push, resume, duplicate delivery, interrupted transfer, divergent revisions, tombstones, authentication failure, host-key mismatch, and redacted diagnostics.
2. Implement the replication CLI against loopback URLs only. The remote URL is reached exclusively through a separately launched SSH tunnel with strict host-key checking and a dedicated known-hosts file.
3. Add Buzz Tauri configuration stored outside renderer-controlled environment: local port, home host alias, pinned host fingerprint, remote loopback port, node IDs, Keychain credential references, schedule, and exact tool allowlist.
4. Spawn and supervise the tunnel without a shell; use fixed arguments, no agent forwarding, no remote command, bounded startup/readiness time, and deterministic teardown. Reject IP/host mismatch, changed key, wildcard binds, proxy variables, redirects, and unprotected credential files.
5. Add the local Memory service to the Mac service topology bound to literal loopback with a persistent local vault and rebuildable index. Extend service readiness, backup, restore, and known-service allowlists together.
6. Prove local write/read with all networks disabled, then re-enable only the home LAN and prove bidirectional resume and visible conflict creation.
7. Commit AgentMemory CLI changes and Buzz integration changes separately.

### Task 5: Build the signed read-only Apple-input helper

**Repository:** `NavigatorRAN/buzz`

**Files:**

- Add: `desktop/apple-inputs/BuzzAppleInputs.xcodeproj/project.pbxproj`
- Add: `desktop/apple-inputs/Sources/main.swift`
- Add: `desktop/apple-inputs/Sources/Protocol.swift`
- Add: `desktop/apple-inputs/Sources/EventKitReader.swift`
- Add: `desktop/apple-inputs/Sources/NotesReader.swift`
- Add: `desktop/apple-inputs/Sources/FileReader.swift`
- Add: `desktop/apple-inputs/Tests/ProtocolTests.swift`
- Add: `desktop/apple-inputs/Tests/EventKitFixtureTests.swift`
- Add: `desktop/apple-inputs/Tests/NotesFixtureTests.swift`
- Add: `desktop/apple-inputs/Tests/FileReaderTests.swift`
- Add: `desktop/src-tauri/src/command_services/apple_inputs.rs`
- Add: `desktop/src/shared/api/tauriAppleInputs.ts`
- Add: `desktop/src-tauri/tauri.macos.conf.json`
- Modify: `desktop/src-tauri/Info.plist`
- Modify: `desktop/src-tauri/Entitlements.plist`
- Modify: `desktop/src-tauri/build.rs`
- Modify: `Justfile`
- Modify: `scripts/bundle-sidecars.sh`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/release.yml`
- Modify: `.github/workflows/signed-macos-canary.yml`

**Interfaces:**

- Fixed newline-delimited JSON request/response protocol with operations `permission_status`, `request_permission`, `read_calendar`, `read_reminders`, `read_notes`, and `read_files`.
- Every response includes source, permission state, observed time, bounded records, truncation state, and per-source error. The helper never mutates source applications or files.

**Steps:**

1. Create an Xcode command-line helper project with deterministic build settings and fixture-driven tests that do not request real privacy permissions.
2. Implement EventKit permission canaries for Calendar and Reminders using current macOS APIs; canaries report `not_determined` without prompting. Put permission requests behind the separate explicit operation.
3. Implement bounded read-only Calendar and Reminder queries with explicit date windows, selected calendar/list allowlists, recurring-event identifiers, deletion/staleness metadata, and no write-capable protocol operation.
4. Implement Notes through one fixed `NSAppleScript`/Apple Event program assembled only from escaped allowlisted folder identifiers. Renderer input can neither supply script text nor choose another application.
5. Implement bounded allowlisted file enumeration/read with canonical paths, device/inode checks, symlink rejection, file type/size/count limits, and stable source metadata.
6. Add Rust helper supervision using only the sibling bundled executable, cleared environment, fixed working directory, bounded stdout/stderr, timeouts, process teardown, strict exact-key JSON validation, and fail-soft source results.
7. Add Calendar/Reminders legacy and macOS 14+ usage descriptions, Apple Events usage description, calendar and automation entitlements, macOS-only external binary configuration, sidecar build/copy lists, and signed-canary entitlement verification.
8. Use `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer` for build/test. Prove the helper is executable inside `Buzz.app`; do not claim DMG success unless the DMG step itself completes.
9. Commit only Task 5 changes.

### Task 6: Admit local Memory/RAG tools and expose truthful service status

**Repository:** `NavigatorRAN/buzz`

**Files:**

- Add: `desktop/src-tauri/src/command_services/rag.rs`
- Add: `desktop/src-tauri/src/command_services/policy.rs`
- Modify: `desktop/src-tauri/src/managed_agents/runtime/lmstudio.rs`
- Modify: `desktop/src-tauri/src/managed_agents/runtime/tests/lmstudio.rs`
- Modify: `desktop/src/features/command-console/hooks/useCommandConsoleStatus.ts`
- Modify: `desktop/src/features/command-console/hooks/useCommandConsoleStatus.test.mjs`
- Modify: `desktop/src/features/command-console/ui/CommandSystemStatus.tsx`
- Modify: `desktop/src/features/command-console/ui/CommandSystemStatus.test.mjs`
- Modify: `desktop/src/features/command-console/domain/contracts.ts`
- Modify: `desktop/src/features/command-console/domain/contracts.test.mjs`
- Modify corresponding shared API wrappers and Tauri command registration

**Interfaces:**

- Produces cryptographically verified knowledge status, replication/conflict status, Apple permission/source status, and exact `LM_STUDIO_MCP_INTEGRATIONS` entries for local Memory/RAG tools.

**Steps:**

1. Write failing Rust tests proving only authenticated literal-loopback Memory/RAG services with exact expected server identities, active snapshot/node metadata, and tool catalog subsets can be admitted.
2. Recompute snapshot/revision/envelope hashes in the trusted Rust boundary; TypeScript exact-shape validation remains a display/persistence guard and is never treated as cryptographic proof.
3. Build the native MCP integration list only from catalog-owned policy after readiness succeeds. User/persona/global environment cannot add tools, change URLs, remove required authentication, or admit a LAN host.
4. Add exact read-only tool allowlists. Memory write tools are available only to explicit command-memory workflows; RAG is read-only. Conflict resolution and snapshot activation are never model-callable.
5. Replace hard-coded Phase 1 placeholders with live Memory, RAG, and Apple status sources. Render active snapshot, freshness, validation, replication cursor, conflict count, permissions, degraded sections, and actionable diagnostics without leaking content or credentials.
6. Reject retrieved prompt injection, stale/unknown snapshot references, missing citations, conflicted memory fields, and Apple/file records outside the configured allowlist before adviser context construction.
7. Run focused Tauri, Node, contract, runtime, and Command Console tests.
8. Commit only Task 6 changes.

### Task 7: Prove offline operation, recovery, and Phase 3 runbooks

**Repositories:** all three

**Files:**

- Add: `scripts/check-command-knowledge.sh`
- Add: `scripts/tests/check-command-knowledge-test.sh`
- Add: `docs/command-console/phase-3-knowledge-productivity.md`
- Modify: `docs/command-console/phase-2-local-agent-runtime.md`
- Modify: `Justfile`
- Modify each companion repository README/runbook as required

**Steps:**

1. Add deterministic fake-service tests for snapshot verification/activation, Memory replication/conflicts, MCP admission, Apple permission denial, stale data, malformed/oversized responses, and redacted errors.
2. Export a real signed RAG bundle without changing the active home collection; import it through staging on the Mac; verify schema/counts/models/checksums/signature/golden queries; and retain the previous snapshot.
3. Run approved navigation golden queries against home and Mac services and record source overlap, metadata-filter behavior, and declared quality thresholds without recording sensitive query or document content.
4. Disable internet and home LAN, restart the Mac stack, prove RAG search, local Memory read/write, Apple fail-soft inputs, and loopback native MCP readiness. Do not claim a complete Daily Command Brief until Phase 4.
5. Re-enable only home LAN, prove SSH pinning, resumable replication, duplicate delivery, tombstones, and a synthetic divergent-entity conflict. Restore the pre-test state.
6. Back up and restore local RAG, Memory, service configuration, signer pins, and Buzz state on a clean test profile. Credentials remain Keychain-backed and outside archives.
7. Run repository-specific full checks, Buzz `just ci`, Xcode helper tests, signed-app packaging, and whole-phase independent review. Fix every Critical and Important finding and rerun affected suites.
8. Record the verified endpoints, versions, signer fingerprints, test evidence, unresolved limitations, and deployment gotchas in Memory MCP with `agent="CODEX"`.
9. Push all three branches and update their stacked draft PRs only after their acceptance gates pass.

## Final verification

Run from each repository's activated environment:

```bash
# RAG-MCP
python3 -m pytest -q
python3 -m py_compile ingest/*.py retrieval/*.py rag_snapshot/*.py

# AgentMemory
cd MemoryMCPServer
.venv/bin/pytest
cd ..
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift test

# Buzz
source bin/activate-hermit
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer just apple-inputs-test
just check-command-knowledge
just desktop-check
just test-unit
just ci
```

Acceptance additionally requires:

- No direct agent or Tauri request is sent to `192.168.1.26`, `192.168.1.107`, `192.168.1.11`, or any other LAN/public model/MCP endpoint.
- The home service is reachable for synchronisation only through a pinned SSH tunnel terminating on literal loopback.
- The RAG mirror works with internet and home LAN disabled and reports the exact active signed snapshot.
- A failed RAG import leaves the previous snapshot active and recoverable.
- Local Memory writes succeed offline; resumed replication is idempotent; divergent revisions remain visible and excluded from unattended use.
- Apple permission canaries never prompt, permission denial is fail-soft, and no Apple/file write operation exists.
- Every factual adviser input carries source metadata, retrieval time, and snapshot/revision identity.
- The macOS `.app` contains executable signed local-runtime and Apple-input helper binaries.
- This phase remains advisory and does not claim Defence accreditation or permission for classified material.
