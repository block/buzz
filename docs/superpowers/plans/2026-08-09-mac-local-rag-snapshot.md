# Mac-Local RAG Snapshot Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Install a complete, signed, refreshable RAG corpus on the MacBook that returns citation-grade evidence with every external network path disabled.

**Architecture:** RAG MCP gains an explicit dense offline retrieval mode using the existing `bge-m3` dense vector and a Mac-local embedding endpoint. A producer exports Qdrant plus canonical manifest/catalog metadata; a staged restorer verifies hashes and atomically activates a snapshot. Buzz commissions the loopback MCP endpoint and reuses its existing signed-snapshot and evidence validation.

**Tech Stack:** Python 3.12, FastMCP, Qdrant, Ollama-compatible embeddings, Docker Compose ARM64, Rust/Tauri, Ed25519 signatures, shell canaries.

## Global Constraints

- This plan spans `/Users/matthewwarren/Documents/RAG MCP` and Buzz; use one branch and PR per repository.
- The Mac is retrieval-only. Corpus ingest and document parsing remain home-side workflows.
- Offline mode is explicitly `dense`; it never silently pretends sparse retrieval or reranking ran.
- Preserve document, collection, page, section, `point_id`, chunk hash, snapshot ID, and retrieval time for every result.
- An incomplete or invalid refresh must leave the last known-good snapshot active.
- The semantic canary, not `/health` or a collection list, is the acceptance gate.

---

## Task 1: Add an Explicit Dense Offline Retrieval Mode in RAG MCP

**Repository:** `/Users/matthewwarren/Documents/RAG MCP`

**Files:**
- Modify: `retrieval/config.py`
- Modify: `retrieval/retrieve.py`
- Create: `tests/test_offline_dense_retrieval.py`

- [ ] Write failing tests for `RAG_RETRIEVAL_MODE=dense`, invalid mode rejection, dense-only Qdrant request shape, and absence of sparse/reranker calls.
- [ ] Add `retrieval_mode: Literal["hybrid", "dense"]` to settings with `hybrid` as the existing deployment default.
- [ ] Extract `_retrieve_dense(query, collection, limit)` so it embeds once and queries the named `dense` vector directly.
- [ ] Keep the existing hybrid RRF/rerank path unchanged when mode is `hybrid`.
- [ ] Include `retrieval_mode`, embedding model identity, and snapshot ID in response diagnostics without weakening evidence metadata.
- [ ] Run `pytest -q tests/test_offline_dense_retrieval.py` and the existing retrieval tests.
- [ ] Commit: `feat(retrieval): add explicit dense offline mode`.

## Task 2: Export a Canonical Signed Snapshot

**Repository:** `/Users/matthewwarren/Documents/RAG MCP`

**Files:**
- Create: `rag_snapshot/__init__.py`
- Create: `rag_snapshot/manifest.py`
- Create: `scripts/export_offline_snapshot.py`
- Create: `tests/test_snapshot_export.py`

- [ ] Write failing tests for deterministic manifest serialization, collection inventory, point/vector counts, payload schema, chunk identity hashes, file hashes, and a changed corpus producing a changed snapshot ID.
- [ ] Define `SnapshotManifestV1` with schema version, snapshot ID, created time, source deployment, Qdrant version, embedding model/hash, vector names and dimensions, retrieval config hash, collection entries, catalogue hash, artefact hashes, and signing key ID.
- [ ] Compute `snapshot_id` from canonical JSON excluding the signature; never use a timestamp as identity.
- [ ] Export Qdrant's native collection snapshots and the canonical catalogue to a staging directory.
- [ ] Sign `manifest.json` and emit the existing Buzz-compatible `manifest.pub` and `manifest.sig` files.
- [ ] Refuse export when any collection has an unexpected vector dimension, missing required payload key, or incomplete native snapshot.
- [ ] Run `pytest -q tests/test_snapshot_export.py`.
- [ ] Commit: `feat(snapshot): export signed offline rag bundle`.

## Task 3: Restore and Activate Atomically on the Mac

**Repository:** `/Users/matthewwarren/Documents/RAG MCP`

**Files:**
- Create: `scripts/restore_offline_snapshot.py`
- Create: `rag_snapshot/restore.py`
- Create: `tests/test_snapshot_restore.py`

- [ ] Write failing tests for bad signature, hash mismatch, missing collection snapshot, wrong vector dimension, interrupted restore, failed semantic canary, and successful activation.
- [ ] Restore into `snapshots/staging/<snapshot_id>` and a temporary Qdrant collection namespace.
- [ ] Verify signature, all hashes, collection counts, payload schema, embedding identity, and retrieval configuration before activation.
- [ ] Run the fixed semantic canary against the staged namespace before changing the active pointer.
- [ ] Atomically replace only `snapshots/active.json`; retain the previous snapshot and Qdrant namespace for rollback.
- [ ] Make reapplying the same snapshot idempotent and reject an older snapshot unless `--allow-rollback <snapshot_id>` names it explicitly.
- [ ] Run `pytest -q tests/test_snapshot_restore.py`.
- [ ] Commit: `feat(snapshot): restore and activate rag atomically`.

