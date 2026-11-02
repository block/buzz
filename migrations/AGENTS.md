# Schema changes: read first

Before changing anything in this directory, read
[docs/database.md](../docs/database.md). In short:

- Add no triggers, functions, procedures, rules, or foreign keys. There are no
  exceptions, including the community write-fence attachment.
- Put no data backfills in migrations: no `UPDATE`, `INSERT`, or `DELETE` over
  existing rows. Use the backfill framework in
  [docs/data-backfills.md](../docs/data-backfills.md), or, until it ships, a
  reviewed one-off script in `scripts/maintenance/`.
- Make each change in every schema location: `migrations/`,
  `schema/tables/public/`, the `schema/` manifest and sibling directories,
  `scripts/reconcile-schema-after-pgschema.sql` where it applies, and, for
  existing `push_gateway_*` tables, `crates/buzz-push-gateway/migrations/`.
  Do not add new gateway tables here.
- Never edit a merged migration. Fix forward.
- Adding a `community_id` table has its own checklist in that document. A
  tenant-owned one is blocked until desired-state bootstraps stop attaching
  fence triggers to every `community_id` table.
- Keep every migration safe for the relay that is currently deployed, whether
  migrations run at startup or out of band. Ship in this order: new schema,
  then new code, then backfill, then constraints and cleanup. A contract
  migration fails closed if its backfill has not run, and its release notes
  name the writer release that upgrades must not skip.
- Bound every lock (`SET LOCAL lock_timeout`).
