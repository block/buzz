# Implementation Plan — Triggerless Write Contract Foundation (2026-09-17)

## Status

Implemented on this branch for the first approved foundation slice.

## Objective

Establish application-owned transaction protocols for community fencing and
replica-floor ordering while preserving current trigger/function enforcement.

## Scope in this change

1. **Community-fence foundations**
   - Added `Db::begin_community_write_transaction(community)`.
   - Added `Db::begin_community_write_transaction_batch(communities)` with
     stable UUID lock ordering and empty-input fail-closed behavior.
   - Added tests proving:
     - shared writer lock blocks deletion exclusive lock,
     - post-fence fresh admission is rejected,
     - batch ordering is deterministic and avoids opposite-order lock hazards,
     - empty batch input is rejected.

2. **Replica-floor foundations (no second publication concept)**
   - Added `REPLICA_FLOOR_LOCK_KEY` and retained the existing
     `probe_once/sample_writer` heartbeat publication model as the sole
     token/fence-wall publication mechanism.
   - Added `Db::begin_replica_floor_locked_event_write_transaction()`:
     shared replica-floor transaction lock only (no caller timestamp preflight),
     while retaining the existing commit-time trigger/GUC backstop.
   - Updated the existing probe handshake to acquire the exclusive floor lock
     **before** sampling `S`, activity scan, and heartbeat token commit.
   - Added tests proving:
     - compliant shared writer blocks probe progress,
     - probe resumes and records normal token/fence-wall after writer release,
     - `sample_writer` cannot sample `S` until a held shared lock releases
       (timestamp marker proof with bounded synchronization).

## Explicitly not in this change

- Trigger/function removal.
- FK-based replacement for advisory lock ordering.
- Direct-owner SQL execution paths outside reviewed writer protocols.
- Any second replica-floor publication API outside `probe_once`.
- Persisting an active floor cutoff in schema.

## Validation performed

- Targeted RED→GREEN for each added behavior.
- Aggregated sweeps:
  - `community_fence_` tests,
  - `replica_floor_` tests,
  - `probe_` tests.
