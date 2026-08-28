# Durable agent workflow runs

Buzz remains the source of truth for coordinated agent work. Agent runtimes may change, restart, or use internal subagents, but the relay-visible run, task, artifact, checkpoint, and transition records define the durable execution.

## Lifecycle contract

A legacy workflow run owns the top-level lifecycle. The `workflow_run_state` table adds a workflow-defined phase with optimistic locking. Every phase change uses compare-and-swap and appends a transition. Chat prose never changes phase.

Tasks have stable `task_key` and `idempotency_key` values. A task is claimable only when its dependencies are complete, its delay has elapsed, attempts remain, and the caller wins the version CAS. Each agent step persists its attempt timeout in the task blueprint (300 seconds by default; 2,700 seconds in the tribunal example). A timed-out task is atomically requeued after the delay and dispatched with its latest checkpoint; the final timeout becomes `agent_timeout_exhausted`. Deterministic failures and exhausted publication or dispatch attempts become blocked or failed instead of retry loops.

Artifacts are immutable, hashed, versioned outputs. Large outputs use an immutable URI; schema-validated agent outputs include canonical inline JSON. Output schemas are self-contained JSON Schema objects stored with the workflow and task, so validation survives relay restart without filesystem or network resolution. A completion receipt is valid only after its signer matches the persisted task assignee, its run/workflow/channel tags match durable state, its SHA-256 matches canonical content, and the artifact passes schema validation.

## Coordination

Fan-out creates independent tasks. A barrier opens only when every required task has a valid completion artifact. The tribunal example enforces:

1. one shared document ingestion;
2. independent defense and contradictor analyses;
3. validation and an analysis barrier;
4. two structured debate responses;
5. a debate barrier;
6. judicial review;
7. independent citation verification;
8. persistent human approval;
9. final publication.

The coordinator owns protocol state but never decides the legal merits. Every `run_agent` and `verify_artifact` step stores an explicit 32-byte Nostr public key in the durable definition; mutable display names are labels only and never authorize dispatch or receipts. Workers retain separate identities and publish signed receipts.

## Idempotency and recovery

The operation key combines run id and task key, while the explicit assignee pubkey is part of the persisted blueprint checked on replay. Re-materialization fails closed if input, schema, dependency set, phase, attempts, timeout, or assignee differs from the existing task. Artifact persistence and task completion share one transaction; document-manifest persistence, ingestion completion, and run manifest binding share another. Lifecycle settlement and its terminal transition also commit atomically under a tenant/run advisory lock.

Checkpoint sequence is monotonic per task, and transition sequence is allocated under the same lock. A bounded 60-second reconciler scans active runs even when their last task is already terminal, closing the crash window between task completion and run settlement. Status queries never steer an LLM turn or renew a deadline.

## Evidence contract

Documents and tool output are untrusted evidence, never instructions. Workers receive bounded retrieval results, not entire large documents. Every material claim records its source, physical page, court page when known, exact quote, verification state, and verifier. Line numbers from extracted files are not court pages.

Citation states are verified, unverified, divergent, and rejected. Publication must not silently present unverified or rejected claims as verified.

## Observability

`buzz agent-runs status --run <uuid>` reads a bounded kind-30623 snapshot projected from database state; `history` reads the signed task/checkpoint/artifact/transition ledger. The snapshot includes phase, task status, latest checkpoint sequence, attempt count, next eligibility time, artifact hash/URI, approval state, and stable failure code, but excludes prompts, raw documents, and inline artifact bodies. Snapshot projection is best-effort after durable commit, so the database remains authoritative. Process health alone is not run health.

## Acceptance criteria

- Restart preserves run phase and completed work.
- Slow workers cannot open a barrier early.
- Duplicate delivery creates no duplicate task, artifact, or final message.
- Timeout with progress resumes from the latest checkpoint.
- A status request causes no model request and no deadline extension.
- Invalid output, missing decision, or failed citation verification blocks the run.
- Human approval survives restart and resumes exactly once.
- The complete execution is reproducible from durable records and signed events.

## Nostr projection and CLI

