# Submit a synthetic user-deletion request

This first slice saves and reads requests. It does not discover users, build
inventory, approve requests, start workers, or delete data. Only synthetic
`demo:` labels are accepted.

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
| `status` | `submitted` |
| `requested_by` | `operator-a` |
| `reason` | `null`, unless supplied with `--reason` |
| `created_at` | Submission time |
| `updated_at` | Same time; no execution has started |

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

This slice stops at `submitted`. Inventory, approval, stage history, worker
ownership, and execution will be added in later changes. Their fields and tables
can be added through normal additive migrations.
