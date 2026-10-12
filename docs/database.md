# Database: target architecture and schema-change rules

`draft`

This document states where the Buzz PostgreSQL database is going and the rules
every schema or data change must follow on the way there. Read it before
changing anything under `migrations/`, `schema/`,
`scripts/reconcile-schema-after-pgschema.sql`, or a `buzz-db` query whose
behavior depends on schema shape.

The key words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** are to be
interpreted as described in BCP 14 when they appear in all capitals.

## Target architecture

PostgreSQL stores data. The application owns behavior.

| Concern | Today | Target |
|---|---|---|
| Schema source of truth | Two parallel sources: ordered sqlx migrations in `migrations/` and the desired-state manifest in `schema/` | The declarative desired state in `schema/tables/public/` is the source of truth. SchemaBot plans each change from it in review and applies it. |
| Who applies DDL | Opt-in startup migrations (`BUZZ_AUTO_MIGRATE`), `buzz-admin migrate`, and partition creation at relay startup and on a schedule | A reviewed external workflow. The serving relay runs no DDL, and its database role cannot. |
| Integrity between tables | Some foreign keys, including `ON DELETE CASCADE` and `ON DELETE SET NULL` | None in the database. The owning application transaction checks references and performs deletes explicitly. |
| Guards and follow-up writes | Triggers and PL/pgSQL functions (`schema/functions/`, `schema/triggers/`) | None in the database. Guards run in the owning writer's transaction; follow-up work is recorded in the same transaction and processed as durable, retryable application work. |
| Data repair | Some historical repair embedded in migrations | Backfill definitions in application code, run by the durable framework in [data-backfills.md](data-backfills.md). |
| Rollout | Effectively schema and code together | Expand, then deploy, then backfill, then contract. Each step is applied and verified on its own. |

The directories in `schema/` other than `tables/` hold objects SchemaBot cannot
apply. They are expected to shrink toward zero. A change that adds to them
moves away from the target.

### Why

- **SchemaBot cannot manage what the target forbids.** SchemaBot's declarative
  PostgreSQL format has no foreign keys. It refuses to pull tables that carry
  triggers, comments, non-default relation options, or partitioning. It never
  executes `DROP TABLE`, and its native apply path admits only `CREATE TABLE`,
  `CREATE INDEX`, and bounded `ALTER TABLE` statements. Every new trigger,
  function, or foreign key is another object that must be removed before
  adoption.
- **Startup must not change the schema.** A relay that migrates at boot can
  hold locks on hot tables during a rolling deploy. A relay that starts against
  an already-compatible schema cannot.
- **Behavior in the database is hard to see, test, and roll back.** Rust
  owns the rest of the write path. Logic split between Rust and PL/pgSQL is
  logic with two owners.
- **Migrations are not a job runner.** A data rewrite inside a migration runs
  unbounded, in one transaction, while holding locks, with no checkpoint, pause,
  or recovery. The backfill framework exists to provide those properties.

The work is tracked as the safe-startup and SchemaBot adoption effort. Until it
finishes, existing triggers, functions, and foreign keys stay in place as
backstops. They are removed one at a time, after their application replacement
has shipped and been verified. They are not removed early, and new ones are not
added.

## Rules for changes now

These rules apply to every change, starting now, including changes that touch
only one table.

### 1. No new triggers or functions

A change MUST NOT add `CREATE TRIGGER`, `CREATE FUNCTION`, `CREATE PROCEDURE`,
or `CREATE RULE`. It MUST NOT widen what an existing trigger or function
does. Put the behavior in the Rust transaction that owns the write:

- **Validation or guards:** check inside the writer's transaction before
  committing. Use the existing community-write and replica-floor lock helpers
  described under the supported-writer contract in
  [ARCHITECTURE.md](../ARCHITECTURE.md).
- **Follow-up writes** (counters, queues, materialized state): write them in the
  same transaction, or record a durable work item atomically with the
  originating write and process it idempotently.
