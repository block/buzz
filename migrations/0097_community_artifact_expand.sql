-- Expand only. Keep the legacy conflict target until every writer and reader
-- enforces NIP-AR community audiences. Activation requires the separate,
-- guarded operator contract in scripts/maintenance/community-artifact-contract.sql.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE artifact_heads ADD COLUMN IF NOT EXISTS scope TEXT NOT NULL DEFAULT 'channel';
ALTER TABLE artifact_heads ALTER COLUMN channel_id DROP NOT NULL;
ALTER TABLE artifact_revisions ADD COLUMN IF NOT EXISTS scope TEXT NOT NULL DEFAULT 'channel';
-- Community payloads stay outside the chat/search event store. Channel
-- receipts retain their existing event-store payload and wire behavior.
ALTER TABLE artifact_revisions ADD COLUMN IF NOT EXISTS community_event JSONB;
ALTER TABLE artifact_revisions ADD COLUMN IF NOT EXISTS redacted BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE communities ADD COLUMN IF NOT EXISTS artifact_type_policies JSONB NOT NULL DEFAULT '{}';

-- Small deployments build inline within the budget. Larger deployments may
-- prebuild these exact indexes concurrently with auto-migration disabled.
CREATE UNIQUE INDEX IF NOT EXISTS artifact_heads_scoped_identity
    ON artifact_heads (community_id, scope, artifact_id);
CREATE INDEX IF NOT EXISTS artifact_revisions_scoped_identity
    ON artifact_revisions (community_id, scope, artifact_id);

CREATE INDEX IF NOT EXISTS artifact_heads_live_type
    ON artifact_heads (community_id, scope, artifact_type, artifact_id) WHERE NOT deleted;

DO $$
BEGIN
    IF (SELECT count(*) FROM (VALUES
        ('artifact_heads','scope','text',true),
        ('artifact_heads','channel_id','uuid',false),
        ('artifact_revisions','scope','text',true),
        ('artifact_revisions','community_event','jsonb',false),
        ('artifact_revisions','redacted','boolean',true),
        ('communities','artifact_type_policies','jsonb',true)
    ) expected(rel,col,typ,required)
    JOIN pg_attribute a ON a.attrelid=to_regclass(expected.rel)
      AND a.attname=expected.col AND NOT a.attisdropped
      AND a.atttypid=to_regtype(expected.typ) AND a.attnotnull=expected.required) <> 6 THEN
        RAISE EXCEPTION 'community artifact expand column definition mismatch';
    END IF;
    IF (SELECT count(*) FROM (VALUES
        ('artifact_heads','scope', $default$'channel'::text$default$),
        ('artifact_revisions','scope', $default$'channel'::text$default$),
        ('artifact_revisions','redacted','false'),
        ('communities','artifact_type_policies', $default$'{}'::jsonb$default$)
    ) expected(rel,col,expr)
    JOIN pg_attribute a ON a.attrelid=to_regclass(expected.rel) AND a.attname=expected.col
    JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum
      AND pg_get_expr(d.adbin,d.adrelid)=expected.expr) <> 4 THEN
        RAISE EXCEPTION 'community artifact expand default definition mismatch';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i WHERE i.indexrelid='artifact_heads_live_type'::regclass
          AND i.indisvalid AND i.indisready AND NOT i.indisunique
          AND pg_get_indexdef(i.indexrelid) = 'CREATE INDEX artifact_heads_live_type ON public.artifact_heads USING btree (community_id, scope, artifact_type, artifact_id) WHERE (NOT deleted)'
    ) THEN
        RAISE EXCEPTION 'community artifact typed index definition mismatch';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        WHERE i.indexrelid='artifact_heads_scoped_identity'::regclass
          AND i.indrelid='artifact_heads'::regclass AND i.indisunique
          AND i.indisvalid AND i.indisready AND i.indpred IS NULL AND i.indexprs IS NULL
          AND pg_get_indexdef(i.indexrelid) = 'CREATE UNIQUE INDEX artifact_heads_scoped_identity ON public.artifact_heads USING btree (community_id, scope, artifact_id)'
    ) OR NOT EXISTS (
        SELECT 1 FROM pg_index i
        WHERE i.indexrelid='artifact_revisions_scoped_identity'::regclass
          AND i.indrelid='artifact_revisions'::regclass AND NOT i.indisunique
          AND i.indisvalid AND i.indisready AND i.indpred IS NULL AND i.indexprs IS NULL
          AND pg_get_indexdef(i.indexrelid) = 'CREATE INDEX artifact_revisions_scoped_identity ON public.artifact_revisions USING btree (community_id, scope, artifact_id)'
    ) THEN
        RAISE EXCEPTION 'community artifact expand index definition mismatch';
    END IF;
END $$;
