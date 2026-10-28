-- Read-state access paths: one index seek answers "newest countable message
-- here" and "what arrived after this position" for a channel timeline or a
-- thread, and "my mentions in this channel since then". Arrival order
-- (received_at), not author time. No backfill: rows written before this
-- migration are not on the new paths.
--
-- events.read_timeline: an eligible message that counts on its channel
-- timeline (top-level, or a broadcast reply), set at insert.
-- thread_metadata.received_at: a reply's arrival, set at insert; NULL on root
-- stubs and earlier rows.
-- personal_read_frontiers.unfollowed: a sticky unfollow of a thread.
-- personal_read_frontiers.explicit: a follow intent, never dormant.
-- personal_read_frontiers.active_at: the actor's last involvement in a
-- thread; an implicit follow is dormant once it is older than the relay's
-- window. Earlier rows start at migration time.
--
-- Brownfield operators MUST add the five columns and prebuild the four
-- indexes concurrently as documented in docs/buzz-v1-read-state.md. Startup is
-- bounded like 0049: a large table fails deployment rather than blocking
-- ingestion.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';

DO $$
DECLARE
    spec RECORD;
BEGIN
    -- Even ADD COLUMN IF NOT EXISTS requests a writer-conflicting lock: skip
    -- a column an operator added beforehand.
    FOR spec IN SELECT * FROM (VALUES
        ('events', 'read_timeline', 'BOOLEAN NOT NULL DEFAULT false'),
        ('thread_metadata', 'received_at', 'TIMESTAMPTZ'),
        ('personal_read_frontiers', 'unfollowed',
         $d$BOOLEAN NOT NULL DEFAULT false CHECK (NOT unfollowed OR root_id <> ''::bytea)$d$),
        ('personal_read_frontiers', 'explicit', 'BOOLEAN NOT NULL DEFAULT false'),
        ('personal_read_frontiers', 'active_at', 'TIMESTAMPTZ NOT NULL DEFAULT now()')
    ) AS s(tbl, col, definition) LOOP
        IF NOT EXISTS (
            SELECT 1 FROM pg_attribute
            WHERE attrelid = ('public.' || spec.tbl)::regclass
              AND attname = spec.col AND NOT attisdropped
        ) THEN
            EXECUTE format('ALTER TABLE public.%I ADD COLUMN %I %s',
                spec.tbl, spec.col, spec.definition);
        END IF;
    END LOOP;

    -- A partitioned parent's definition reads ON ONLY; CREATE recurses to
    -- every partition regardless.
    FOR spec IN SELECT * FROM (VALUES
        ('idx_events_read_timeline',
         'CREATE INDEX idx_events_read_timeline ON ONLY public.events USING btree (community_id, channel_id, received_at DESC, id) WHERE (read_timeline AND (deleted_at IS NULL))'),
        ('idx_thread_metadata_read_arrival',
         'CREATE INDEX idx_thread_metadata_read_arrival ON public.thread_metadata USING btree (community_id, root_event_id, received_at DESC, event_id) WHERE (received_at IS NOT NULL)'),
        ('idx_event_mentions_pubkey_channel_created',
         'CREATE INDEX idx_event_mentions_pubkey_channel_created ON public.event_mentions USING btree (community_id, pubkey_hex, channel_id, event_created_at DESC)'),
        ('idx_personal_read_frontiers_active',
         $d$CREATE INDEX idx_personal_read_frontiers_active ON public.personal_read_frontiers USING btree (community_id, actor, active_at DESC) WHERE (root_id <> '\x'::bytea)$d$)
    ) AS s(name, definition) LOOP
        -- IF NOT EXISTS still requests a writer-conflicting lock: skip CREATE
        -- for a prebuilt index, then validate it.
        IF to_regclass('public.' || spec.name) IS NULL THEN
            EXECUTE replace(spec.definition, ' ON ONLY ', ' ON ');
        END IF;
        IF NOT EXISTS (
            SELECT 1 FROM pg_index i
            JOIN pg_class c ON c.oid = i.indexrelid
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = 'public' AND c.relname = spec.name
              AND i.indisvalid AND i.indisready AND i.indislive
              AND pg_get_indexdef(i.indexrelid) = spec.definition
        ) THEN
            RAISE EXCEPTION '% invalid or wrong definition; see docs/buzz-v1-read-state.md', spec.name;
        END IF;
    END LOOP;
END $$;