- **Derived values:** compute them in the application.

Dropping or narrowing an existing trigger or function is welcome. Follow the
retirement sequence below.

**Transitional exception: the community write fence.** Until the fence moves
into application writers, every new table with a `community_id` column MUST be
fenced. Do this by calling the existing helper,
`SELECT attach_community_write_fence('<table>')`, in the migration and in
`schema/triggers/<table>.sql`. The table also goes in the deletion store's
`EXPECTED_SCOPED_TABLES`. Without these steps, the migration lint fails, the
relay's serving-catalog check refuses to start, and community deletion reports
catalog drift. The call attaches an existing, generic trigger; it defines no
new behavior. No other trigger attachment is permitted.

A `DO` block in a migration is not a persisted function. It MAY guard
idempotent DDL, as migration `0049` does. It MUST NOT be used to run data
repair.

### 2. No foreign keys

A change MUST NOT add `REFERENCES` or `FOREIGN KEY`, including in a new table.
For a new reference:

- Store the referenced key, led by `community_id` for tenant-scoped data.
- Check that the referenced row exists, in the same community, inside the
  writing transaction. Lock it or otherwise serialize against a concurrent
  delete when a dangling reference would be unsafe.
- Delete or detach dependent rows explicitly in the transaction that deletes
  the parent. Community deletion MUST cover the new table; see the fence
  exception under Rule 1.
- Make readers tolerate a missing parent. Without a database constraint, a
  dangling row is possible after an unsupported write, and readers MUST NOT
  panic or leak data across communities when they meet one.

Removing an existing foreign key requires its replacement checks and explicit
deletes to ship first, then the constraint drop as a separate contract step.

### 3. Backfills are not migrations

A migration MUST contain schema changes only. It MUST NOT `UPDATE`, `INSERT`,
or `DELETE` existing rows, and MUST NOT run any statement whose work grows with
table size.

- Historical data repair MUST be a backfill definition that conforms to
  [data-backfills.md](data-backfills.md): stable ID, finite upper bound,
  bounded batches, atomic mutation plus checkpoint, and validation.
- The normal write path that maintains the new invariant MUST be fully rolled
  out, with old writers drained, before the backfill starts. The backfill spec
  explains why and what breaks otherwise.
- Constant seed rows that a new table or registry needs, such as an
  `_operator_global_tables` entry, are schema, not backfill. They MAY ship in a
  migration. They MUST also appear in `schema/seeds/` and
  `scripts/reconcile-schema-after-pgschema.sql`.

### 4. Change every schema location

Until SchemaBot owns the schema, one logical change spans several files. A PR
that changes the schema MUST update all of these that apply:

| Location | What goes there |
|---|---|
| `migrations/NNNN_<name>.sql` | The forward migration for existing databases. Use the next free number; rebase and renumber on a collision. |
| `schema/tables/public/<table>.sql` | The table's final `CREATE TABLE` and its `CREATE INDEX` statements, one table per file. This is the SchemaBot-readable source. |
| `schema/schema.sql` | An `\i` line for each new file, after the tables it depends on. |
| `schema/types/`, `schema/partitions/`, `schema/seeds/` | The matching non-table objects. The layout test permits nothing else here, and Rule 1 forbids additions to `functions/` and `triggers/`. |
| `scripts/reconcile-schema-after-pgschema.sql` | Idempotent convergence, plus a live assertion, for anything `pgschema` does not preserve: seed rows, some storage parameters, partition attachment, and the shapes listed in AGENTS.md *Common Gotchas* #7. |
| `crates/buzz-push-gateway/migrations/` | Push-gateway schema, which lives in its own database. It has no desired-state directory today, but every rule here still applies. |

A fresh desired-state database and a migrated database MUST converge on the
same live catalog. The layout test
(`desired_state_schema_layout_is_complete_and_schemabot_shaped`) checks file
structure only. Parity tests cover selected surfaces, and the full drift gate
does not exist yet. For tables and indexes you touch, add or extend a parity
assertion that compares the live catalog from both bootstrap paths, following
`admin_schema_parity_between_desired_state_and_migrations`. A string match
against `schema.sql` does not count.

