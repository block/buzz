-- Drop the foreign keys on the partitioned tables before pgschema apply.
--
-- pgschema cannot drop a foreign key from a partitioned table. Its plan drops
-- the parent's constraint, then the copy on each partition, which the first
-- drop already removed (SQLSTATE 42704). This file runs migration 0104's drop
-- for a database that pgschema brings up to date instead.
--
-- For the test launchers only: scripts/start-relay-for-tests.sh, which reuses
-- its database, and the populated-upgrade test run it before apply.
-- Deployments apply migration 0104 instead (docs/database.md, "Retiring a
-- trigger, function, or foreign key"). It does nothing on an empty database or
-- one without the keys.
--
-- As in 0104: each drop locks the table, every partition and communities, so
-- take every lock in one statement, communities last, with budgets that end
-- the attempt before the 1s deadlock_timeout. On failure nothing changes;
-- retry.
BEGIN;
SET LOCAL lock_timeout = '300ms';
SET LOCAL statement_timeout = '800ms';
DO $$
BEGIN
    IF to_regclass('events') IS NULL THEN
        RETURN;
    END IF;
    LOCK TABLE events, delivery_log, communities IN ACCESS EXCLUSIVE MODE;
    ALTER TABLE events DROP CONSTRAINT IF EXISTS events_community_id_fkey;
    ALTER TABLE delivery_log DROP CONSTRAINT IF EXISTS delivery_log_community_id_fkey;
END $$;
COMMIT;
