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
  `push_gateway_*` tables, `crates/buzz-push-gateway/migrations/`.
- Never edit a merged migration. Fix forward.
- Adding a `community_id` table has its own checklist in that document. A
  tenant-owned one is blocked until deletion's catalog checks accept
  application-fenced tables (BUZZ-314/BUZZ-315).
- Keep every migration safe for the relay that is currently deployed, whether
  migrations run at startup or out of band. Ship in
  this order: new schema, then new code, then backfill, then constraints and
  cleanup.
- Bound every lock (`SET LOCAL lock_timeout`). Build indexes on populated
  tables concurrently.
