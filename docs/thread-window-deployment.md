# Thread-window index deployment

Migration `0049_thread_window_index.sql` adds the index used by opt-in,
newest-first thread windows:

```sql
CREATE INDEX idx_thread_metadata_window
    ON public.thread_metadata
        (community_id, root_event_id, event_created_at DESC, event_id ASC);
```

The migration bounds both lock acquisition and execution time. That is safe for
fresh or small databases, but it deliberately fails instead of holding a
write-conflicting table lock while building the index on a populated database.
Brownfield deployments must prebuild the exact index concurrently before
starting a relay version that includes migration 0049.

## Brownfield procedure

Run the following against the relay database with a role allowed to create an
index. `CREATE INDEX CONCURRENTLY` cannot run inside a transaction block.

```sql
CREATE INDEX CONCURRENTLY idx_thread_metadata_window
    ON public.thread_metadata
        (community_id, root_event_id, event_created_at DESC, event_id ASC);
```

Then verify that PostgreSQL considers the index ready, live, valid, and an exact
match for the definition expected by migration 0049:

```sql
SELECT
    i.indisvalid,
    i.indisready,
    i.indislive,
    pg_get_indexdef(i.indexrelid) AS definition
FROM pg_index AS i
JOIN pg_class AS c ON c.oid = i.indexrelid
JOIN pg_namespace AS n ON n.oid = c.relnamespace
WHERE n.nspname = 'public'
  AND c.relname = 'idx_thread_metadata_window';
```

Expected flags are all `true`. The expected definition is:

```text
CREATE INDEX idx_thread_metadata_window ON public.thread_metadata USING btree (community_id, root_event_id, event_created_at DESC, event_id)
```

After verification, deploy normally. Migration 0049 detects the prebuilt index,
skips the write-conflicting `CREATE INDEX`, validates the catalog shape again,
and records the migration.

## Recovery

A cancelled or failed concurrent build can leave an invalid index behind. Do not
retry deployment against that remnant: migration 0049 rejects invalid, unready,
non-live, or differently defined indexes.

Remove only the failed index, outside a transaction, then repeat the prebuild and
verification steps:

```sql
DROP INDEX CONCURRENTLY IF EXISTS public.idx_thread_metadata_window;
```

Do not replace this with a non-concurrent build on a populated production table.
The bounded startup failure is intentional; ingestion availability takes
precedence over completing the rollout in one attempt.
