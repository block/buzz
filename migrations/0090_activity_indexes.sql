-- Indexes for GET /buzz/v1/me/activity on the columns 0089 added to existing
-- tables. Separate from 0089 so a deployment that applies migrations out of
-- band can prebuild them between the two:
--
--   CREATE INDEX CONCURRENTLY personal_read_frontiers_seq
--       ON public.personal_read_frontiers (community_id, actor, seq);
--   CREATE INDEX CONCURRENTLY idx_event_mentions_pubkey_received
--       ON public.event_mentions (community_id, pubkey_hex, received_at DESC, event_id);
--
-- A prebuilt index is skipped; otherwise it is built inline under the budgets
-- below. Either way it is then verified (docs/database.md Rule 6).
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

DO $$
BEGIN
    IF to_regclass('public.personal_read_frontiers_seq') IS NULL THEN
        CREATE INDEX personal_read_frontiers_seq
            ON public.personal_read_frontiers (community_id, actor, seq);
    END IF;
    IF to_regclass('public.idx_event_mentions_pubkey_received') IS NULL THEN
        CREATE INDEX idx_event_mentions_pubkey_received
            ON public.event_mentions (community_id, pubkey_hex, received_at DESC, event_id);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indexrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relname = 'personal_read_frontiers_seq'
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) =
              'CREATE INDEX personal_read_frontiers_seq ON public.personal_read_frontiers USING btree (community_id, actor, seq)'
    ) THEN
        RAISE EXCEPTION 'personal_read_frontiers_seq invalid or wrong definition';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indexrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relname = 'idx_event_mentions_pubkey_received'
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) =
              'CREATE INDEX idx_event_mentions_pubkey_received ON public.event_mentions USING btree (community_id, pubkey_hex, received_at DESC, event_id)'
    ) THEN
        RAISE EXCEPTION 'idx_event_mentions_pubkey_received invalid or wrong definition';
    END IF;
END $$;
