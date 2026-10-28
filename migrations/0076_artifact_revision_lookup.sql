-- Populated databases prebuild this index concurrently.
-- See docs/artifact-sync-deployment.md.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';

DO $$
BEGIN
    IF to_regclass('public.idx_artifact_revisions_artifact') IS NULL THEN
        IF EXISTS (SELECT 1 FROM public.artifact_revisions LIMIT 1) THEN
            RAISE EXCEPTION 'prebuild idx_artifact_revisions_artifact concurrently; see docs/artifact-sync-deployment.md';
        END IF;
        CREATE INDEX idx_artifact_revisions_artifact
            ON public.artifact_revisions (community_id, artifact_id);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index
        WHERE indexrelid=to_regclass('public.idx_artifact_revisions_artifact')
          AND indisvalid AND indisready AND indislive
          AND pg_get_indexdef(indexrelid)=
              'CREATE INDEX idx_artifact_revisions_artifact ON public.artifact_revisions USING btree (community_id, artifact_id)'
    ) THEN
        RAISE EXCEPTION 'idx_artifact_revisions_artifact invalid or wrong definition; see docs/artifact-sync-deployment.md';
    END IF;
END $$;
