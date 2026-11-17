-- A channel's parent: the channel whose readers can also read this one.
-- Set with kind:9002 ["parent", <uuid>] and cleared with ["parent", ""];
-- published in kind:39000 as ["parent", <uuid>]. The relay keeps the tree
-- at most three channels deep and free of loops when it writes the column.
-- NULL on every existing row: no channel has a parent until someone sets one.
-- No foreign key (docs/database.md); a parent that is later deleted grants
-- nothing, because every read joins it on `deleted_at IS NULL`.
--
-- The index is built inline, as 0083 does, under the lock and statement
-- budgets below (docs/database.md Rule 6). It is partial on a column that
-- is NULL everywhere, so it stays empty until parents are set. A deployment
-- too large for the budgets migrates out of band and builds it concurrently
-- first; the migration then only verifies it.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';
SET LOCAL search_path = public, pg_catalog;

ALTER TABLE channels ADD COLUMN IF NOT EXISTS parent_id UUID;

DO $$
BEGIN
    IF to_regclass('public.idx_channels_parent') IS NULL THEN
        CREATE INDEX idx_channels_parent
            ON public.channels (community_id, parent_id)
            WHERE parent_id IS NOT NULL;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indexrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relname = 'idx_channels_parent'
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) =
              'CREATE INDEX idx_channels_parent ON public.channels USING btree (community_id, parent_id) WHERE (parent_id IS NOT NULL)'
    ) THEN
        RAISE EXCEPTION 'Migration 0089 stopped: idx_channels_parent is invalid or has a different definition. Drop it and build it again, then migrate again.';
    END IF;
END $$;
