-- Effective default channels per community: the open, unarchived stream
-- channels with `auto_join` set. On a community that does not require relay
-- membership, every human AUTH lists the ones the person has never joined
-- (`list_never_joined_auto_join_channels` in crates/buzz-db/src/store/channel.rs);
-- without this index that read walks every open channel in the community to
-- find the few that are flagged. The partial index holds only flagged rows, so
-- it stays tiny and the read becomes one index probe. Additive and expand-only:
-- the running relay ignores it.
--
-- Built inline under the budgets below, as 0049 does (docs/database.md Rule 6).
-- A deployment that applies migrations out of band and cannot build it within
-- them prebuilds the exact index first, then applies this migration, which
-- only verifies it:
--   CREATE INDEX CONCURRENTLY idx_channels_auto_join ON public.channels (community_id)
--       WHERE auto_join AND channel_type = 'stream' AND visibility = 'open'
--         AND archived_at IS NULL AND deleted_at IS NULL;
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';
-- pg_get_indexdef qualifies the enum casts below (`public.channel_type`) when
-- `public` is not on the search_path; pin it so the exact compare holds.
SET LOCAL search_path = public, pg_catalog;
-- IF NOT EXISTS still requests a writer-conflicting ShareLock before checking
-- whether the index exists. Bypass CREATE entirely for a prebuilt index, then
-- validate it below; a failed concurrent build must never count as success.
DO $$
BEGIN
    IF to_regclass('public.idx_channels_auto_join') IS NULL THEN
        CREATE INDEX idx_channels_auto_join
            ON public.channels (community_id)
            WHERE auto_join AND channel_type = 'stream' AND visibility = 'open'
              AND archived_at IS NULL AND deleted_at IS NULL;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indexrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relname = 'idx_channels_auto_join'
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) =
              'CREATE INDEX idx_channels_auto_join ON public.channels USING btree (community_id) WHERE (auto_join AND (channel_type = ''stream''::channel_type) AND (visibility = ''open''::channel_visibility) AND (archived_at IS NULL) AND (deleted_at IS NULL))'
    ) THEN
        RAISE EXCEPTION 'Migration 0087 stopped: idx_channels_auto_join is invalid or has a different definition. Drop it, build it again (see the comment at the top of this migration), then migrate again.';
    END IF;
END $$;
