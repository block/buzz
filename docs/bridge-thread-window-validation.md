# Thread-window validation and rollout notes

## Candidate and evidence boundary

Validated on 2026-09-22, branch `cid/thread-window`, against base
`77729abfb692b25a0f4ec4a69add86af2e32c0dd`. The production diff is confined to
core/DB/relay, migration 0049, desired-state schema and protocol documentation.
No client opt-in, compatibility fallback or UI behavior is shipped here.
See [NIP-TW](nips/NIP-TW.md) for the normative contract and deployment procedure.

The evidence root for this run is
`WORK_LOGS/CID_THREAD_WINDOW_20260922/` in the contributor's Buzz workspace,
outside the checkout. This is a local artifact path, not a public download.
`FINAL_STATE.txt`, the recorded command logs and the benchmark source digest
identify the tested uncommitted source; the commit containing this report is
its reviewable delivery. Do not attribute these measurements to the base alone.

Topology: native debug relay/test binaries; isolated Docker PostgreSQL 17.10
and Redis; per-test databases created by the repository's nextest wrapper.
Replica tests use divergent databases and controlled reader transactions, **not
physical streaming replication**. No Kubernetes, production load, media or Git
object-store conformance claim is made.

## Invariant / evidence ledger

| Invariant | Authority and premises | Probe and control |
|---|---|---|
| No legacy behavioral change | No opt-in: existing dispatch/SQL | Signed router fixture with mobile `thread_cursor: -1`, depth `2147483647`, both cursor spellings, absent/false flag; exact rows and no new bounds |
| Complete newest-first traversal | Pre-probe scope/kind/depth/deletion predicates; raw cursor | DB walks of 0/1/50/51/501/10,000 replies, same-second groups, damaged 50th candidate, tombstoned root; both schema paths |
| Bounds describe authorized scope | Fresh writer authorization; host-derived tenant; relay signer | Signed NIP-98 router requests, expected signer/signature/tags/binding, denied and revoked member with deliberately stale cache, colliding channel IDs across tenants, wrong-channel/non-conversation roots |
| Clients cannot forge authority | Shared HTTP/WS ingest relay-only kind gate | HTTP submission plus WS event-handler rejection of client-signed 39007 |
| Aux never silently truncates | Raw scan metadata; two-hop closure; shared budgets | 1,001-row positive control then corruption, deleted aux ID plus channel-less deletion, sentinel aux exclusion, 8,192-row and 65th-query rejection, aggregate 8 MiB rejection |
| Deadline includes authorization | One HTTP new-mode deadline; cancellation | Hold `channel_members` DDL lock: 503 at eight seconds; unlock and same request succeeds |
| Reader coverage is not update freshness | Cursor upper fence and retained REPEATABLE READ transaction | Divergent missing-middle fixture and deliberately overclaimed-fence control; default writer head; recent edit/delete absent in held snapshot but visible in new one |
| Degradation restarts closure | Failed reader transaction degrades once; ledger persists | Terminate reader after first aux page, insert newer writer edit tombstone/deletion, require deletion returned with no duplicate events; closed reader acquisition uses writer |
| Index cannot silently be wrong | Catalog definition/validity gate; migration transaction | Wrong-order and invalid same-name indexes reject without advancing ledger; concurrent prebuild keeps OID; desired-state catalog checked |
| Binary rollback remains possible | Old binary must not run old embedded migrator | Old binary boot/read/write on version-48 DB with auto-migration off; new binary reads old write after restart; old migrator rejects 48 as expected |

Tests live in:

- `crates/buzz-core/src/thread_window.rs`
- `crates/buzz-db/src/store/thread_window/postgres_tests.rs`
- `crates/buzz-db/src/runtime/tests/thread_window_postgres_tests.rs`
- `crates/buzz-relay/src/api/bridge/thread_window/postgres_tests.rs` and children

## Reproduction commands

Activate Hermit and provide an **isolated** PostgreSQL administrator URL,
`PGHOST/PGPORT/PGUSER/PGPASSWORD`, and Redis URL as described in
[`buzz-db/TESTING.md`](../crates/buzz-db/TESTING.md). Ordinary relay tests also
need `DATABASE_URL`, `TEST_DATABASE_URL` and `BUZZ_TEST_DATABASE_URL` pointing to an existing
schema-loaded disposable database (some media tests are not marked ignored).
Do not use a stale template name after a PostgreSQL lane cleans it up.

```bash
. ./bin/activate-hermit
git rev-parse HEAD
scripts/test-postgres-test-discovery.sh
scripts/postgres-test-run.sh -p buzz-db -p buzz-relay --lib --tests --test-threads 8
cargo nextest run -p buzz-core -p buzz-db -p buzz-relay --test-threads 4 --no-fail-fast
cargo test -p buzz-core -p buzz-db -p buzz-relay --doc
cargo check -p buzz-core -p buzz-db -p buzz-relay
cargo clippy -p buzz-core -p buzz-db -p buzz-relay --all-targets -- -D warnings
cargo fmt --all -- --check
just file-size-check security-review-check
```

