-- Populated databases prebuild these indexes concurrently after migration 0070.
-- See docs/artifact-sync-deployment.md.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';

DO $$
DECLARE
    item RECORD;
    populated BOOLEAN;
BEGIN
    FOR item IN SELECT * FROM (VALUES
        ('artifact_heads_modified', 'artifact_heads', '(community_id, modified_at, artifact_id)', ''),
        ('artifact_heads_channel', 'artifact_heads', '(community_id, channel_id, artifact_id)', ''),
        ('channel_members_sync', 'channel_members', '(community_id, pubkey, sync_modified_at, channel_id)', ''),
        ('artifact_revisions_moves', 'artifact_revisions', '(community_id, modified_at, event_id)', ' WHERE source_channel_id IS NOT NULL')
    ) AS indexes(name, table_name, columns, predicate)
    LOOP
        IF to_regclass('public.' || item.name) IS NULL THEN
            EXECUTE format('SELECT EXISTS(SELECT 1 FROM public.%I LIMIT 1)',item.table_name) INTO populated;
            IF populated THEN
                RAISE EXCEPTION 'prebuild % concurrently; see docs/artifact-sync-deployment.md', item.name;
            END IF;
            EXECUTE format('CREATE INDEX %I ON public.%I %s%s', item.name, item.table_name, item.columns, item.predicate);
        END IF;
        IF NOT EXISTS (
            SELECT 1 FROM pg_index WHERE indexrelid=to_regclass('public.' || item.name)
                AND indisvalid AND indisready AND indislive
                AND pg_get_indexdef(indexrelid)=format('CREATE INDEX %s ON public.%s USING btree %s%s',
                    item.name,item.table_name,item.columns,
                    CASE WHEN item.predicate='' THEN '' ELSE ' WHERE (source_channel_id IS NOT NULL)' END)
        ) THEN
            RAISE EXCEPTION '% is invalid or has a different definition', item.name;
        END IF;
    END LOOP;
END $$;
