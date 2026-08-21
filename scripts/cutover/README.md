# 1321 cutover: single-community → multi-tenant

`1321_backfill_default_community.sql` is a **one-off operator script**, not a
startup migration. It takes a pre-1321 (single-community) Buzz Postgres to the
1321 multi-tenant schema, assigning every existing row to one default community
derived from the deployment host.

> It is **not** embedded in the relay binary. The relay's `sqlx::migrate!`
> embeds only `migrations/0001_initial_schema.sql` — the consolidated 1321
> schema. Fresh deployments need only that; this script exists solely to carry
> existing pre-1321 data across the rewrite. (It uses psql client features —
> `\set`, `\ir`, `:'var'` interpolation, `\gset` — that the embedded sqlx
> migrator cannot run, which is why it must live here and not under
> `migrations/`.)

## When you need it

Only when upgrading a Postgres that already holds **pre-1321 single-community
data** to 1321. A brand-new deployment does **not** run this — it provisions
from `migrations/0001_initial_schema.sql` (or `schema/schema.sql`) directly.

## Preconditions

- The DB is pristine pre-1321: no `communities` table, no `community_id`
  columns, no leftover `legacy` schema. The script's guard **refuses to run**
  otherwise (already-migrated, partially-migrated, or failed-prior-run states
  all raise and abort — see the guard block at the top of the script).
- **Take a `pg_dump` / PVC snapshot first.** There is no down-path. The
  snapshot **is** the rollback.

## Run it

The whole script runs in **one transaction** (all-or-nothing). Pass the
deployment host as `:host`; it is normalized exactly as the relay normalizes a
community host (`rtrim(lower(host), '.')`).

```sh
psql "$DATABASE_URL" \
     -v host="'your-deployment-host.example.com'" \
     -v ON_ERROR_STOP=1 \
     -f scripts/cutover/1321_backfill_default_community.sql
```

`-f` (not stdin) matters: the script `\ir`-includes
`../../migrations/0001_initial_schema.sql` **relative to its own location**, so
it resolves the canonical 1321 schema regardless of your cwd.

### What it does

1. Renames all pre-1321 tables and enum types aside into a `legacy` schema.
2. `\ir`-includes the repo's `0001` verbatim → the correct 1321 schema in a
   clean `public` (structurally identical to a fresh 1321 DB **by
   construction** — no hand-transcription).
3. Creates the one default community from `:host`.
4. Copies every row forward, stamped with that `community_id` (`search_tsv`
   regenerates as GENERATED; new-in-1321 columns default NULL).
5. `DROP SCHEMA legacy CASCADE`.

## After it commits

Boot the 1321 relay with **`BUZZ_AUTO_MIGRATE=false`**. The schema is already
correct; the relay's idempotent boot-time `ensure_configured_community` /
allowlist→relay_members backfill finds this community and no-ops.

If you want the relay's sqlx migrator to consider `0001` already applied (so a
later `BUZZ_AUTO_MIGRATE=true` boot doesn't try to re-run it), seed its
`_sqlx_migrations` row to match a fresh 1321 install:

```sql
-- version 1, description 'initial schema'; checksum must match the embedded 0001.
-- Easiest: diff _sqlx_migrations against a freshly-migrated 1321 DB and copy the row.
```

This is **not** required for correct operation — a fresh `0001` run against the
already-correct schema is a no-op only if the migrator's checksum matches, so
prefer leaving `BUZZ_AUTO_MIGRATE=false` unless you have a reason to seed.

## Verify (post-commit, outside the txn)

```sql
SELECT count(*) FROM communities;                       -- expect 1
SELECT count(*) FROM events WHERE community_id IS NULL;  -- expect 0
SELECT to_regnamespace('legacy');                        -- expect NULL (gone)
```

For a stronger guarantee, take a constraint/trigger/index/column diff of this
DB against a freshly-migrated 1321 DB — it should be empty.
