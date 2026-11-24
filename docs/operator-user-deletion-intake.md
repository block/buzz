# Prepare and approve a user-deletion request

Use the subject’s public key to select current memberships. Preview and submission use the same query. Submission saves the list and its digest in one transaction. Approval binds the request to that list.

## Prepare a local database

From the repository root, activate Hermit and start a separate PostgreSQL
container. These are disposable local credentials.

```bash
. ./bin/activate-hermit
export POSTGRES_PASSWORD="$(openssl rand -hex 24)"
docker run --detach --name buzz-user-deletion-intake \
  --publish 127.0.0.1:55433:5432 \
  --env POSTGRES_USER=buzz \
  --env POSTGRES_PASSWORD \
  --env POSTGRES_DB=buzz postgres:17-alpine
export DATABASE_URL="postgres://127.0.0.1:55433/buzz?user=buzz&password=${POSTGRES_PASSWORD}"
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
  --pubkey "$USER_PUBKEY" --requested-by operator-a
```

The command prints the saved row as JSON. It contains:

| Field | Value |
| --- | --- |
| `id` | New UUID; copy this for inspection |
| `entity_type` | `user` |
| `target` | Normalized user public key |
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
original row.

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

Operator names are self-asserted. The workflow starts only when an operator runs the approved request. See [the executor walkthrough](operator-user-deletion-executor.md) for the stage order and retry behavior.
