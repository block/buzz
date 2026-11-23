-- Build within bounded startup budgets; large deployments prebuild concurrently.
-- See docs/private-media-index-deployment.md.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';
DO $migration$
BEGIN
    IF to_regclass('public.idx_events_media_edit_target') IS NULL THEN
        CREATE INDEX idx_events_media_edit_target ON public.events
            (community_id, channel_id, (decode(lower(jsonb_path_query_first(tags, 'strict $[*] ? (@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]', '{}'::jsonb, true) #>> '{}'), 'hex')), created_at DESC, id DESC)
            WHERE kind = 40003 AND deleted_at IS NULL;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        WHERE i.indexrelid = 'public.idx_events_media_edit_target'::regclass
          AND i.indrelid = 'public.events'::regclass
          AND i.indisvalid AND i.indisready AND i.indislive
          AND NOT i.indisunique AND i.indnkeyatts = 5 AND i.indnatts = 5
          AND pg_get_indexdef(i.indexrelid,1,true) = 'community_id'
          AND pg_get_indexdef(i.indexrelid,2,true) = 'channel_id'
          AND pg_get_indexdef(i.indexrelid,4,true) = 'created_at'
          AND pg_get_indexdef(i.indexrelid,5,true) = 'id'
          AND i.indoption::text = '0 0 0 3 3'
          AND pg_get_expr(i.indexprs, i.indrelid) = $expr$decode(lower((jsonb_path_query_first(tags, 'strict $[*]?(@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]'::jsonpath, '{}'::jsonb, true) #>> '{}'::text[])), 'hex'::text)$expr$
          AND pg_get_expr(i.indpred, i.indrelid) = '((kind = 40003) AND (deleted_at IS NULL))'
          AND (SELECT amname FROM pg_am WHERE oid=(SELECT relam FROM pg_class WHERE oid=i.indexrelid))='btree'
    ) THEN
        RAISE EXCEPTION 'idx_events_media_edit_target invalid or wrong definition; see docs/private-media-index-deployment.md';
    END IF;
END $migration$;
