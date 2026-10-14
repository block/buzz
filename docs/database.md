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
| Schema source of truth | Two parallel sources: ordered sqlx migrations in `migrations/` and the desired-state manifest in `schema/` | The declarative desired state in `schema/tables/public/` is the source of truth. Deployments that apply schema out of band plan each change from it; Block's hosted deployment uses SchemaBot for this, but the source does not depend on any one tool. `migrations/` stays, because auto-migration stays supported (next row). Each app owns its own migrations and declarative schema: the relay's copies of the push gateway's tables are debt to remove (Rule 4). |
| Who applies DDL | Startup migrations when `BUZZ_AUTO_MIGRATE` is on (binary default off, Helm chart default on), `buzz-admin migrate`, the push gateway's pre-upgrade Helm Job, and partition creation at relay startup and on a schedule | Both modes stay supported. Deployments at scale, including Block's hosted deployment, apply schema changes out of band through a reviewed workflow before deploying, and their serving relay runs no DDL. Smaller and self-hosted deployments MAY keep auto-migration on. Still open: how the two modes share one schema source, and how auto-migration orders a release whose expand, backfill, and contract steps arrive in one upgrade (Rule 5). Monthly partition creation moves out of the relay under BUZZ-178. Changes to partitioned tables (`events`, `delivery_log`) are outside SchemaBot's envelope: it refuses `CREATE TABLE ... PARTITION OF` on a live parent and concurrent index builds on partitioned parents. |
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
- **At scale, startup should not change the schema.** A relay that migrates at
  boot can hold locks on hot tables during a rolling deploy. A relay that
  starts against an already-compatible schema cannot. Auto-migration stays
  supported for deployments where that risk is acceptable, so every change
  must be safe in both modes (Rule 5).
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
does; this document treats any edit to an existing function's body, other
than dropping or narrowing it, as widening. Put the behavior in the Rust transaction that owns the write:

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

**No exception for the community write fence.** A new table MUST NOT get a
fence trigger, including by calling the existing helper
`attach_community_write_fence`. Writers of a new tenant-owned table MUST be
fenced in the application instead, by community admission. Today
`AdmittedTx` (`crates/buzz-db/src/runtime/admitted_tx.rs`) wraps event-write
transactions: event ingestion and the writes it makes in the same
transaction. It is constructed by `DeletionStore::guard_transaction`, which
takes the shared community deletion lock and refuses a community that is not
`active`. Helpers that take `&mut AdmittedTx` reject a raw transaction at
compile time, but crate code can still bypass the type, so the fence triggers
remain the authoritative backstop (ARCHITECTURE.md § *Supported Writer
Contract*). A table without a trigger has no database backstop. The design
that extends admission to every fenced table and names a chokepoint for each
writer is BUZZ-314; the implementation, which ends by dropping the fence
triggers and functions, is BUZZ-315. That work MUST also update
ARCHITECTURE.md § *Dual enforcement*.

**Today this blocks new tenant-owned tables.** Community deletion's catalog
checks still assume that every tenant-scoped table carries a trigger fence:

- An unlisted `community_id` table, fenced or not, fails the exact-set check
  against `EXPECTED_SCOPED_TABLES`
  (`community deletion catalog drift (missing=…, unknown=…)`). Listing it
  without a trigger fails the write-fence drift check. Either way, deletion
  fails closed for every community, not only the new table's.
- Relay startup refuses a binary that lists the table in
  `EXPECTED_SCOPED_TABLES` when the database has no live fence on it
  (`community serving write fences missing`).
- Desired-state bootstraps run `schema/community_write_fence.sql`, which
  attaches a fence trigger to every `community_id` table automatically.

So before a feature can add a tenant-owned `community_id` table, these checks
must recognise application-fenced tables, and the desired-state loop must stop
attaching triggers to them. That change belongs to the BUZZ-314/BUZZ-315 work.
Land it before the feature, or as the feature's first PR. Do not work around
it with a trigger, or with an entry in `community_write_fence_excluded_table`
for a table the community owns.

#### Checklist: adding a tenant-owned `community_id` table

