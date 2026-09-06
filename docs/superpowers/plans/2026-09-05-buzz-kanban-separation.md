# Buzz Kanban Separation Implementation Plan

**Goal:** Give Buzz a dedicated self-hosted Kanban AI project, preserve five
existing Buzz-native cards, recreate VOICE 0–8, and document safe operation.

**Delivery mode:** STANDARD migration. The data change is narrow but requires a
backup, atomicity, and exact post-migration evidence.

## Scope Lock

### MUST NOW

- Create `Buzz` with ID `28fe656f-de10-4df9-9f49-cef6cbfa22c0`.
- Move the five approved Buzz-native cards by changing only `project_id`.
- Create VOICE 0–8 with the approved IDs, status split, dependencies, and
  architecture constraints.
- Preserve every `[DISCOVERY][BUZZ→LINZA]` card in `linza`.
- Verify a pre-write online backup, transactional postconditions, live database,
  application-facing MCP reads, and a secret-free receipt.
- Add an operator runbook and link it from `AGENTS.md`.

### NOT NOW

- Realtime voice implementation.
- A new SFU, gateway, plugin framework, state store, dependency, or deployment.
- Changes to existing card content, status, comments, or timestamps.
- General-purpose Kanban migration tooling.

### Done When

- Buzz has 14 cards: 5 moved and 9 VOICE.
- VOICE 0–1 are `in-progress`; VOICE 2–8 are `todo`.
- Linza has lost only the five approved IDs and retains the exact preflight set
  of 11 Buzz-inspired discovery task IDs.
- Live and backup `PRAGMA integrity_check` return `ok`.
- Preserved task fields and comments match the backup exactly.
- MCP `list_projects` and `get_board` agree with SQLite.
- The committed runbook contains the exact non-secret receipt.

## Chosen Design

Use one reviewed, temporary Node ESM script inside the existing container. Load
the already installed `better-sqlite3` from `/app/package.json`; do not add a
dependency. The container root filesystem is read-only, so copy the script to
writable `/data`, execute it once, and remove it after verification.

The script uses one named validation contract rather than scattered validation
fences. It compares actual and expected values for:

- target project and epic absence;
- exact source IDs and Linza ownership;
- local owner/collaborator identities;
- the complete task-field inventory excluding only `project_id`;
- source task fields, comments, and unrelated Linza task IDs;
- VOICE IDs, titles, statuses, and total board counts;
- backup/pre-write snapshot equality and both integrity checks.

Create the online backup before the transaction and verify it before any live
write. Revalidate the same snapshot inside the transaction, apply project
creation, collaborator creation, five moves, and nine inserts, then assert all
postconditions before commit. Any failed assertion rolls the transaction back.

The authenticated MCP endpoint is on the SSH host loopback, not in the container
network namespace. Run final MCP reads on the host and read the mounted bearer
key into process memory without printing or persisting it.

## Alternatives

| Option | Benefit | Cost/risk | Decision |
|---|---|---|---|
| MCP-only recreation | Supported public write path | Cannot preserve task or epic IDs; comments/history would need recreation | Rejected |
| One-time validated SQLite transaction | Preserves IDs and content; atomic and reversible from backup | Exceptional privileged operation | Chosen |
| Permanent migration framework | Reusable | No second use case; adds maintenance and attack surface | Not now |
| Do nothing | No migration risk | Buzz work remains mixed with Linza and roadmap remains absent | Rejected |

## DRY / KISS / YAGNI Audit

- **DRY — OK:** one canonical validation contract owns identity, field
  completeness, and state comparisons.
- **KISS — SIMPLE ENOUGH:** one temporary script uses the existing database
  library; no production code or service changes.
- **YAGNI — NO SPECULATION:** no reusable migration framework or voice runtime
  foundation is introduced.
- **Deletion test:** backup, transaction, field/comment comparison, and MCP read
  each close a named integrity or application-visibility risk. Everything else
  is excluded.

## Execution

### Task 1 — Fail-closed dry run

- [x] Inspect live schema, owner identities, source cards, comments, counts,
  foreign-key mode, and integrity with a read-only handle.
- [x] Confirm one `linza`, no `Buzz`, absent epic ID, exactly five approved
  source IDs in Linza, and 11 discovery cards.
- [x] Validate the temporary script syntax in and outside the container.
- [x] Run dry-run and confirm planned project ID, five sources, and nine VOICE
  IDs without changing the database.

**Evidence:** 433 Linza tasks before migration; source count 5; discovery count
11; initial live integrity `ok`.

### Task 2 — Atomic live migration

- [x] Create and verify the online backup before opening the write transaction;
  record its size and SHA-256 in the permanent receipt.
- [x] Revalidate exact pre-write state inside the transaction.
- [x] Insert the Buzz project and owner collaborator, move five cards, and insert
  VOICE 0–8 in one transaction.
- [x] Verify all postconditions before transaction commit.

**Evidence:** migration applied at `2026-09-06T07:12:21.062Z`; backup basename
`kanban-before-buzz-20260906T071221.062Z.sqlite`.

### Task 3 — Independent data and application verification

- [x] Open live and backup databases read-only and rerun integrity checks.
- [x] Compare every moved field except `project_id`, plus all source comments.
- [x] Confirm unrelated Linza task IDs and the exact 11 discovery task IDs
  remain.
- [x] Call MCP `list_projects` and `get_board` through host loopback.
- [x] Remove the temporary migration script after verification.

**Evidence:** live/backup integrity `ok`; moved fields/comments equal; Buzz has
14 tasks; Linza has 428; MCP reports 10 `todo`, 2 `in-progress`, and 2 previously
completed moved tasks.

### Task 4 — Repository runbook

- [x] Add `docs/kanban-ai.md` with routing, access, backup, failure, and roadmap
  boundaries.
- [x] Record the exact non-secret migration receipt.
- [x] Link the runbook from `AGENTS.md`.
- [x] Run identifier, secret-pattern, Markdown whitespace, and diff checks.
- [x] Prepare the DCO-signed commit after independent review.

## Validation and Rollback

Focused checks:

```text
SQLite: exact IDs/counts/statuses, preserved fields/comments, live+backup integrity
MCP: list_projects and get_board(include_comments=false)
Docs: required identifiers, receipt basename, secret-pattern scan, git diff --check
```

Rollback is the verified backup in `/data/backups`. Restoring it discards every
board write after the receipt timestamp. Restore only if later verification
reveals a migration defect: stop the application to prevent concurrent-write
corruption, inventory and explicitly accept or export later changes, verify the
recorded SHA-256, replace the database, and rerun SQLite and MCP checks.

The exact receipt and permanent operating rules live in
[`docs/kanban-ai.md`](../../kanban-ai.md).
