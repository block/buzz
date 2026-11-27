-- Mark permanently rejected search_outbox rows as parked so they do not
-- block newer messages or pause backfill; see schema/tables/public/search_outbox.sql.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE search_outbox
    ADD COLUMN IF NOT EXISTS parked_at TIMESTAMPTZ;

DO $$
BEGIN
    IF to_regclass('public.search_outbox_active_queue') IS NULL THEN
        CREATE INDEX search_outbox_active_queue
            ON public.search_outbox (created_at)
            WHERE parked_at IS NULL;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indexrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relname = 'search_outbox_active_queue'
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) =
              'CREATE INDEX search_outbox_active_queue ON public.search_outbox USING btree (created_at) WHERE (parked_at IS NULL)'
    ) THEN
        RAISE EXCEPTION 'search_outbox_active_queue invalid or wrong definition';
    END IF;

    IF to_regclass('public.search_outbox_active_community_queue') IS NULL THEN
        CREATE INDEX search_outbox_active_community_queue
            ON public.search_outbox (community_id, created_at)
            WHERE parked_at IS NULL;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indexrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relname = 'search_outbox_active_community_queue'
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) =
              'CREATE INDEX search_outbox_active_community_queue ON public.search_outbox USING btree (community_id, created_at) WHERE (parked_at IS NULL)'
    ) THEN
        RAISE EXCEPTION 'search_outbox_active_community_queue invalid or wrong definition';
    END IF;

    IF to_regclass('public.search_outbox_parked_queue') IS NULL THEN
        CREATE INDEX search_outbox_parked_queue
            ON public.search_outbox (parked_at)
            WHERE parked_at IS NOT NULL;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indexrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relname = 'search_outbox_parked_queue'
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) =
              'CREATE INDEX search_outbox_parked_queue ON public.search_outbox USING btree (parked_at) WHERE (parked_at IS NOT NULL)'
    ) THEN
        RAISE EXCEPTION 'search_outbox_parked_queue invalid or wrong definition';
    END IF;
END $$;
