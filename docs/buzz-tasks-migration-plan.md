# Buzz Tasks PR 1 migration plan

PR 1 creates only the `buzz_tasks` projection, its constraints/index, and its
existing community write-fence trigger. It does not alter `events.search_tsv`
or rebuild `idx_events_search_tsv`.

## Lock and transaction behavior

SQLx runs migration 0031 in one transaction, so all locks are retained until
commit. `CREATE TABLE` takes `ACCESS EXCLUSIVE` on the new `buzz_tasks`
relation. Its foreign keys take `SHARE ROW EXCLUSIVE` on the referenced
`communities`, `channels`, and `users` tables; normal reads may continue, but
writes to those parent tables wait. `CREATE INDEX` takes `SHARE` and the
write-fence `CREATE TRIGGER` takes `SHARE ROW EXCLUSIVE`, both only on the new,
empty task table. Migration 0031 sets `lock_timeout = '5s'`, so it rolls back
instead of waiting indefinitely for a parent-table lock. There is no table lock
or heap rewrite on `events`, and no production GIN build.

This makes startup cost independent of the production `events` row count. The
child table is empty, so foreign-key validation and index creation scan no task
rows; parent-table size is not scanned. Actual production event volume therefore
does not change migration 0031's work. Before deployment, operators should still
record current sizes and active blockers as evidence for the separate FTS plan:

```sql
SELECT count(*) AS events_rows,
       pg_size_pretty(pg_total_relation_size('events')) AS events_total,
       pg_size_pretty(pg_relation_size('idx_events_search_tsv')) AS search_index;

SELECT pid, state, wait_event_type, wait_event, xact_start, query
FROM pg_stat_activity
WHERE xact_start IS NOT NULL
ORDER BY xact_start;
```

No production row-count measurement is needed to size a search rewrite in this
PR because that rewrite has been removed. The query above is a required rollout
record, not an input to migration 0031. Lock acquisition is still workload-
dependent, so deployment monitoring must watch blocked sessions and migration
duration. A lock timeout abort is safe and should be retried in a quieter window;
it must not be bypassed by increasing the timeout without a new review.

## Failure, rollback, and recovery

If migration 0031 is interrupted, PostgreSQL rolls back the transaction: the
new table, index, constraints, and trigger disappear together. Retry after the
blocking transaction or underlying error is removed. No invalid GIN index or
partially rewritten `events` heap can remain because PR 1 never touches either.

After a committed migration, application rollback may leave the unused
projection table in place. A schema rollback is operator-run: stop task event
publishers/readers and `DROP TABLE buzz_tasks`. Signed task events remain in
`events`, so a corrected projection can be rebuilt.

## Search privacy and future storage hardening

The global search SQL has an unconditional denylist for kinds 44300-44302,
applied before caller-supplied kind filters. This protects brownfield databases
even when their existing generated vector indexes arbitrary kinds. Fresh
databases additionally use migration 0008's positive storage allowlist.

Changing the generated column on a populated database requires dropping and
re-adding `search_tsv` and rebuilding its GIN index. PostgreSQL takes
`ACCESS EXCLUSIVE` for the generated-column replacement, blocking reads and
writes for a duration proportional to the production heap, plus index-build
time and WAL. That work is explicitly a separate, staged maintenance delivery:

1. measure row count, heap/index size, build time, WAL, and lock time on a
   production-sized clone;
2. choose a maintenance window and free-space/WAL headroom;
3. fail fast with a bounded `lock_timeout`;
4. run the rewrite transactionally or use a separately designed shadow-column
   plan if the measured outage is unacceptable; and
5. verify the expression and search-negative tests before resuming writes.

The existing `scripts/maintenance/nip_rs_search_allowlist.sql` documents the
same out-of-band pattern. It is not part of PR 1 deployment and must not be run
automatically before PR 2.
