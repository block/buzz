-- Kind:44620 must have a raw NULL search vector, including malformed payloads.
-- Keep generated storage: no trigger/replication/restore maintenance contract and
-- no row UPDATE that could cross a community deletion fence. PG17 SET EXPRESSION
-- preserves the column and its dependent indexes, but STILL rewrites heaps and
-- indexes. Only installations needing correction pay that cost. See
-- docs/workflow-wake-fts-rollout.md before upgrading a populated legacy database.
DO $$
DECLARE
    existing_expression TEXT;
    partition_expression TEXT;
    safe_expressions TEXT[];
    relation RECORD;
    previous_lock_timeout TEXT := current_setting('lock_timeout');
BEGIN
    -- Bound lock acquisition, not the rewrite duration. Hold the entire tree
    -- stable while inspecting it, including against partition attach/detach.
    PERFORM set_config('lock_timeout', '5s', true);
    LOCK TABLE events IN ACCESS EXCLUSIVE MODE;

    -- Ask PostgreSQL to canonicalize the known safe policies. Comparing whole
    -- expressions is deliberate: a substring or an empty-content probe cannot
    -- prove NULL for every possible private payload. Unknown policies are not
    -- guessed safe. The temporary relation contains no event data.
    CREATE TEMP TABLE workflow_wake_safe_fts (
        kind INT,
        content TEXT,
        allowlist TSVECTOR GENERATED ALWAYS AS (
            CASE WHEN kind IN (0, 9, 40002, 45001, 45003)
                 THEN to_tsvector('simple', content) ELSE NULL::tsvector END
        ) STORED,
        migrated_allowlist TSVECTOR GENERATED ALWAYS AS (
            CASE WHEN kind = 30179 THEN NULL::tsvector ELSE (
                CASE WHEN kind = 30350 THEN NULL::tsvector ELSE (
                    CASE WHEN kind IN (0, 9, 40002, 45001, 45003)
                         THEN to_tsvector('simple', content) ELSE NULL::tsvector END
                ) END
            ) END
        ) STORED,
        desired_policy TSVECTOR GENERATED ALWAYS AS (
            CASE WHEN kind IN (1059, 30179, 30300, 30350, 30622, 44100, 44101, 44200, 44620)
                 THEN NULL::tsvector ELSE to_tsvector('simple', content) END
        ) STORED
    ) ON COMMIT DROP;
    SELECT array_agg(pg_get_expr(adbin, adrelid)) INTO safe_expressions
      FROM pg_attrdef WHERE adrelid = 'pg_temp.workflow_wake_safe_fts'::regclass;

    SELECT pg_get_expr(d.adbin, d.adrelid) INTO existing_expression
      FROM pg_attribute a JOIN pg_attrdef d
        ON d.adrelid = a.attrelid AND d.adnum = a.attnum
     WHERE a.attrelid = 'events'::regclass AND a.attname = 'search_tsv'
       AND a.attgenerated = 's';
    IF existing_expression IS NULL THEN
        RAISE EXCEPTION 'events.search_tsv must be a stored generated column';
    END IF;

    FOR relation IN SELECT relid FROM pg_partition_tree('events'::regclass) LOOP
        SELECT pg_get_expr(d.adbin, d.adrelid) INTO partition_expression
          FROM pg_attribute a JOIN pg_attrdef d
            ON d.adrelid = a.attrelid AND d.adnum = a.attnum
         WHERE a.attrelid = relation.relid AND a.attname = 'search_tsv'
           AND a.attgenerated = 's';
        IF partition_expression IS DISTINCT FROM existing_expression THEN
            RAISE EXCEPTION 'divergent search_tsv policy on %; reconcile partition policy before upgrading', relation.relid::regclass;
        END IF;
    END LOOP;

    IF NOT (existing_expression = ANY(safe_expressions)) THEN
        -- Preserve every non-wake kind's existing policy, signed event fields,
        -- column identity, privileges and dependent objects. DDL recomputation
        -- does not replay row UPDATE triggers or bypass their deletion fences.
        EXECUTE format(
            'ALTER TABLE events ALTER COLUMN search_tsv SET EXPRESSION AS (CASE WHEN kind = 44620 THEN NULL::tsvector ELSE (%s) END)',
            existing_expression
        );
    END IF;
    DROP TABLE pg_temp.workflow_wake_safe_fts;
    PERFORM set_config('lock_timeout', previous_lock_timeout, true);
END $$;