After the catalog change above has landed, all in the same PR:

- [ ] Every writer of the table goes through community admission
      (`AdmittedTx`, or the chokepoint BUZZ-314 defines for its writer).
      Nothing writes the table outside it.
- [ ] No fence trigger: no `attach_community_write_fence` call in the
      migration and no `schema/triggers/` file.
- [ ] The table in `EXPECTED_SCOPED_TABLES` (`crates/buzz-db/src/store/deletion.rs`),
      plus any registration the catalog change introduces for
      application-fenced tables.
- [ ] The table in `PURGE_SCOPED_TABLES` in the same file, ordered child
      before parent. A table missing here keeps its rows after the purge, and
      deletion verification fails closed.
- [ ] The migration count in `embedded_migrator_contains_consolidated_initial_schema`.
- [ ] A Postgres-gated test showing that a write to the table is refused once
      deletion of its community has started, and that the purge removes its
      rows.
- [ ] A run of the Postgres-gated migration-mode tests (see
      [crates/buzz-db/TESTING.md](../crates/buzz-db/TESTING.md)). Most tests
      use desired state, which builds the catalog differently.

#### Checklist: adding a provenance-only table

A table whose `community_id` only records which community a row came from, and
whose rows must stay writable while that community is being deleted, is not
owned by that community and is not fenced. The existing ones (the deletion
control plane, `product_feedback`, `rate_limit_violations`, and
`operator_listener_outbox`) predate this rule and are listed in
`community_write_fence_excluded_table`. A new one MUST name its column
something other than `community_id`, as
`relay_admin_actions.report_community_id` does, so the catalog does not treat
the table as scoped. Adding a case to the exclusion function would widen an
existing function, which Rule 1 forbids. Renaming the
column also removes the table from every automatic deletion surface, so rows
that name a deleted community survive deletion and no check fails, unless the
PR follows this checklist. All in the same PR:

- [ ] Register it as operator-global in all three places: the
      `_operator_global_tables` seed row in the migration, `schema/seeds/`, and
      the hard-coded name list in `operator_global_tables()`
      (`crates/buzz-db/src/runtime/migration.rs`). The lint ignores a registry
      row whose name is not on that list, and without registration the
      `community_id NOT NULL` and key-leads-with-`community_id` lints fail.
- [ ] Choose and implement its community-deletion disposition in
      `crates/buzz-db/src/store/deletion.rs`:
  - **sever:** set the provenance column to `NULL` in the `purge_postgres`
    transaction, as `product_feedback` and `rate_limit_violations` are;
  - **delete and verify:** add it to `RELAY_ADMIN_TABLES` with a row source,
    as the `relay_admin_*` tables are; or
  - **retain on purpose:** add no deletion code, and state the reason in a
    comment beside the table's migration and in the PR, as
    `operator_listener_outbox` is retained. No check enforces this choice.
- [ ] A Postgres-gated deletion test that seeds a row for the target community
      and asserts the chosen disposition. For delete and verify, call
      `purge_postgres` and `verify_postgres_logically_deleted`, following
      `purge_postgres_succeeds_with_relay_admin_action_rows`. For sever, call
      `purge_postgres` and assert the column is `NULL`. This is new coverage:
      no existing test drives the sever path through `purge_postgres`.

A `DO` block in a migration is not a persisted function. It MAY guard
idempotent DDL, as migration `0049` does, or make a contract migration refuse
to run (Rule 5). It MUST NOT be used to run data repair.

### 2. No foreign keys

A change MUST NOT add `REFERENCES` or `FOREIGN KEY`, including in a new table.
For a new reference:

- Store the referenced key, led by `community_id` for tenant-scoped data.
- Check that the referenced row exists, in the same community, inside the
  writing transaction. Lock it or otherwise serialize against a concurrent
  delete when a dangling reference would be unsafe.
- Delete or detach dependent rows explicitly in the transaction that deletes
  the parent. Community deletion MUST cover the new table; see the checklist
  under Rule 1.
- Make readers tolerate a missing parent. Without a database constraint, a
  dangling row is possible after an unsupported write, and readers MUST NOT
  panic or leak data across communities when they meet one.

