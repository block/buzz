# PostgreSQL-backed tests in buzz-db

The dedicated PostgreSQL CI lane discovers tests and Cargo packages by
structure rather than by exact lists. Follow this checklist so a new database
test is run automatically and remains safe under parallel execution.

## Adding a test

1. Put the test in a module whose name ends in `postgres_tests`.
2. Mark it `#[ignore = "requires Postgres"]` so infrastructure-free unit-test
   jobs stay fast.
3. Connect through `crate::test_support::database_url()`. The CI wrapper sets
   this helper's environment to a unique database for each test process; never
   hard-code the shared development database.
4. Keep tests that need infrastructure beyond PostgreSQL and Redis in an
   `external_infra*_tests` module. The PostgreSQL lane excludes those tests.
5. Run `scripts/test-postgres-test-discovery.sh` after adding or moving the
   test. The same guard runs in CI immediately after changed-path detection.

The wrapper isolates destructive tests by dropping the entire per-test
database after the process exits. It does not `DELETE` rows or `TRUNCATE`
shared tables, so tests may run concurrently without coordinating cleanup.

## Choose the schema intentionally

Most tests use the committed desired-state schema from `schema/schema.sql`.
That is the default and is appropriate for data-access behavior.

Tests in `migration::postgres_tests` receive an empty database and own the
embedded migration lifecycle. A test outside that module that intentionally
depends on migration-created triggers or seed rows must prefix its function
name with `migration_schema_`; it also receives an empty database with
`BUZZ_TEST_SCHEMA_MODE=migration`.

Helpers that normally run migrations honor `BUZZ_TEST_SCHEMA_MODE=desired` in
the default lane. Do not rerun migrations against a desired-state database.
When behavior should match in both schema paths, add explicit desired-state and
migration-applied coverage rather than making the bootstrap implicit.

Tests that inspect cluster-wide PostgreSQL state or open least-privilege
sessions include `cluster_global_` in the function name. Migration-backed cases
use `migration_schema_cluster_global_`. Nextest serializes this small group
because separate databases still share `pg_stat_activity` and roles.

## Run the lane locally

Start native PostgreSQL and Redis, activate Hermit, and run:

```bash
. ./bin/activate-hermit
scripts/test-postgres-test-discovery.sh
scripts/postgres-test-run.sh
```

Set `BUZZ_POSTGRES_ADMIN_URL` to a PostgreSQL maintenance database owned by a
role that can create and drop databases. Set `PGHOST`, `PGPORT`, `PGUSER`, and
`PGPASSWORD` for desired-state bootstrap, plus `REDIS_URL` for Redis-backed
tests. The complete privilege-boundary inventory also needs `CREATEROLE` and
membership in `pg_read_all_stats`, or an ephemeral superuser as CI uses.

The runner creates one desired-state source database per invocation and clones
it for ordinary tests. Migration-mode tests start empty. Cleanup retries
transient disconnect races before reporting a warning.
