# Offline Adaptive Command Adviser Programme Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a fully disconnected Command Adviser on the 64 GB M5 Pro MacBook, with one queued multimodal model, a complete local RAG copy, continuous historical memory, autonomously evolving skills, and an optional later model-refinement path.

**Architecture:** Six independently gated phases extend Buzz's existing LM Studio provider, Command Brief scheduler, NIP-AE memory, skill discovery, and signed RAG snapshot validation. Model weights, retrieved knowledge, memory, and skills remain separate versioned components. Each phase produces a recoverable local capability and must pass its own real user journey before the next phase depends on it.

**Tech Stack:** Rust, Tauri 2, React 19, SQLite, Nostr/NIP-44/NIP-AE, Python 3.12, FastMCP, Qdrant, LM Studio, GGUF, shell canaries, optional Hugging Face Transformers/PEFT/TRL on DGX Spark.

## Global Constraints

- Implementation remains behind the active upstream v0.5.8 synchronisation unless the owner explicitly reprioritises it.
- The at-sea baseline is one MacBook Pro M5 Pro with 64 GB unified memory. A second Spark is not an inference dependency.
- All runtime services bind to loopback or another explicitly authorised local interface; no silent cloud fallback is allowed.
- RAG is authoritative for doctrine and reference evidence. Memory and model refinement may improve behaviour but may not replace citations.
- Historical memory and every skill version are append-only. Active views may supersede or deactivate records without deleting them.
- Credentials, authentication material, hidden chain-of-thought, and unbounded duplicate payloads are excluded from capture and training exports.
- Autonomous skill promotion changes agent procedure only; it does not grant authority to change security policy, credentials, model files, external systems, or release configuration.
- Every phase uses test-first changes, exact acceptance evidence, narrow commits, and a separate pull request for each affected repository.
- Before Buzz Git or quality commands, run `. ./bin/activate-hermit` in the same shell command.

---

## Delivery Map

| Order | Plan | Deliverable | Exit gate |
|---|---|---|---|
| 0 | Current upstream plan | Stable Buzz base | Active v0.5.8 sync is completed or owner reprioritises |
| 1 | `2026-08-09-gemma-local-model-qualification.md` | Accepted local multimodal runtime | 32K passes; 64K is measured; tool, image, restart, and queue canaries pass offline |
| 2 | `2026-08-09-mac-local-rag-snapshot.md` | Signed local corpus and retrieval service | Fixed semantic canary returns cited passage metadata with networking disabled |
| 3 | `2026-08-09-adaptive-command-memory.md` | Append-only experience and active recall | Capture, correction, supersession, private/shared scope, rebuild, and restart tests pass |
| 4 | `2026-08-09-autonomous-agent-skills.md` | Versioned self-improving skill registry | Candidate creation, inherited regression, promotion, rollback, and restart pass |
| 5 | `2026-08-09-disconnected-command-adviser-acceptance.md` | Integrated sea-going deployment | Interactive work, three-agent queue, overnight Brief, cold restart, and eight-hour soak pass offline |
| 6 | `2026-08-09-command-adviser-model-refinement.md` | Optional adapted model candidate | Candidate beats or matches baseline gates without weakening evidence or tool reliability |

## Task 1: Close or Explicitly Reprioritise the Current Sync Phase

**Files:**
- Read: `docs/superpowers/plans/2026-08-09-command-adviser-upstream-v0.5.8-sync.md`
- Read: `docs/command-console/ROADMAP.md`
- Modify only if the phase is actually closed: `docs/command-console/ROADMAP.md`

- [ ] Read the current sync plan and list every unchecked acceptance item.
- [ ] Run its required verification commands and preserve the evidence in that phase's PR.
- [ ] Either complete and merge the sync phase or obtain an explicit owner decision to start this programme before it.
- [ ] Record the dependency decision in the programme issue/PR; do not silently interleave architectural work into the sync branch.

## Task 2: Qualify the Gemma Runtime Before Building Around It

**Plan:** `docs/superpowers/plans/2026-08-09-gemma-local-model-qualification.md`

- [ ] Execute the model qualification plan against the exact downloaded `Gemma 4 26B-A4B-IT Q4_K_M` artefact.
- [ ] Admit 32K only after text, structured JSON, tool calling, image input, queueing, cancellation, and cold restart pass with networking disabled.
- [ ] Promote 64K or 128K independently; never infer a larger tier from the smaller-tier result.
- [ ] If Gemma fails, execute the same harness against Ministral 3 14B Instruct; use GPT-OSS-20B only as a text/tool control.
- [ ] Freeze the accepted model/runtime identity and hashes in the sea-going bundle manifest.