The existing migration lints still apply. Every tenant-scoped table has
`community_id NOT NULL`, every key on a scoped table leads with `community_id`,
and operator-global tables are listed explicitly.

### 5. Every schema change is backward compatible with the running app

Production does not run migrations automatically. An operator applies them
while the current relay keeps serving traffic, then deploys the new relay.
For a period, therefore, the **old relay runs against the new schema**. If the
new relay is rolled back, the old relay runs against the new schema again.
Every migration MUST be safe for the currently deployed release to read and
write against.

The relay has no general schema-version compatibility check. It verifies only
specific catalog surfaces, such as the deletion serving fence and the replica
floor guard, so the ordering below is the main protection. When
`BUZZ_AUTO_MIGRATE` is on, an older binary refuses a database that has
migrations it does not know (sqlx `VersionMissing`). A rollback therefore
depends on the older binary not running the migrator.

One feature therefore usually ships as several PRs, each deployed before the
next starts:

1. **Expand (schema).** Add only things the old relay ignores: nullable columns,
   columns with a constant default, new tables, new indexes. Add nothing the old
   relay could violate.
2. **Deploy (code).** Ship the relay that writes the new shape on every write
   path and reads both old and new rows correctly.
3. **Backfill (data).** After the new code is fully rolled out and old writers
   are drained, repair historical rows with a backfill (Rule 3). Readers stay
   correct for both repaired and unrepaired rows throughout.
4. **Contract (schema, then code).** After the backfill completes and
   validates, add the constraints you wanted from the start: `NOT NULL`,
   `CHECK`, `UNIQUE`. When nothing deployed still reads them, drop the old
   columns, indexes, triggers, or functions. Each tightening is its own
   migration, applied after the release that stops depending on the old shape.

Skip a step only when it has nothing to do. For example, a new table with no
historical data needs no backfill.

#### What breaks the old relay

| Change | Why it breaks | Safe form |
|---|---|---|
| Add a `NOT NULL` column without a default | Old `INSERT`s omit it and fail | Add it nullable, deploy writers, backfill, then add the constraint (see below) |
| Rename a column or table | Old queries reference the old name | Add the new name, dual-write, backfill, move readers, then drop the old name |
| Change a column type | Old binds and decodes mismatch, and the change may rewrite the table | Add a new column and migrate through the expand/contract sequence |
| Add `CHECK`, `UNIQUE`, or a narrower index | Old writes violate it, or an old `ON CONFLICT` target no longer matches an index | Deploy writers that satisfy it, backfill, then add it. Keep every index an old `ON CONFLICT` names until no deployed code uses it |
| Add an enum value that new code writes | Old code fails to decode the unknown variant | Deploy readers that accept the value, then deploy writers that produce it |
| Drop a column, table, index, trigger, function, or FK | Old code reads it or relies on its side effect | Contract step only, after the release that stops relying on it is fully deployed |
| Add a table with a `community_id` column | The old relay's exact-set deletion catalog check (`EXPECTED_SCOPED_TABLES`) sees an unknown table, so community deletion fails closed until the new release deploys. Serving is unaffected. | Expected and safe. Keep the gap between expand and deploy short, and do not run community deletion during it |
| `SELECT *` or `RETURNING *` decoded by position | Column order differs between desired-state and migrated databases and changes when columns are added | Select named columns |

#### Adding `NOT NULL` to a populated table

`ALTER TABLE ... SET NOT NULL` scans the whole table under `ACCESS EXCLUSIVE`.
Instead:

