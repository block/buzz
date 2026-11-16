-- Global upload-evidence indexes for offline user media cleanup.
-- Additive and compatible with existing relay writers. Large installations can
-- prebuild these indexes CONCURRENTLY; see docs/operator-user-deletion-executor.md.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';
DO $$
BEGIN
    IF to_regclass('public.idx_media_uploaders_actor_hash') IS NULL THEN
        CREATE INDEX idx_media_uploaders_actor_hash ON public.media_uploaders (pubkey, sha256);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        WHERE i.indexrelid = to_regclass('public.idx_media_uploaders_actor_hash')
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) = 'CREATE INDEX idx_media_uploaders_actor_hash ON public.media_uploaders USING btree (pubkey, sha256)'
    ) THEN
        RAISE EXCEPTION 'idx_media_uploaders_actor_hash invalid or wrong definition; see docs/operator-user-deletion-executor.md';
    END IF;
    IF to_regclass('public.idx_media_uploaders_hash_actor') IS NULL THEN
        CREATE INDEX idx_media_uploaders_hash_actor ON public.media_uploaders (sha256, pubkey);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        WHERE i.indexrelid = to_regclass('public.idx_media_uploaders_hash_actor')
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) = 'CREATE INDEX idx_media_uploaders_hash_actor ON public.media_uploaders USING btree (sha256, pubkey)'
    ) THEN
        RAISE EXCEPTION 'idx_media_uploaders_hash_actor invalid or wrong definition; see docs/operator-user-deletion-executor.md';
    END IF;
    IF to_regclass('public.idx_audit_media_actor_hash') IS NULL THEN
        CREATE INDEX idx_audit_media_actor_hash ON public.audit_log (actor_pubkey, object_id) WHERE action = 'media_uploaded';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        WHERE i.indexrelid = to_regclass('public.idx_audit_media_actor_hash')
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) = 'CREATE INDEX idx_audit_media_actor_hash ON public.audit_log USING btree (actor_pubkey, object_id) WHERE ((action)::text = ''media_uploaded''::text)'
    ) THEN
        RAISE EXCEPTION 'idx_audit_media_actor_hash invalid or wrong definition; see docs/operator-user-deletion-executor.md';
    END IF;
    IF to_regclass('public.idx_audit_media_hash_actor') IS NULL THEN
        CREATE INDEX idx_audit_media_hash_actor ON public.audit_log (object_id, actor_pubkey) WHERE action = 'media_uploaded';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        WHERE i.indexrelid = to_regclass('public.idx_audit_media_hash_actor')
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) = 'CREATE INDEX idx_audit_media_hash_actor ON public.audit_log USING btree (object_id, actor_pubkey) WHERE ((action)::text = ''media_uploaded''::text)'
    ) THEN
        RAISE EXCEPTION 'idx_audit_media_hash_actor invalid or wrong definition; see docs/operator-user-deletion-executor.md';
    END IF;
END $$;
