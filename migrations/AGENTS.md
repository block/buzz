# Schema changes: read first

Before changing anything in this directory, read
[docs/database.md](../docs/database.md). In short:

- Add no triggers, functions, procedures, rules, or foreign keys. The one
  exception is the community write-fence attachment described in that
  document.
- Put no data backfills in migrations: no `UPDATE`, `INSERT`, or `DELETE` over
  existing rows. Use the backfill framework in
  [docs/data-backfills.md](../docs/data-backfills.md).
- Make each change in every schema location: `migrations/`,
  `schema/tables/public/`, the `schema/` manifest and sibling directories,
  `scripts/reconcile-schema-after-pgschema.sql` where it applies, and, for
  `push_gateway_*` tables, `crates/buzz-push-gateway/migrations/`.
- Never edit a merged migration. Fix forward.
- Adding a `community_id` table has its own checklist in that document. Run
  the migration-mode tests; desired-state tests hide a missing fence.
- Keep every migration safe for the relay that is currently deployed. Ship in
  this order: new schema, then new code, then backfill, then constraints and
  cleanup.
- Bound every lock (`SET LOCAL lock_timeout`). Build indexes on populated
  tables concurrently.