Removing an existing foreign key requires its replacement checks and explicit
deletes to ship first, then the constraint drop as a separate contract step.

### 3. Backfills are not migrations

A migration MUST contain schema changes only. It MUST NOT `UPDATE`, `INSERT`,
or `DELETE` existing rows, and MUST NOT scan or rewrite a table while holding a
write-blocking lock. Scans that do not block writes, such as
`VALIDATE CONSTRAINT` or a bounded concurrent index build, are allowed (Rule 6).

- Historical data repair MUST be a backfill definition that conforms to
  [data-backfills.md](data-backfills.md): stable ID, finite upper bound,
  bounded batches, atomic mutation plus checkpoint, and validation.
- **Until the backfill framework ships**, a repair MAY instead be a one-off
  SQL script in `scripts/maintenance/`, reviewed in its own PR and run by an
  operator. The script MUST be idempotent and safe to rerun, MUST work in
  bounded batches that each commit on their own, and MUST end with a
  validation query whose expected result the PR states. Commit per batch
  from a top-level `DO` block with `COMMIT` (outside any `BEGIN`) or from a
  psql or shell loop; never with `CREATE FUNCTION` or `CREATE PROCEDURE`,
  even a temporary one. Once the framework ships, new repairs MUST use it.
- A repair script MUST respect community deletion, as
  [ARCHITECTURE.md](../ARCHITECTURE.md) § *Supported Writer Contract*
  requires of operator maintenance. Each batch MUST take
  `pg_advisory_xact_lock_shared(community_deletion_lock_key(<community>))` for
  every community it touches, and MUST touch only rows whose community has
  `deletion_state = 'active'` and `deleted_at IS NULL`, as
  `DeletionStore::guard_transaction` does. Otherwise the PR MUST document a
  reviewed procedure that keeps the routing fences closed. A script MUST NOT
  disable triggers or set `session_replication_role`.
- The normal write path that maintains the new invariant MUST be fully rolled
  out, with old writers drained, before the backfill starts. The backfill spec
  explains why and what breaks otherwise.
- Constant seed rows that a new table or registry needs, such as an
  `_operator_global_tables` entry, are schema, not backfill. They MAY ship in a
  migration and MUST also appear in `schema/seeds/`. A seed row the running
  relay reads (today only `replica_heartbeat`) MUST also be converged and
  asserted in `scripts/reconcile-schema-after-pgschema.sql`. Rows read only by
  the static lint, such as `_operator_global_tables`, do not go there.

### 4. Change every schema location

Until the declarative schema is the only source, one logical change spans several files. A PR
that changes the schema MUST update all of these that apply:

| Location | What goes there |
|---|---|
| `migrations/NNNN_<name>.sql` | The forward migration for existing databases. Use the next free number; rebase and renumber on a collision. Never edit a merged migration: its checksum changes, and relays that run the migrator fail with sqlx `VersionMismatch`. Fix forward in a new migration. |
| `schema/tables/public/<table>.sql` | The table's final `CREATE TABLE` and its `CREATE INDEX` statements, one table per file. This is the declarative schema source. |
| `schema/schema.sql` | An `\i` line for each new file, after the tables it depends on. |
| `schema/types/`, `schema/partitions/`, `schema/seeds/` | The matching non-table objects. The layout test permits nothing else here. Rule 1 forbids any addition to `functions/` and `triggers/`. |
| `scripts/reconcile-schema-after-pgschema.sql` | Idempotent convergence, plus a live assertion, for anything `pgschema` does not preserve: seed rows, some storage parameters, partition attachment, and the shapes listed in AGENTS.md *Common Gotchas* #7. |
| `crates/buzz-push-gateway/migrations/` **and** the relay copies | The six `push_gateway_*` tables have two homes. The gateway applies its own migration history to its own database, through a pre-install/pre-upgrade Helm Job ([push-gateway-deployment.md](push-gateway-deployment.md)). The relay schema also defines them: `migrations/` (`0015`, `0043`, `0045`, `0060`), `schema/tables/public/push_gateway_*.sql`, and `schema/seeds/push_gateway_*.sql`. Until the duplication is removed, a gateway schema change goes in both, as relay `0060` mirrors gateway `0006`. Every rule here applies to both. The target is for the gateway to own its tables alone, in its own migrations and declarative schema; do not add new gateway tables to the relay schema. |

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
and operator-global tables are registered in all three places listed in the
provenance-only checklist under Rule 1.