The PostgreSQL lane passed **394/394**, with 1,228 tests outside that lane
skipped. Full ordinary touched-package suites ran **1,473**: **1,472 passed**,
one failure, 406 ignored tests. The failure was
`api::mesh_demo::tests::demo_join_forwarded_arm_round_trips_echo` returning 504
rather than 200. It also failed in isolation and on the unmodified base with
identical Redis configuration (`BASELINE_MESH_INFRA.log`). A base run without
Redis returned early; that was **not** a successful mesh round trip.
Doctests (2), check, Clippy, format, PostgreSQL discovery and static gates passed.
Repository-wide `just ci` and its unrelated client builds/tests were not run;
this is not an all-CI-green or independent security approval claim.

Earlier failed attempts are preserved, not reclassified as passing: stale
ordinary-suite DB name / omitted `DATABASE_URL` (six media setup failures), in-process tracing callsite
interference (passed under isolated nextest), mesh timeout, and a cached-generic
SQL plan timeout. `POSTGRES_SIGNOFF.log`, `ORDINARY_SIGNOFF_2.log`,
`FLAKES_ISOLATED.log` and `BASELINE_MESH_INFRA.log` separate those outcomes.

## Measured SQL / index costs

Fixture: 100,001 replies, 100-row same-second groups, 500-byte content,
10% kind 40002 and approximately 12.5% depth 2, warm local Docker database.
Five `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` samples per case; milliseconds
below are separate medians, not HTTP latency or production guarantees.

`BENCHMARK_VERIFIED.py` mechanically extracts the final QueryBuilder SQL and
placeholder order from `store/thread_window.rs`. Each sample prepares a fresh
parameterized statement, matching the parameter-aware first-plan behavior of
SQLx `persistent(false)`. It preserves `VERIFIED_*.sql`,
`EXPLAIN_VERIFIED_*.json` and `BENCHMARK_VERIFIED_RESULTS.json`.
Older `FINAL_*` files instead contain the **forced-generic counterexample**;
do not use those as final-candidate performance results.

| Query | Execution median ms | Planning median ms | Final sample shared hits |
|---|---:|---:|---:|
| Head | 0.317 | 8.267 | 241 |
| Deep cursor | 0.303 | 8.020 | 267 |
| Same-second cursor | 0.333 | 8.200 | 242 |
| Selective kind/depth | 2.183 | 7.891 | 2,644 |

A flattened event join chose root-wide hash joins/sorts despite the new index:
24–46 ms execution. The final parameterized `LATERAL ... LIMIT 1` lookup
preserves all pre-probe predicates and uses the unique event PK. The redundant
upper timestamp range enables a deep index start. Cached generic plans still
regressed (approximately 35 ms deep, 387 ms ties); only this new query uses an
unnamed statement. Planning cost remains real and is included above. No global
planner setting or legacy query changed.

Separately measured legacy SQL was approximately 42 ms head and 25 ms deep,
roughly unchanged with/without the new index. This does not solve the legacy
ASC/ASC access path or establish the production loading incident's cause.

Index/write experiment (`BENCHMARK_RESULTS.json`):

- Concurrent build: about 202 ms on this fixture; valid/ready/live afterward.
- Index size: 13,295,616 bytes.
- 10,000 metadata inserts, median execution: 385.045 ms with index versus
  363.016 ms without (about +6.1%).
- WAL sample: 8,970,356 versus 7,424,336 bytes (about +20.8%).
- Existing indexes retained; no second ASC/ASC index added.

A signed router fixture with 51 replies and 1,001 auxiliaries measured about
167 ms and 1,000,176 serialized response bytes in the delivery debug-build run
(`POSTGRES_DELIVERY.log`). Its spans separate writer authorization (~3.2/~1 ms),
selection (~14 ms), aux (~38 ms then ~3.5–9.5 ms/page), and pool acquisitions. This is diagnostic cost
attribution under local test load, not a stable latency benchmark.

## Standalone wire / rollback exercise

`ROLLBACK_3.py` and `ROLLBACK_3_RESULTS.log` capture two isolated relay binaries
sharing only their run-owned version-48 database, sequentially:

1. Feature binary auto-migrates, creates channel/root/reply through CLI.
2. Base binary boots with `BUZZ_AUTO_MIGRATE=false`, reads those messages and
   writes another reply.
3. Base `/query` ignores `thread_window` and returns HTTP 200 without 39007.
4. Feature binary restarts, reads the old write, walks newest-first one-row
   pages to terminal bounds, and matches independently computed binding hashes.

The standalone run uses loopback development authentication (`X-Pubkey` for
raw query probes; signed CLI writes) and disables the Git conformance probe
because no object store is provisioned. These are isolated test settings, not
production-default changes. The stronger NIP-98/signature/expected-signer
checks run through the real router integration tests above. Test relay
processes are stopped by the script even on assertion failure.

## Remaining rollout gates

- Prebuild concurrently on a representative populated deployment, under the
  approved schema-change workflow; inspect definition and valid/ready/live
  flags. A failed same-name remnant requires repair, not IF NOT EXISTS.
- Measure production-scale build/write cost, pooling and query selectivity.
- Validate actual replica topology if enabling replica routing; a reply fence
  proves insertion coverage, not current edits/deletions or immediate revocation.
- Resolve/track the inherited mesh failure and run repository-wide CI before
  merge. No client should opt in until it verifies every page's expected signer
  and request-bound bounds; absence is neither EOF nor proof of support.
