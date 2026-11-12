# Prepare and approve a synthetic user-deletion request

For read-only discovery of actual memberships from a hex key or npub, see
[Preview a user's current-community inventory](operator-user-deletion-inventory.md).

Submit saves a request and its inventory together. By default (`--pubkey demo`),
this is community B (participant), then community C (former participant).
Use `--pubkey <hex-or-npub>` to discover current communities instead. The request
stores the normalized hex key as its target; the inventory contains only
community IDs, labels and relationships. `--target demo:<label>` still works for
separate demo runs and cannot be combined with `--pubkey`.

Approve saves authorization without creating stages. The `run` command initializes
execution through `get_next_stage`, dispatches dummy handlers, and asks the
transition function for the next saved row. No user or community data is deleted.
See [the inventory walkthrough](operator-user-deletion-inventory.md) for both
complete paths.

## Prepare a local database

From the repository root, activate Hermit and start a separate PostgreSQL
container. These are disposable local credentials.

```bash
. ./bin/activate-hermit
docker run --detach --name buzz-user-deletion-intake \
  --publish 127.0.0.1:55433:5432 \
  --env POSTGRES_USER=buzz \
  --env POSTGRES_PASSWORD=buzz-local-demo \
  --env POSTGRES_DB=buzz postgres:17-alpine
export DATABASE_URL=postgres://buzz:buzz-local-demo@127.0.0.1:55433/buzz
cargo build -p buzz-admin
# Wait for "accepting connections" before migrating.
docker exec buzz-user-deletion-intake pg_isready -U buzz -d buzz
./target/debug/buzz-admin migrate
```

Use an unused port and container name if these are occupied. The commands below
need only PostgreSQL; a relay, Redis, and object storage are unnecessary.

## Submit and see the row

```bash
./target/debug/buzz-admin user-deletions submit \
  --target demo:alice --requested-by operator-a
```

The command prints the saved row as JSON. It contains:

| Field | Value |
| --- | --- |
| `id` | New UUID; copy this for inspection |
| `entity_type` | `user` |
| `target` | `demo:alice` |
| `status` | `inventoried` |
| `requested_by` | `operator-a` |
| `reason` | `null`, unless supplied with `--reason` |
| `created_at` | Submission time |
| `updated_at` | Same time; no execution has started |
| `inventory_manifest` | Ordered communities and relationships |
| `inventory_digest` | SHA-256 identifying the saved inventory |

List recent requests, then inspect the UUID returned above:

```bash
./target/debug/buzz-admin user-deletions list --limit 10
REQUEST_ID=<returned-uuid>
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID"
```

To see the PostgreSQL row directly:

```bash
docker exec buzz-user-deletion-intake psql -X -U buzz -d buzz -c \
  "SELECT id, entity_type, target, status, requested_by, created_at
   FROM entity_deletion_requests ORDER BY created_at DESC;"
```

Exactly one row is saved in `entity_deletion_requests`. Repeating the same
target, operator, and reason returns that row without changing its timestamps.
Reusing the target with a different operator or reason fails and preserves the
original row. Use a new `demo:` label for a new request.

## Approve without starting execution

Inspect the inventory before approving. Copy its `inventory_digest` into the
command so approval is bound to what you inspected:

```bash
INVENTORY_DIGEST=<digest-from-inspect>
./target/debug/buzz-admin user-deletions approve "$REQUEST_ID" \
  --approved-by operator-b --inventory-digest "$INVENTORY_DIGEST"
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID"
```

The parent now has `status: approved`. Approval evidence lives in the separate
`entity_deletion_approvals` table. Each row has `id`, `request_id`,
`inventory_digest`, `approved_by`, and `approved_at`. Inspection returns these
rows in an `approvals` array (empty before approval), ordered by time and ID.
The parent keeps the inventory; approval rows bind each operator's decision to
its digest without overwriting earlier decisions.

`stage_history` is empty after approval. The first `run` creates the initial
stage. Inspection reads parent, approvals, and history from one consistent
snapshot. No worker is started by approval.

```bash
docker exec buzz-user-deletion-intake psql -X -U buzz -d buzz -c \
  "SELECT id, status, inventory_digest
   FROM entity_deletion_requests ORDER BY created_at DESC;"
docker exec buzz-user-deletion-intake psql -X -U buzz -d buzz -c \
  "SELECT id, request_id, approved_by, approved_at, inventory_digest
   FROM entity_deletion_approvals ORDER BY approved_at, id;"
docker exec buzz-user-deletion-intake psql -X -U buzz -d buzz -c \
  "SELECT request_id, stage, status, is_current, target_community_id,
          started_at, finished_at, metadata_json
   FROM entity_deletion_stage_history;"
```

The first approval inserts an approval row and changes the parent status in
one transaction. A failure rolls both back.
Repeating the same operator and digest preserves the approval ID and timestamp.
Another operator adds a new approval row, with its own time, without creating
another stage or changing the earlier approval:

```bash
./target/debug/buzz-admin user-deletions approve "$REQUEST_ID" \
  --approved-by operator-c --inventory-digest "$INVENTORY_DIGEST"
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID"
```

There are now two approvals and still no stage rows. The unique
key `(request_id, inventory_digest, approved_by)` makes retries idempotent.
A digest that does not match the saved inventory is rejected. This slice has no
inventory replacement or later approval gates yet; those policies can be added
without replacing these historical rows. A partial unique index prevents two
current stages for one request. Executor retries update the same stage row.

Operator names are self-asserted demo input. This is not authenticated approval
for real deletion. Owned communities and actual deletion handlers belong to
later changes. Continue with [the executor walkthrough](operator-user-deletion-executor.md)
to run the five dummy stages, inspect transitions, and try failure recovery.

## Upgrade and rollout

Apply after the intake schema from PR #8496. Migrations 0069 and 0070 add
inventory and execution fields, widen the lifecycle constraint, and create the
operator-global stage-history and approval tables. Constraint validation uses a
separate transaction so it does not retain the schema-change transaction's
write-blocking lock. Both
migrations set a five-second lock timeout and a thirty-second statement timeout.
There are no new triggers, functions, foreign keys, or data backfills.

Existing `submitted` requests remain unchanged by migration. Resubmit with the
same target, operator, and reason to prepare their dummy inventory; the request
ID and original creation time stay unchanged. The previous CLI can still insert
and read requests against the expanded schema. The serving relay does not
automatically execute these requests; an operator runs the CLI worker explicitly.
Stage history and approvals intentionally survive
community deletion as operator recovery/audit evidence.

Verify on a test environment with the submit, inspect, approve, and SQL commands
above using a new `demo:` target. Rollback has the usual sqlx limitation: an older
binary running auto-migration refuses unknown migrations (`VersionMissing`).
Do not roll back to an older auto-migrating relay without the deployment's
reviewed rollback procedure; the additive schema itself can remain in place.

The unmerged draft migration 0069 changed when approvals moved to their own
table, and execution fields were added to requests. If you tested an earlier
draft, use a fresh disposable local database;
do not edit its sqlx checksums or erase its saved demo requests.

PR #8520 did not change SQL schema. Earlier draft requests whose
stage metadata predates saved inputs/results/predecessor IDs are not a supported
resume format. Keep their records intact and submit a new `demo:` label to test
this version. Approval no longer creates a gate row; it is initialized by the
first `get_next_stage` call under worker ownership.