## Task 3: Make RAG Fully Local and Verifiably Current

**Plan:** `docs/superpowers/plans/2026-08-09-mac-local-rag-snapshot.md`

- [ ] Implement snapshot export and dense offline retrieval in the RAG MCP repository first.
- [ ] Prove the exported snapshot with a fixed collection-specific semantic evaluation before installing it into Buzz.
- [ ] Commission the loopback endpoint and signed manifest in Buzz using the existing protected configuration path.
- [ ] Test interrupted refresh and rollback so a failed update cannot replace the last known-good corpus.

## Task 4: Add Continuous Historical Memory and Selective Active Recall

**Plan:** `docs/superpowers/plans/2026-08-09-adaptive-command-memory.md`

- [ ] Add idempotent derived indexing and active-view queries to Memory MCP.
- [ ] Add encrypted append-only experience records and a durable projection outbox to Buzz.
- [ ] Capture every bounded turn outcome, correction, source identity, skill identity, and validation result without capturing secrets or hidden reasoning.
- [ ] Assemble recent context plus selectively recalled active memories; keep superseded records in history and out of the default prompt.
- [ ] Rebuild the derived Memory MCP index from authoritative Buzz events and prove byte-equivalent active results.

## Task 5: Enable Autonomous, Versioned Skill Evolution

**Plan:** `docs/superpowers/plans/2026-08-09-autonomous-agent-skills.md`

- [ ] Add immutable encrypted skill-version events and one addressable active pointer per skill.
- [ ] Materialise active skills into a disposable managed directory while keeping signed events authoritative.
- [ ] Generate candidates from repeated verified outcomes and corrections, preserving parent lineage and inherited tests.
- [ ] Promote only between turns after deterministic validation and replay evaluation; automatically retain and restore the last known-good version.
- [ ] Prove rollback, restart, corrupt projection recovery, specialist-private scope, and Command-Team-shared scope.

## Task 6: Prove the Complete Disconnected Product

**Plan:** `docs/superpowers/plans/2026-08-09-disconnected-command-adviser-acceptance.md`

- [ ] Build and verify a checksum-addressed sea-going bundle containing the app, model, companion vision files, embedding model, RAG snapshot, local service images/binaries, protected configuration backup, and recovery material.
- [ ] Disable all external networking and run one-to-three agent collaboration through the capacity-one generation queue.
- [ ] Complete an overnight Daily Command Brief with durable specialist checkpoints and cited local evidence.
- [ ] Interrupt the app, LM Studio, RAG service, and Mac at defined checkpoints; verify deterministic recovery without duplicate publication.
- [ ] Pass an eight-hour offline soak with no cloud attempt, unbounded disk growth, stuck queue item, or silent component degradation.

## Task 7: Open the Optional Model-Refinement Gate Only with Enough Evidence

**Plan:** `docs/superpowers/plans/2026-08-09-command-adviser-model-refinement.md`

- [ ] Keep this phase closed until the archive contains at least 1,000 accepted training examples, 200 validation examples, and 200 untouched evaluation examples after semantic deduplication by task and source identity.
- [ ] Export only verified, redacted behaviour examples; keep doctrine and current facts in RAG.
- [ ] Train LoRA/QLoRA candidates on the available DGX Spark and preserve the base model, adapter, dataset manifest, configuration, and hashes.
- [ ] Re-run the complete Mac-local model and disconnected product gates before considering promotion.
- [ ] Reject any candidate that weakens citation discipline, tool-call validity, multimodal handling, rollback behaviour, or context stability even if style scores improve.

## Task 8: Close the Programme with Recoverable Evidence

**Files:**
- Modify: `docs/command-console/ROADMAP.md`
- Create: `docs/command-console/offline-command-adviser-operations.md`
- Create: `docs/command-console/offline-command-adviser-acceptance-record.md`

- [ ] Link all merged phase PRs, exact commits, signed manifests, model hashes, and acceptance reports from the roadmap.
- [ ] Document pre-deployment refresh, offline readiness, daily operation, backup, cold restart, RAG rollback, skill rollback, and post-deployment reconciliation.
- [ ] Have a second agent or owner execute the operations guide from a clean account without unpublished knowledge.
- [ ] Run `just ci` in Buzz and the full documented gates in Memory MCP and RAG MCP.
- [ ] Commit the final evidence and close the programme only when all required phases are green.
