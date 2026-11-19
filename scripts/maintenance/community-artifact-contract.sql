-- OPERATOR CONTRACT, never an automatic migration. Read
-- docs/community-artifact-rollout.md before running this file.
-- The calling session must explicitly attest that all serving backends,
-- deletion executors and operator tools run the compatible audience bridge:
-- SET buzz.community_artifact_contract_ready = 'all-backends-enforce-community-audience';
BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
DO $$
DECLARE
    current_key text[];
BEGIN
    IF current_setting('buzz.community_artifact_contract_ready', true)
       IS DISTINCT FROM 'all-backends-enforce-community-audience' THEN
        RAISE EXCEPTION 'community artifact contract requires explicit fleet compatibility attestation';
    END IF;
    SELECT array_agg(a.attname::text ORDER BY k.ord) INTO current_key
      FROM pg_index i
      CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY k(num,ord)
      JOIN pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=k.num
     WHERE i.indrelid='artifact_heads'::regclass AND i.indisprimary
       AND i.indisvalid AND i.indisready AND i.indpred IS NULL AND i.indexprs IS NULL;
    IF current_key = ARRAY['community_id','scope','artifact_id'] THEN
        RETURN; -- Already contracted; safe to rerun without touching payloads.
    END IF;
    IF current_key IS DISTINCT FROM ARRAY['community_id','artifact_id'] THEN
        RAISE EXCEPTION 'unexpected artifact head primary key';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
         WHERE i.indexrelid=to_regclass('artifact_heads_scoped_identity')
           AND i.indrelid='artifact_heads'::regclass AND i.indisunique
           AND i.indisvalid AND i.indisready AND i.indpred IS NULL AND i.indexprs IS NULL
           AND pg_get_indexdef(i.indexrelid) = 'CREATE UNIQUE INDEX artifact_heads_scoped_identity ON public.artifact_heads USING btree (community_id, scope, artifact_id)'
    ) THEN
        RAISE EXCEPTION 'missing or incompatible expanded artifact identity index';
    END IF;
    -- Atomic: no interval without a primary key. All key columns are already
    -- NOT NULL; attaching the prebuilt index needs no table rewrite or backfill.
    ALTER TABLE artifact_heads DROP CONSTRAINT artifact_heads_pkey,
        ADD CONSTRAINT artifact_heads_pkey PRIMARY KEY USING INDEX artifact_heads_scoped_identity;
END $$;
COMMIT;
