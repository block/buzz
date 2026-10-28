-- Rebuild events_pkey in its declared column order.
--
-- pgschema creates events with the partition key moved to the front of its
-- primary key, and every later plan then tries to drop and re-add the inherited
-- key on each partition, which fails (SQLSTATE 42P16).
-- scripts/reconcile-schema-after-pgschema.sql repairs that after every apply
-- and carries a verbatim copy of the block below. A reused database reconciled
-- before that repair existed still fails its next apply, before the reconcile
-- script can run, so scripts/start-relay-for-tests.sh, the one launcher that
-- reuses its database, runs this file before pgschema apply. It does nothing on
-- an empty database or one whose key is already in order.
--
-- Both copies are temporary. The reconcile block, and the test that keeps the
-- copies identical, can go once pgschema creates the key in declared order
-- (1.13.3 does). This file and the launcher step can go once no local database
-- reconciled before the reconcile block existed remains.

DO $$
DECLARE
    declared CONSTANT text := 'PRIMARY KEY (community_id, created_at, id)';
BEGIN
    -- Also runs before pgschema apply, when events may not exist yet.
    IF to_regclass('events') IS NULL THEN
        RETURN;
    END IF;

    IF (
        SELECT pg_get_constraintdef(oid) FROM pg_constraint
        WHERE conrelid = 'events'::regclass AND contype = 'p'
    ) IS DISTINCT FROM declared THEN
        -- The rebuild locks events and every partition ACCESS EXCLUSIVE. Fail
        -- fast instead of queueing behind a relay that still uses them.
        BEGIN
            PERFORM set_config('lock_timeout', '10s', true);
            ALTER TABLE events DROP CONSTRAINT IF EXISTS events_pkey;
            ALTER TABLE events
                ADD CONSTRAINT events_pkey PRIMARY KEY (community_id, created_at, id);
        EXCEPTION WHEN lock_not_available THEN
            RAISE EXCEPTION 'timed out locking events to rebuild events_pkey'
                USING HINT = 'Stop any relay using this database, then rerun.';
        END;
    END IF;

    IF EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE contype = 'p'
          AND conrelid IN (
              SELECT 'events'::regclass
              UNION ALL
              SELECT inhrelid FROM pg_inherits WHERE inhparent = 'events'::regclass
          )
          AND pg_get_constraintdef(oid) <> declared
    ) THEN
        RAISE EXCEPTION 'events and its partitions must have %', declared;
    END IF;
END $$;
