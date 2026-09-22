# Thread-window validation and rollout notes

## Candidate and evidence boundary

Validated on 2026-09-22, branch `cid/thread-window`, against base
`77729abfb692b25a0f4ec4a69add86af2e32c0dd`. The production diff is confined to
core/DB/relay, migration 0049, desired-state schema and protocol documentation.
No client opt-in, compatibility fallback or UI behavior is shipped here.
See [NIP-TW](nips/NIP-TW.md) for the normative contract and deployment procedure.

The original delivery is `20de962da973f914dc16151bc68955f755af6bd6`; its evidence
root is `WORK_LOGS/CID_THREAD_WINDOW_20260922/` in the contributor's Buzz
workspace. The follow-up commit containing this revision of the report repairs
four findings from independent review. Migration 0048 is still unshipped and
its checksum changed: these runs use fresh disposable databases, not ledger
rewrites to upgrade a database initialized by the superseded candidate. Its
evidence root is
`WORK_LOGS/CID_THREAD_WINDOW_MIGRATION_FIX_20260922/`. Both are local artifact
paths outside the checkout, not public downloads. `FINAL_STATE.txt`, command
logs, source hashes and the recorded diff bind results to their tested source.
Follow-up suite logs show original HEAD plus uncommitted repairs, not a passing
rerun of the original commit. Do not conflate the two candidates.

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
| Aux access changes never sign incomplete closure | Fresh initial/final writer access sets | Deterministic first-aux-page barrier; grant and revocation in another channel both produce retryable 503, with before/after visibility and retry controls |
| Deadline includes authorization | Production writer-pool budgets plus HTTP upper deadline | Hold `channel_members` DDL lock: default five-second lock timeout produces sanitized 503; configured `lock_timeout_ms: 0` reaches eight-second deadline; unlock/retry succeeds in both cases |
| Reader coverage is not update freshness | Cursor upper fence and retained REPEATABLE READ transaction | Divergent missing-middle fixture and deliberately overclaimed-fence control; default writer head; recent edit/delete absent in held snapshot but visible in new one |
| Degradation restarts closure | Failed reader transaction degrades once; ledger persists | Terminate reader after first aux page, insert newer writer edit tombstone/deletion, require deletion returned with no duplicate events; closed reader acquisition uses writer |
| Index cannot silently be wrong or block writers after prebuild | Catalog definition/validity gate; schema advisory lock | Wrong-order and invalid same-name indexes specifically reject catalog shape; valid prebuild keeps OID/ledger 48 while an ingestion transaction remains open and witness inserts succeed during the migration transaction |
| Rapid insertion cannot require fresh statistics for a 50-row page | Ordered metadata boundary; unique-key event lookup before visibility filtering | Both 10k-reply schema-path walks disable metadata/event-partition autoanalyze; unchanged four-second statement budget |
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

The follow-up PostgreSQL lane passed **399/399**, with 1,228 tests outside that
lane skipped (`raw/FINAL_POSTGRES.log`). Full ordinary touched-package suites
ran **1,473**: **1,472 passed**, one failure, 411 skipped
(`raw/FINAL_ORDINARY.log`). The failure was
`api::mesh_demo::tests::demo_join_forwarded_arm_round_trips_echo` returning 504
rather than 200. It also failed in isolation and on the unmodified base with
identical Redis configuration (original `BASELINE_MESH_INFRA.log`). A base run
without Redis returned early; that was **not** a successful mesh round trip.
Doctests (2), check, all-targets Clippy with warnings denied, format, PostgreSQL
discovery and static gates passed (`raw/FINAL_GATES.log`). Repository-wide
`just ci` and unrelated client builds/tests were not run. This is not an
all-CI-green or independent approval claim; reviewers assessed the original
SHA and must re-review the repairs separately.

### Review findings and falsifiable repairs

1. **Prebuilt-index migration blocked ingestion (coffee).** PostgreSQL takes a
   writer-conflicting ShareLock for `CREATE INDEX IF NOT EXISTS` before checking
   existence. Migration 0048 now bypasses CREATE on the catalog-only prebuilt
   path, while retaining exact validity/definition checks and schema advisory
   locking. The new overlapping writer/migration/witness test fails on the
   original SQL and passes repaired. Malformed-index tests require the specific
   catalog rejection, not any error (`raw/RED_POSTGRES.log`,
   `raw/FINAL_POSTGRES.log`; the intermediate `raw/GREEN_POSTGRES.log` passes
   the migration regression but still fails the stale-statistics case).
2. **Auxiliary access grants produced signed incomplete closure (am).** The
   original one-way removal check is replaced by complete HashSet equality.
   Grant/revocation tests pause polling after the first auxiliary page and
   commit the membership change before continuing. The grant control returned
   HTTP 200 on original code; repaired code returns retryable 503 and a retry
   has the expected new visibility (`raw/RED_AUX_POSTGRES.log`).
3. **Stale statistics exhausted the SQL deadline (cyberpunk and cid).** The
   original delivery run reported 394/394, but independent reruns at that SHA
   reproduced a 10k-reply continuation failure: 393/394 in cyberpunk's full
   lane, also reproduced serially. Actual SQLx auto_explain evidence shows a
   custom/unnamed plan taking about 4.99 seconds, roughly 9,900 lateral lookups
   and 2.96 million buffer hits. This was not merely load or the earlier generic
   plan issue. The repair orders metadata before lateral lookups and separates
   unique-key event lookup from visibility predicates; all predicates still
   precede outer limit+1. The new regression keeps autoanalyze disabled in both
   schema paths. The diagnostic 30-second allowance was only for collecting
   the failing plan; production remains four seconds
   (`raw/DIAGNOSTIC_30S_PG.log:246–293`, `raw/FINAL_POSTGRES.log`).
