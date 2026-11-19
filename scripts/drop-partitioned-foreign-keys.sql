-- Drop the foreign keys on the partitioned tables before pgschema apply.
--
-- pgschema cannot drop a foreign key from a partitioned table. Its plan drops
-- the parent's constraint, then the copy on each partition, which the first
-- drop already removed (SQLSTATE 42704). Migration 0104 drops these two keys;
-- this file does the same for a database that pgschema brings up to date
-- instead. scripts/start-relay-for-tests.sh, the one launcher that reuses its
-- database, runs it before apply. It does nothing on an empty database or one
-- without the keys.
DO $$
BEGIN
    IF to_regclass('events') IS NULL THEN
        RETURN;
    END IF;
    -- Each drop locks the table, every partition and communities.
    PERFORM set_config('lock_timeout', '10s', true);
    ALTER TABLE events DROP CONSTRAINT IF EXISTS events_community_id_fkey;
    ALTER TABLE IF EXISTS delivery_log
        DROP CONSTRAINT IF EXISTS delivery_log_community_id_fkey;
END $$;
