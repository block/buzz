-- Undo 0097 (community artifact expand), step 1 of 3. The community-scoped
-- artifact feature that needed 0097 was reverted (#9689), and no release after
-- the revert reads or writes the columns 0097 added. 0097 stays in place
-- because databases that auto-migrated have recorded its checksum.
--
-- Required stop: every relay pod and job must run #9689 or a later release
-- before 0100 runs. Relays built from #9650 itself read artifact_heads.scope
-- and write with ON CONFLICT on artifact_heads_scoped_identity.
--
-- This step fails closed on a database whose artifact head key is not the
-- channel key. Only a database that ran the removed operator contract script,
-- or was bootstrapped from #9650's declared schema, has the scoped key. Such a
-- database needs the separate recovery step in #9689 first.
--
-- Then it starts restoring NOT NULL on artifact_heads.channel_id without a
-- table scan under ACCESS EXCLUSIVE: 0099 validates this check, and 0100 sets
-- NOT NULL from it.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

DO $$
BEGIN
    IF (SELECT array_agg(a.attname::text ORDER BY k.ordinality)
          FROM pg_constraint c
          CROSS JOIN LATERAL unnest(c.conkey) WITH ORDINALITY AS k(attnum, ordinality)
          JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum
         WHERE c.conrelid = 'artifact_heads'::regclass AND c.contype = 'p')
       IS DISTINCT FROM ARRAY['community_id', 'artifact_id'] THEN
        RAISE EXCEPTION 'artifact_heads primary key is not (community_id, artifact_id); recover this database as described in #9689 before migrating';
    END IF;
END $$;

ALTER TABLE artifact_heads
    ADD CONSTRAINT artifact_heads_channel_id_not_null CHECK (channel_id IS NOT NULL) NOT VALID;
