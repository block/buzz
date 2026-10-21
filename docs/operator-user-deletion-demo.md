# User deletion workflow demonstration

This exercises real PostgreSQL requests, approvals, leases, stage history and
recovery for a synthetic user in three participant communities. Every action
prints `DEMO ONLY`. It does not discover or delete real users, change memberships,
gate access, contact storage, or delete owned communities.

## Prepare an isolated development database

Activate Hermit and start a separate PostgreSQL instance. Use a different
container name and port from a running Buzz development environment.

```bash
. ./bin/activate-hermit
docker run --detach --name buzz-user-deletion-demo \
  --publish 127.0.0.1:55432:5432 \
  --env POSTGRES_USER=buzz \
  --env POSTGRES_PASSWORD=buzz-local-demo \
  --env POSTGRES_DB=buzz postgres:17-alpine
export DATABASE_URL=postgres://buzz:buzz-local-demo@127.0.0.1:55432/buzz
cargo build -p buzz-admin
./target/debug/buzz-admin migrate
```

These are disposable local credentials. If the port is occupied, choose another
one in both the Docker command and URL. No Redis or media-storage configuration
is required by `user-deletions`.

## Submit, prepare and approve

An operator manually verifies the incoming request in the real operational
process. This demonstration uses `demo:alice`; it is not an account lookup.
Only `demo:` labels are accepted, and inventory is always the fixed participant
fixture. A different label creates an independent demonstration.

```bash
./target/debug/buzz-admin user-deletions list
./target/debug/buzz-admin user-deletions submit \
  --target demo:alice --requested-by operator-a
```

Copy the returned `id` into `REQUEST_ID`. Submission alone creates a `submitted`
request. Repeating the identical target/operator/reason returns that same
request without resetting its progress.

```bash
REQUEST_ID=<returned-uuid>
./target/debug/buzz-admin user-deletions inventory "$REQUEST_ID"
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID"
./target/debug/buzz-admin user-deletions approve "$REQUEST_ID" \
  --approved-by operator-b
```

Inventory saves the three targets and ordered plan with a digest. Approval must
come from a different operator and covers the saved digest. Approval also creates
the first pending/current stage. A request cannot run before approval.

## Execute and inspect

```bash
./target/debug/buzz-admin user-deletions run "$REQUEST_ID"
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID"
```

The 15 stages are:

| Sequence | Target | Action |
| --- | --- | --- |
| 1 | Request | `gate_user` |
| 2–4 | Communities A, B, C | `gate_community` for each |
| 5–7 | Community A | `drain`, `delete_data`, `verify_community` |
| 8–10 | Community B | `drain`, `delete_data`, `verify_community` |
| 11–13 | Community C | `drain`, `delete_data`, `verify_community` |
| 14 | Request | `verify_user` |
| 15 | Request | `post_delete` |

All gates are simulated. They do not enforce access restrictions.

`entity_deletion_requests` holds overall status, inventory and worker ownership.
`entity_deletion_stage_history` holds each reached stage's target, status,
`is_current`, `started_at`, `finished_at`, `metadata_json` and latest error.
The final parent status is `demo_completed`, with no current stage or live lease.
Completion output explicitly says no user data was deleted.

## Demonstrate recovery

Use a fresh synthetic target for each scenario and repeat submission, inventory
and approval first.

To stop after A finishes:

```bash
./target/debug/buzz-admin user-deletions run "$REQUEST_ID" --stop-after 7
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID"
./target/debug/buzz-admin user-deletions run "$REQUEST_ID"
```

The stop releases ownership and leaves B's `drain` row pending/current.
The next invocation claims a new generation and continues that row.

To fail during B's data-removal stage:

```bash
./target/debug/buzz-admin user-deletions run "$REQUEST_ID" --fail-at 9
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID"
# Wait until the saved next_attempt_at (normally 30 seconds after failure).
./target/debug/buzz-admin user-deletions drain
```

The failed command exits nonzero. A's completed rows remain unchanged, B's current
row records the error, and ownership is released. The next invocation without
`--fail-at` resumes B. The flag injects a failure on every invocation where it is
specified; it is not a persistent request setting.

Five consecutive failures at the same stage block automatic claims. After
remediation, unblock and retry:

```bash
./target/debug/buzz-admin user-deletions unblock "$REQUEST_ID" \
  --unblocked-by operator-c --reason "Removed the injected failure"
./target/debug/buzz-admin user-deletions run "$REQUEST_ID"
```

`--step-delay-ms 15000` allows observing periodic heartbeats or interrupting a
worker. SIGTERM/Ctrl-C releases its matching lease. After an abrupt process exit,
a later worker can claim the request once `lease_until` expires. An old generation
cannot update stage progress. A printed action whose transition did not commit
can print again when retried.

## Repeatable checks

The PostgreSQL lane uses an isolated database per test. Follow
`crates/buzz-db/TESTING.md` for its environment and run:

```bash
scripts/test-postgres-test-discovery.sh
scripts/postgres-test-run.sh -p buzz-db -p buzz-deletion --lib \
  -E 'test(entity_deletion) | test(user_postgres_tests) | (package(buzz-deletion) & test(postgres_tests))'
```

Run the actual binary demonstration against a disposable, migrated database:

```bash
BUZZ_USER_DELETION_TEST_DATABASE_URL="$DATABASE_URL" \
  python3 scripts/test-user-deletion-demo.py --binary ./target/debug/buzz-admin
```

That script creates unique synthetic requests and verifies approval rejection,
completion, stopping/resumption, failure in B followed by a delayed retry,
and heartbeat renewal followed by SIGTERM and resumption.
It leaves the demonstration rows for inspection.

Schema migrations may roll out normally while these tables are unused.
Introducing real execution requires a separate change and cleanup of
demonstration records. Owned-community policy, real identity discovery, real
write gates and real cleanup handlers remain future work.