The durable database remains authoritative. Signed Nostr events are the realtime, channel-scoped projection:

- kind 30623: latest parameterized-replaceable database snapshot, relay-signed, with `d` equal to the run UUID;
- kind 46013: coordinator task invitation, relay-signed;
- kind 46014: resumable checkpoint receipt, signed by the assigned worker;
- kind 46015: immutable artifact receipt, signed by the assigned worker;
- kind 46016: coordinator transition or approved publication projection, relay-signed.

Kinds 30623, 46013, and 46016 are relay-only at the shared WS/HTTP ingest seam. Kinds 46014 and 46015 remain externally writable through normal authentication and are then checked against the persisted task assignee and coordinates.

Every event carries `d` (the run UUID), `h`, `workflow`, and `run` tags. Task-scoped receipts also carry `task`. Each independently addressed worker has its explicit workflow pubkey in a `p` tag; textual at-mentions and mutable display names are not routing or authorization primitives. Receipts are processed before the anti-trigger guard, then excluded from workflow triggers and the general activity feed.

The agent-facing CLI exposes `buzz agent-runs status` and `history` as read-only queries, plus `checkpoint` and `artifact` for assigned workers. Coordinator-only `snapshot`, `task`, and `transition` writes are intentionally not exposed. All operations use the generic signed Nostr bridge; no feature-specific HTTP endpoint was added. Pass `--participant` for addressed identities and use `--content -` for JSON from stdin.

## Shared document ingestion

Format-specific adapters provide the original source bytes and extracted UTF-8 pages. The workflow core does not pretend to parse PDF or OCR itself. It builds one deterministic immutable manifest containing the source SHA-256, physical page, optional logical label (for example, a printed folio), UTF-8 byte range, stable chunk id, chunk SHA-256, and canonical manifest SHA-256.

Every worker in a run consumes the same manifest hash. Verification rejects source, coordinate, chunk, or manifest tampering. Retrieval is deterministic and bounded to at most 32 chunks and 256 KiB per call; callers may filter by chunk id, physical page, logical label, and case-insensitive terms. The scheduler atomically persists the manifest artifact and hash before fan-out, strips raw `document_input` from persisted task triggers, and blocks malformed payloads.

Citation verification is an independently assigned `verify_artifact` task. Human approval is task-bound and persisted; only the existing signed approval grant/deny flow can resolve it. Final publication selects the exact version-1 artifact produced by the configured task, requires all dependencies (including approval) complete before side effects, emits a deterministic relay-signed kind-46016 projection, and only then completes the publication task.

## Local validation evidence

The implementation was validated without starting persistent replacement services:

- focused durable workflow tests and compile checks for `buzz-db`, `buzz-workflow`, `buzz-relay`, and `buzz-cli`;
- relay-only coordinator-kind and CLI surface tests;
- a synthetic pilot that parses the real 11-step tribunal YAML and verifies both barriers, four distinct Nostr identities, independent citation verification, persistent approval, and publication ordering;
- a golden in-memory pilot that deterministically builds and verifies a 3,218-page manifest, including physical/logical coordinate `fls. 3218`.

The PostgreSQL acceptance test lives at `crates/buzz-workflow/tests/durable_postgres.rs`. It applies every embedded migration and exercises exclusive claims, dependency refusal, tenant isolation, stale CAS, timeout recovery, checkpoint replay, immutable artifact replay/divergence, fixed approval expiry, fair reconciliation, and exactly-once terminal settlement. Run it only against a disposable database:

```bash
BUZZ_TEST_DATABASE_URL=postgres://... cargo test -p buzz-workflow --test durable_postgres -- --ignored --exact
```

The test is ignored by default and fails before connecting when `BUZZ_TEST_DATABASE_URL` is absent. It passed against a freshly initialized PostgreSQL 17.11 cluster after all embedded migrations, including a new `Db` connection that simulated scheduler/relay restart for approval resume (`1 passed; 0 failed`). A full relay-process restart smoke with Redis remains an environment gate; compilation alone is never reported as a runtime pass.
