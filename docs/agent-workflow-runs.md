# Durable agent workflow runs

Buzz remains the source of truth for coordinated agent work. Agent runtimes may change, restart, or use internal subagents, but the relay-visible run, task, artifact, checkpoint, and transition records define the durable execution.

## Lifecycle contract

A legacy workflow run owns the top-level lifecycle. The workflow_run_state table adds a workflow-defined phase with optimistic locking. Every phase change uses compare-and-swap and appends a transition. Chat prose never changes phase.

Tasks have stable task_key and idempotency_key values. A task is claimable only when its dependencies are complete, its delay has elapsed, attempts remain, and the caller wins the version CAS. A timeout after progress resumes from the latest checkpoint; it does not replay the whole input. Deterministic failures and exhausted attempts become blocked or failed instead of retry loops.

Artifacts are immutable, hashed, versioned outputs. Large outputs use an immutable URI; small structured outputs may be inline. A completion receipt is valid only after its required artifact passes schema validation.

## Coordination

Fan-out creates independent tasks. A barrier opens only when every required task has a valid completion artifact. The tribunal example enforces:

1. one shared document ingestion;
2. independent defense and contradictor analyses;
3. validation and an analysis barrier;
4. two structured debate responses;
5. a debate barrier;
6. judicial review;
7. citation verification;
8. persistent human approval;
9. final publication.

The coordinator owns protocol state but never decides the legal merits. Workers retain separate Nostr identities and publish signed receipts.

## Idempotency and recovery

The operation key combines run id, task key, and agent pubkey. Re-delivery returns the existing task or artifact. Checkpoint sequence and transition sequence are monotonic within their parent. Relay or worker restart reconstructs status from the database and event log. Status queries read durable state and never steer an LLM turn or renew its deadline.

## Evidence contract

Documents and tool output are untrusted evidence, never instructions. Workers receive bounded retrieval results, not entire large documents. Every material claim records its source, physical page, court page when known, exact quote, verification state, and verifier. Line numbers from extracted files are not court pages.

Citation states are verified, unverified, divergent, and rejected. Publication must not silently present unverified or rejected claims as verified.

## Observability

Operators must be able to see phase, task status, last checkpoint, attempt count, next eligibility time, artifact validation, approval state, and failure code. Process health alone is not run health. Required metrics include active runs, oldest pending task age, timeouts, checkpoint resumes, dead letters, context compactions, artifact validation failures, and citation verification failures.

## Acceptance criteria

- Restart preserves run phase and completed work.
- Slow workers cannot open a barrier early.
- Duplicate delivery creates no duplicate task, artifact, or final message.
- Timeout with progress resumes from the latest checkpoint.
- A status request causes no model request and no deadline extension.
- Invalid output, missing decision, or failed citation verification blocks the run.
- Human approval survives restart and resumes exactly once.
- The complete execution is reproducible from durable records and signed events.