## Task 4: Package the Mac-Local Retrieval Stack

**Repository:** `/Users/matthewwarren/Documents/RAG MCP`

**Files:**
- Modify: `compose/docker-compose.yml`
- Create: `compose/docker-compose.offline-mac.yml`
- Create: `config/offline-mac.env.example`
- Create: `scripts/check_offline_runtime.py`
- Create: `tests/test_offline_runtime_config.py`

- [ ] Write a failing configuration test that rejects non-loopback binds, remote Qdrant URLs, remote embedding URLs, hybrid mode, enabled ingest workers, and missing active snapshot.
- [ ] Define an ARM64-compatible Compose override with only Qdrant and retrieval/MCP services; do not include Docling, sparse, reranker, or ingest services.
- [ ] Point dense embedding to a literal loopback Ollama-compatible endpoint with exact `bge-m3` model identity and hash in the manifest.
- [ ] Use read-only snapshot mounts where possible and a separate writable Qdrant volume.
- [ ] Add health checks that validate active snapshot identity and a real embedding request, not only open ports.
- [ ] Run `pytest -q tests/test_offline_runtime_config.py` and `docker compose -f compose/docker-compose.yml -f compose/docker-compose.offline-mac.yml config`.
- [ ] Commit: `feat(runtime): package mac offline rag services`.

## Task 5: Define and Pass the Golden Retrieval Evaluation

**Repository:** `/Users/matthewwarren/Documents/RAG MCP`

**Files:**
- Modify: `scripts/evaluate_rag.py`
- Create: `evaluation/offline-command-adviser.jsonl`
- Create: `tests/test_offline_evaluation.py`

- [ ] Create a versioned evaluation set with exact collection, query, expected document/section, acceptable point IDs, and forbidden unsupported conclusions.
- [ ] Include doctrine lookup, ambiguous terminology, cross-collection isolation, no-answer, and stale-document discrimination cases.
- [ ] Add metrics for recall at 5, metadata completeness, citation identity validity, no-answer precision, and p95 latency.
- [ ] Set the initial gate to 100% metadata completeness and citation identity validity, no unsupported answer payloads, and a reviewed per-query recall threshold recorded in the evaluation file.
- [ ] Run the evaluation against home hybrid retrieval as a reference and Mac dense retrieval as the candidate; store both reports.
- [ ] Fail the phase if dense retrieval does not meet the fixed gate; improve snapshot/retrieval or explicitly design a local sparse/reranker deployment rather than relaxing the corpus silently.
- [ ] Commit: `test(retrieval): add offline command adviser evaluation`.

## Task 6: Commission the Snapshot in Buzz

**Repository:** `/Users/matthewwarren/Documents/Buzz AI`

**Files:**
- Create: `scripts/commission-command-rag.sh`
- Create: `scripts/tests/commission-command-rag-test.sh`
- Modify: `scripts/check-command-knowledge.sh`
- Modify: `desktop/src-tauri/src/command_services/rag.rs`
- Modify: `desktop/src-tauri/src/command_services/rag/evidence.rs`

- [ ] Write failing shell tests for invalid signature, snapshot-ID mismatch, remote MCP endpoint, missing Keychain reference, failed semantic canary, and successful commissioning.
- [ ] Extend the Rust snapshot validator only for newly required manifest fields; retain compatibility with the signed canonical files already expected by Buzz.
- [ ] Install protected `command-rag.json` through the existing protected configuration path and store credentials in Keychain, never in the manifest.
- [ ] Require literal loopback for disconnected mode and expose the active snapshot ID to evidence records.
- [ ] Update `check-command-knowledge.sh` to require a collection-specific semantic result with document, section/page, `point_id`, and snapshot ID.
- [ ] Run the shell tests and targeted Tauri RAG tests.
- [ ] Commit: `feat(command-rag): commission local signed snapshots`.

## Task 7: Prove Refresh, Rollback, and Disconnected Retrieval

**Files:**
- Modify: `docs/command-console/phase-3-knowledge-productivity.md`
- Create: `docs/command-console/offline-rag-operations.md`

- [ ] Export snapshot A, install it, and pass the semantic canary with external networking disabled.
- [ ] Interrupt snapshot B during copy, Qdrant restore, and pre-activation evaluation; verify A remains active each time.
- [ ] Install valid B, verify its signed snapshot ID appears in Command Adviser evidence, then explicitly roll back to A.
- [ ] Ask a real Command Adviser question whose answer requires the local corpus and verify quoted evidence maps back to the returned document/section/point ID.
- [ ] Restart Qdrant, RAG MCP, and Buzz; repeat the same semantic query and verify the active identity is unchanged.
- [ ] Run `. ./bin/activate-hermit && just ci` in Buzz and the full RAG MCP test/evaluation suite.
- [ ] Open separate RAG MCP and Buzz PRs, linking producer evidence before consumer commissioning.
