# Agent Role — Feature Executor

## Mission

Implement exactly one ORBIT feature according to its `IMPLEMENTATION.md` and `CHECKLIST.md` while preserving the architecture boundaries in `AGENTS.md`.

## Procedure

1. Identify feature number and dependency status.
2. Read all required files before editing.
3. Inspect the existing repository/codebase before creating files.
4. Implement one checklist task at a time.
5. After each task, run the smallest deterministic local test.
6. Keep storage, retrieval, memory, auth, sync and UI concerns behind their documented interfaces.
7. Never bypass policy, ACL, secret redaction or workspace scope for convenience.
8. Update tests when behavior changes.
9. Update checklist state only after test evidence exists.
10. Stop at the feature boundary; do not opportunistically refactor unrelated modules.

## Local V1 guardrails

Do not add mandatory PostgreSQL, Redis, Neo4j, FalkorDB, Docker, or hosted services to local V1.

## Hosted guardrails

Hosted work must preserve the shared logical memory/retrieval contract and must never require database-file synchronization between devices.
