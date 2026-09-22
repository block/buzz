-- Newest-first thread keyset index. Additive: legacy root/parent indexes remain.
-- Brownfield operators MUST prebuild concurrently as documented in NIP-TW.
-- Startup is deliberately bounded: a busy/large table fails deployment rather
-- than blocking ingestion for an unbounded index build. Retry after prebuild.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';
CREATE INDEX IF NOT EXISTS idx_thread_metadata_window
    ON thread_metadata (community_id, root_event_id, event_created_at DESC, event_id ASC);

-- IF NOT EXISTS alone would accept a failed concurrent build, or a same-name
-- index with the wrong order. Neither is a successful deployment.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indexrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relname = 'idx_thread_metadata_window'
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) =
              'CREATE INDEX idx_thread_metadata_window ON public.thread_metadata USING btree (community_id, root_event_id, event_created_at DESC, event_id)'
    ) THEN
        RAISE EXCEPTION 'idx_thread_metadata_window invalid or wrong definition; see NIP-TW index deployment';
    END IF;
END $$;
