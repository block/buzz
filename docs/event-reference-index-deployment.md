# Event-reference index deployment

Migration 0089 adds `idx_events_reference_ids` for content-deletion planning
and final reference checks. It indexes valid hexadecimal `e` IDs in lowercase.
Signed event rows and the existing tag index remain unchanged. The index adds
storage and event-insert work; it avoids expanding community history while a
deletion batch blocks community writers.

This is an additive schema expansion. Existing relays can continue serving
while the index is prepared. Apply the schema before running the new content
handler. Both preview and deletion refuse a missing, invalid or differently
defined index. Every delete batch rechecks it under the shared schema lock
before taking the exclusive community write lock.

## Small databases and automatic migration

Migration 0089 builds the missing index inline, including on populated tables,
with a one-second lock timeout and five-second statement timeout. A valid
prebuilt index is verified without requesting the table lock for index creation.
Desired-state installations declare the same ordinary GIN index in
`schema/tables/public/events.sql`.

## Large databases and out-of-band migration

Keep automatic migration off. Before deploying the new handler, run:

```sh
psql -X --set=ON_ERROR_STOP=1 --dbname="$DATABASE_URL" \
  --file=scripts/maintenance/prebuild-event-reference-index.sql
```

Run outside a transaction, on the writer. The procedure creates an `ON ONLY`
parent, builds each missing partition index concurrently, and attaches the
children. Concurrent builds have a 30-minute statement budget and may wait for
old snapshots; short metadata operations retain a one-second lock budget.
Completion validates the exact parent definition and ready/live/valid flags.
Then apply migration 0089 through the ordinary migration runner.

Do not create or detach event partitions during this procedure. If a new
partition appears, rerun and require the final validation to succeed. Once the
valid parent exists, newly created partitions inherit its index automatically.

An interrupted build may leave an invalid unattached child. Rerunning removes
only invalid, unattached indexes whose definition matches the parent, then
rebuilds and attaches them. A same-named parent of another shape is an error;
inspect it instead of silently replacing it. No event rows are changed.

## Verification and recovery

```sql
SELECT i.indisvalid, i.indisready, i.indislive,
       pg_get_indexdef(i.indexrelid)
FROM pg_index i
WHERE i.indexrelid = 'public.idx_events_reference_ids'::regclass;
```

All three flags must be true and the definition must match migration 0089.
Check `EXPLAIN (ANALYZE, BUFFERS)` for the shipped final-reference query on
representative history; populated partitions should use the normalized GIN
index for selective absence checks. Do not force planner settings to obtain
that result.

After a timeout, fix the blocking transaction or use the concurrent procedure
before retrying migration. Do not disable the handler's index check. Coordinate
any index removal or replacement through the exclusive schema/destruction
lock used by the migration runner; never drop it out of band during deletion.

A code rollback may leave this additive index installed. As with every newer
schema version, an older binary must run with automatic migration disabled,
otherwise SQLx rejects the unknown migration version. Rolling back to a handler
with case-sensitive reference checks restores that correctness defect; avoid
running content deletion with that older handler.
