-- Attach the universal fence to one community-scoped relation. Future
-- migrations must invoke this helper explicitly after CREATE/ALTER introduces
-- community_id; the migration lint enforces that contract.
CREATE FUNCTION attach_community_write_fence(target REGCLASS) RETURNS VOID
LANGUAGE plpgsql AS $$
DECLARE
    relation_name NAME;
BEGIN
    SELECT c.relname
      INTO relation_name
      FROM pg_class c
      JOIN pg_namespace n ON n.oid = c.relnamespace
     WHERE c.oid = target
       AND n.nspname = current_schema()
       AND c.relkind IN ('r', 'p')
       AND NOT c.relispartition;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'community write fence target % is not a table in the current schema', target
            USING ERRCODE = 'wrong_object_type';
    END IF;
    IF community_write_fence_excluded_table(relation_name) THEN
        RETURN;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_attribute
         WHERE attrelid = target AND attname = 'community_id' AND NOT attisdropped
    ) THEN
        RAISE EXCEPTION 'community write fence target % has no community_id', target
            USING ERRCODE = 'undefined_column';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
         WHERE tgrelid = target
           AND tgname = 'community_write_fence_' || relation_name
           AND NOT tgisinternal
    ) THEN
        EXECUTE format(
            'CREATE TRIGGER %I BEFORE INSERT OR UPDATE OR DELETE ON %s '
            'FOR EACH ROW EXECUTE FUNCTION enforce_community_write_fence()',
            'community_write_fence_' || relation_name,
            target
        );
    END IF;
END
$$;