4. **Production authorization lock timeout returned 500 (peon).** The raw-pool
   router fixture missed the deployed five-second writer lock budget. Ordinary
   thread-window fixtures now use `Db::new(DbConfig { ..Default::default() })`
   and production after-connect policy. The new-mode adapter classifies SQLSTATE
   57014/55P03 and pool acquisition timeout as sanitized retryable 503; unrelated
   faults stay sanitized 500. The default-lock regression failed at HTTP 500
   before the fix (397/398 passed), then passed; the separate eight-second test
   explicitly disables the optional DB lock timeout through production config.
   The replica-failure fixture still constructs injected reader/writer sessions
   intentionally (`raw/RED_PRODUCTION_TIMEOUT_2.log`, `raw/FINAL_POSTGRES.log`).

Earlier failed attempts remain evidence, not passes: stale ordinary-suite DB
name / omitted `DATABASE_URL` (six media setup failures), in-process tracing
callsite interference (passed under isolated nextest), mesh timeout, and the
original generic-plan timeout. Original `POSTGRES_SIGNOFF.log`,
`ORDINARY_SIGNOFF_2.log`, `FLAKES_ISOLATED.log` and `BASELINE_MESH_INFRA.log`
retain those outcomes. Follow-up `raw/RED_PRODUCTION_TIMEOUT.log` is a compile
setup failure (a non-re-exported constant), not the successful red control;
`raw/RED_PRODUCTION_TIMEOUT_2.log` is the actual HTTP 500 reproduction.

## Measured SQL / index costs

Fixture: 100,001 replies, 100-row same-second groups, 500-byte content,
10% kind 40002 and approximately 12.5% depth 2, warm local Docker database.
Five `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` samples per case; milliseconds
below are separate medians, not HTTP latency or production guarantees.

Follow-up `BENCHMARK_ORDERED.py` mechanically extracts the final QueryBuilder
SQL and placeholder order from `store/thread_window.rs` (source SHA-256
`2d9c84c38193cfd2700e88f97b6779a49a12accf37b2b0a98ae6f342906fd799`). Each
sample prepares a fresh parameterized statement, matching SQLx
`persistent(false)` first/custom-plan behavior. It preserves `VERIFIED_*.sql`,
`EXPLAIN_VERIFIED_*.json` and `BENCHMARK_VERIFIED_RESULTS.json` in the follow-up
evidence root. The original root's `BENCHMARK_VERIFIED.py` and results describe
the superseded query; its older `FINAL_*` files are a forced-generic
counterexample, not measurements of either final candidate.

| Query | Execution median ms | Planning median ms | Final sample shared hits |
|---|---:|---:|---:|
| Head | 0.359 | 8.110 | 241 |
| Deep cursor | 4.801 | 8.226 | 520 |
| Same-second cursor | 0.340 | 7.919 | 242 |
| Selective kind/depth | 2.767 | 8.346 | 2,644 |

The repaired deep query is **slower** on this warm 100k fixture than the
original measured 0.303 ms. This is an availability repair under stale
statistics, not a blanket performance improvement. The ordered subquery's
`OFFSET 0` is an optimization boundary, not offset pagination: no eligible
candidate is skipped or prematurely limited. Metadata can still be sorted
under poor estimates, but event lookups occur afterward and use the unique key
before applying channel/deletion/kind predicates. The outer limit+1 remains
after eligibility. No global planner setting, SQL deadline or legacy query
changed. Planning cost remains real and is included above.

Original comparison experiments remain distinct: a flattened event join took
24–46 ms; forced generic plans took about 35 ms deep / 387 ms ties. Avoiding
those plans alone did not prevent the independently reproduced 10k failure.

Separately measured legacy SQL was approximately 42 ms head and 25 ms deep,
roughly unchanged with/without the new index. This does not solve the legacy
ASC/ASC access path or establish the production loading incident's cause.

Index/write experiment (original `BENCHMARK_RESULTS.json`; index definition
unchanged by the follow-up):

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

The original `ROLLBACK_3.py` / `ROLLBACK_3_RESULTS.log` and follow-up
`ROLLBACK_FOLLOWUP.py` / `raw/ROLLBACK_FOLLOWUP_RESULTS.log` each capture two
isolated relay binaries sharing only a fresh run-owned version-48 database,
sequentially. Both runs passed:

1. Feature binary auto-migrates, creates channel/root/reply through CLI.
2. Base binary boots with `BUZZ_AUTO_MIGRATE=false`, reads those messages and
   writes another reply.
3. Base `/query` ignores `thread_window` and returns HTTP 200 without 39007.
4. Feature binary restarts, reads the old write, walks newest-first one-row
   pages to terminal bounds, and matches independently computed binding hashes.

The follow-up additionally holds a confirmed lock-acquisition barrier on
`channel_members` while querying the live binary over TCP. With default writer
configuration, it returns HTTP 503 and only
`{"error":"thread database timeout; retry window"}` in **5.007 seconds**;
after rollback of the lock transaction the same query returns 200 with one
bounds event. Successful pre-lock pages and post-unlock recovery distinguish
this from an unrelated setup failure. This supplements the failing-original /
passing-repaired production-constructor regression above.

The standalone runs use loopback development authentication (`X-Pubkey` for
raw query probes; signed CLI writes) and disable the Git conformance probe
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