```sql
-- contract migration A
SET LOCAL lock_timeout = '5s';
ALTER TABLE t ADD CONSTRAINT t_col_not_null CHECK (col IS NOT NULL) NOT VALID;

-- contract migration B (separate step; scans but does not block writes)
SET LOCAL lock_timeout = '5s';
ALTER TABLE t VALIDATE CONSTRAINT t_col_not_null;

-- contract migration C (PostgreSQL 12+ uses the validated CHECK and skips the scan)
SET LOCAL lock_timeout = '5s';
ALTER TABLE t ALTER COLUMN col SET NOT NULL;
ALTER TABLE t DROP CONSTRAINT t_col_not_null;
```

The desired-state file declares only the final `col ... NOT NULL`.

#### Retiring a trigger, function, or foreign key

1. Ship application code that does the trigger's work, or the foreign key's
   check and cascade, while the database object is still in place. The two
   paths MUST be idempotent with each other, so the duplicate work is harmless.
2. After that release is fully deployed, drop the database object in a
   separate migration. Remove it from `schema/` in the same PR.
3. Removing a guard requires a falsifiable test showing that the application
   path enforces what the guard enforced.

### 6. Bound every lock and statement

- Every migration MUST start with `SET LOCAL lock_timeout`, and SHOULD set
  `SET LOCAL statement_timeout` when a statement can scan. A migration that
  cannot get its lock fails and is retried. It MUST NOT queue behind
  traffic on a hot table.
- An index on an existing populated table MUST be built with
  `CREATE INDEX CONCURRENTLY`. CONCURRENTLY cannot run inside the migration
  transaction, so follow the prebuild-then-verify procedure in
  [thread-window-deployment.md](thread-window-deployment.md). Partitioned
  parents need per-partition concurrent builds attached to an `ON ONLY` parent
  index.
- A change MUST NOT rewrite or fully scan a large table under a
  write-blocking lock. That includes column type changes, volatile defaults,
  `SET NOT NULL` without a validated check, and changes to the generated
  `events.search_tsv` column. Use the expand/contract forms above.
- Write changes that SchemaBot can apply. Its native path admits
  `CREATE TABLE`, `CREATE INDEX`, and bounded `ALTER TABLE` on tables up to its
  configured size ceiling (1 GiB by default), with short lock and statement
  budgets. A multi-statement plan is not atomic. A change outside that envelope
  needs a separately reviewed operational procedure, and the PR MUST say so.

### 7. Old and new relays coexist during rollout

Rolling deploys run two relay versions against one database. Durable records
written by the new release, such as backfill rows, outbox items, and new enum
values, MUST be handled safely by the old one. This means ignoring them,
leaving them for the new release, or failing closed. The old release MUST NOT
misinterpret them.

## Review checklist

A PR that touches the schema or a schema-dependent query states in its
description:

- [ ] Which phase this is (expand, deploy, backfill, or contract), and which
      PRs or releases come before and after it.
- [ ] That no trigger, function, procedure, rule, or foreign key is added.
- [ ] That no migration mutates existing rows, and the backfill ID for any
      repair.
- [ ] That every location in Rule 4 is updated, with a catalog-level parity
      test for each changed table and index.
- [ ] That the currently deployed relay works against the new schema, and how
      that was checked.
- [ ] Lock and statement timeouts, concurrent index builds, and any operator
      prebuild step.

## Related documents

- [ARCHITECTURE.md](../ARCHITECTURE.md): *Supported Writer Contract* and the
  `buzz-db` crate.
- [data-backfills.md](data-backfills.md): the backfill specification.
- [multi-tenant-conformance.md](multi-tenant-conformance.md): tenant-scoping
  obligations.
- [thread-window-deployment.md](thread-window-deployment.md): the brownfield
  concurrent-index procedure.
- [crates/buzz-db/TESTING.md](../crates/buzz-db/TESTING.md): desired-state
  and migration-mode tests.
- `schema/schema.sql` header: the desired-state layout.
- [SchemaBot PostgreSQL support](https://github.com/block/schemabot/blob/main/docs/postgresql.md):
  what SchemaBot can plan and apply.
