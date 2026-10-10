# Native channel-label command E2E

This runs the candidate CLI against **two separate candidate relay processes**
sharing isolated PostgreSQL and Redis. It uses the production HTTP bridge and
NIP-98 admission. A loopback proxy discards one successful response **after**
commit, records request hashes, then forwards the exact-event retry. This is not
a mocked receipt server.

## Requirements and execution

- An empty, disposable PostgreSQL database and disposable Redis service. Do not
  point this harness at Desktop, staging, production, or another test's database.
- Built `buzz`, `buzz-relay`, and `buzz-admin` binaries from the same checkout.
- Python 3.10+ (Hermit `uv` can use an existing managed Python). No Python packages.
- Local loopback ports; seven free ports are selected. A bind race fails setup.

```bash
. ./bin/activate-hermit
bin/cargo build -p buzz-cli -p buzz-relay -p buzz-admin
# Create a fresh DB using your isolated PostgreSQL administration connection.
# The harness migrates it; it never drops a database or changes DB credentials.
export LABEL_DATABASE_URL='postgres://user:password@127.0.0.1:5432/labels_unique_run'
export LABEL_REDIS_URL='redis://127.0.0.1:6379'
bin/uv run --no-project --offline python scripts/channel-labels-e2e/run.py   --evidence /tmp/buzz-e2e/labels-unique-run   --confirm-isolated-empty-database
```

Use `--bin-dir` for another Cargo target directory. The evidence directory must
not exist. It is mode 0700 and contains generated fixture private keys: **do not
publish it wholesale**. Share sanitized logs and public artifact hashes only.
The harness records source revision, dirty status and all three binary SHA-256s
in `SOURCE.json`. `FLOWS.stdout` lists assertions; `PROXY.jsonl` binds the lost ACK
and exact retry to identical command bodies and different transport proofs.
`TEARDOWN.json` records stopped processes; the database remains for inspection.
Remove the run-owned database and evidence manually when no longer needed.

## Evidence boundaries

Measured here: labeled creation, verified reads, committed-but-lost ACK,
opposing mutation, exact retry without reapplication, no-op snapshot reuse,
ordinary topic/purpose preservation (including empty labels), concurrent CLI
additions, generic label discovery, unauthorized rejection, both backend
processes exercised, and final readiness of both processes.

The concurrent additions do **not** prove lock overlap. Deterministic lock
ordering, rollback, archive/deletion/removal races, current ACLs, cross-community
UUID/event collisions, stale discovery caches, mixed/id reads, activation, and
pagination are covered by the PostgreSQL/router suites instead. Run complete
packages with `scripts/postgres-test-run.sh -p buzz-db -p buzz-relay -p buzz-admin`;
see `crates/buzz-db/TESTING.md` for isolated runner configuration. Core and CLI
unit suites cover malformed grammar, snapshot trust and torn/exclusive journals.

Test-only `BUZZ_GIT_CONFORMANCE_PROBE=false` avoids requiring S3 for this non-Git
workflow. Git/media admission, Docker relay images, Kubernetes, forced live
cross-process overlap, multi-tenant bystander progress, process-kill recovery,
and deployment credential fencing are **not established by this harness**.
The offline cutover declaration enables these isolated test processes; it does
not exercise a real fleet upgrade. See `docs/channel-labels-rollout.md`.

The runner's readiness polling is bounded to 60 seconds; every command has a
bounded timeout and all run-owned processes are terminated on completion/failure.
Generated credentials never leave loopback test infrastructure.
