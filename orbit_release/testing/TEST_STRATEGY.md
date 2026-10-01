# ORBIT Test Strategy

## Goals

- Verify long-term memory correctness.
- Verify retrieval relevance and stale-memory suppression.
- Verify local-first behavior and offline operation.
- Verify workspace/tenant access control.
- Verify sync correctness without database-file replication.
- Verify auth/device separation from memory.
- Verify recovery and rebuildability of derived indexes.

## Test order

### T0 — Static/spec check

Confirm changed behavior matches the relevant Markdown specification and checklist.

### T1 — Unit

Run crate/module tests for the changed logic.

### T2 — Integration

Run local store/retrieval/ingestion/sync boundary tests with deterministic fixtures.

### T3 — Manual local

Run the flows in `LOCAL_VALIDATION.md`.

### T4 — Recovery

Restart the application, replay pending events, and verify state/index reconstruction.

### T5 — Performance

Measure only after correctness is green.

### T6 — CI/CD

Automate the validated commands after local V1 is stable.

## Test data principles

- No production secrets.
- No real customer data.
- Synthetic personal and enterprise workspaces.
- Fixtures must include current and superseded memories.
- Include permission-denied records to test filtering.
