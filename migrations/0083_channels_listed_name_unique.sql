-- Contract: two listed channels in a community never share a name, ignoring
-- case. A listed channel is an open, not unlisted stream or forum that is not
-- deleted; an archived one still counts. The previous release checks the rule
-- in the relay on create and rename; from this release the index enforces it,
-- and the relay reports a refused write as "invalid: an open channel named
-- ... already exists" (`LISTED_NAME_INDEX` in crates/buzz-db/src/store/channel.rs).
--
-- Before deploying, every serving relay must enforce the rule and
-- scripts/maintenance/rename_duplicate_listed_channel_names.sh must have
-- renamed the existing duplicates. See docs/channel-name-uniqueness-deployment.md.
--
-- The migration builds the index itself, as 0049 does, under the lock and
-- statement budgets below (docs/database.md Rule 6). A deployment too large
-- for them migrates out of band and builds the index concurrently first; the
-- migration then only verifies it.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';
-- pg_get_indexdef qualifies the enum casts below (`public.channel_visibility`)
-- when `public` is not on the search_path; pin it so the exact compare holds.
SET LOCAL search_path = public, pg_catalog;
-- IF NOT EXISTS still requests a writer-conflicting ShareLock before checking
-- whether the index exists. Bypass CREATE entirely for a prebuilt index, then
-- validate it below; a failed concurrent build must never count as success.
DO $$
BEGIN
    IF to_regclass('public.idx_channels_listed_name') IS NULL THEN
        IF EXISTS (
            SELECT 1 FROM public.channels
            WHERE visibility = 'open' AND NOT unlisted
              AND channel_type IN ('stream', 'forum') AND deleted_at IS NULL
            GROUP BY community_id, lower(name)
            HAVING count(*) > 1
        ) THEN
            RAISE EXCEPTION 'Migration 0083 stopped: listed channels with duplicate names remain. Rename them with scripts/maintenance/rename_duplicate_listed_channel_names.sh (docs/channel-name-uniqueness-deployment.md, steps 2 and 3), then migrate again.';
        END IF;

        CREATE UNIQUE INDEX idx_channels_listed_name
            ON public.channels (community_id, lower(name))
            WHERE visibility = 'open' AND NOT unlisted
              AND channel_type IN ('stream', 'forum') AND deleted_at IS NULL;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indexrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relname = 'idx_channels_listed_name'
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) =
              'CREATE UNIQUE INDEX idx_channels_listed_name ON public.channels USING btree (community_id, lower((name)::text)) WHERE ((visibility = ''open''::channel_visibility) AND (NOT unlisted) AND (channel_type = ANY (ARRAY[''stream''::channel_type, ''forum''::channel_type])) AND (deleted_at IS NULL))'
    ) THEN
        RAISE EXCEPTION 'Migration 0083 stopped: idx_channels_listed_name is invalid or has a different definition. Drop it and build it again (docs/channel-name-uniqueness-deployment.md, Recovery), then migrate again.';
    END IF;
END $$;