### 5. Every schema change is backward compatible with the running app

Buzz supports two ways of applying migrations, and every change MUST be safe
under both:

- **Auto-migration.** The relay binary defaults `BUZZ_AUTO_MIGRATE` to off,
  but the Helm chart defaults `migrate.autoMigrate` to `true`. With it on, the
  first new pod migrates at boot while old pods are still serving.
- **Out of band.** An operator applies the migrations while the current relay
  keeps serving traffic, then deploys the new relay. Block's hosted
  deployment works this way at scale, with `migrate.autoMigrate=false`.

Either way, for a period the **old relay runs against the new schema**. If the
new relay is rolled back, the old relay runs against the new schema again.
Every migration MUST be safe for the currently deployed release to read and
write against.

The relay has no general schema-version compatibility check. It verifies only
specific catalog surfaces, such as the deletion serving fence and the replica
floor guard, so the ordering below is the main protection. When
`BUZZ_AUTO_MIGRATE` is on, an older binary refuses a database that has
migrations it does not know (sqlx `VersionMissing`), so `helm rollback` with
auto-migration on fails at startup, and so does any old pod that restarts
mid-rollout after a new pod has migrated. A rollback therefore depends on the older
binary not running the migrator. Making rollback work under auto-migration is
part of supporting both modes.

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

**Under auto-migration the steps can collapse.** The migrator applies every
pending migration at one boot, before the new code serves and before any
backfill or repair script runs. An operator who upgrades across several
releases therefore applies expand and contract together. How auto-migration
should order such an upgrade is unresolved. Until it is, a contract migration
MUST fail closed when the step it depends on has not run:

- `VALIDATE CONSTRAINT` and `SET NOT NULL` already fail on unrepaired rows.
- A contract that drops a column or table, or otherwise loses data, MUST first
  check that the backfill or repair has completed, and `RAISE` from a
  top-level `DO` block when unrepaired rows remain.
- The PR MUST add a Postgres-gated test that applies the expand and contract
  migrations in one run against unrepaired rows and asserts the migration
  errors without dropping data.

A failed contract stops the relay from starting, which is an outage but not
data loss. The release notes MUST tell auto-migrating operators which repair
to run before upgrading past that release.

#### What breaks the old relay

| Change | Why it breaks | Safe form |
|---|---|---|
| Add a `NOT NULL` column without a default | Old `INSERT`s omit it and fail | Add it nullable, deploy writers, backfill, then add the constraint (see below) |
| Rename a column or table | Old queries reference the old name | Add the new name, dual-write, backfill, move readers, then drop the old name |
| Change a column type | Old binds and decodes mismatch, and the change may rewrite the table | Add a new column and migrate through the expand/contract sequence |
| Add `CHECK`, `UNIQUE`, or a narrower index | Old writes violate it, or an old `ON CONFLICT` target no longer matches an index | Deploy writers that satisfy it, backfill, then add it. Add a `CHECK` as `NOT VALID`, then `VALIDATE CONSTRAINT` in a later step. Keep every index an old `ON CONFLICT` names until no deployed code uses it |
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

The desired-state file declares only the final `col ... NOT NULL`, and that
change lands in the same PR as migration C. Until then, the desired state keeps
the column nullable. Between A and C, migrated databases carry
`t_col_not_null` and fresh ones do not, so exclude that constraint from any
parity assertion on the table for that window.

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
- [ ] That no migration mutates existing rows, and the backfill ID (or,
      until the framework ships, the `scripts/maintenance/` script and how it
      takes the community deletion lock) for any repair.
- [ ] For a contract migration, how it fails closed when the step it depends
      on has not run.
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
