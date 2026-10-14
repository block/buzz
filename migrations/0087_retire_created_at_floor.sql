SET LOCAL lock_timeout = '5s';

-- DESTRUCTIVE CONTRACT: deploy phase 1's application floor check to every
-- writer pod/job first, with no rollback expected. Stop out-of-band writers
-- and hold the replica breaker closed through this drop until replay catches
-- up. Old pods that already started do not recheck the guard on each probe.
-- Keep the 960-second application floor and the ordered heartbeat handshake.
-- No other trigger, function, FK, or tenant fence is retired here.
DROP TRIGGER IF EXISTS events_created_at_floor ON events;

-- Dropping the parent removes its inherited clones (including partitions
-- created since 0021). Also remove any independently attached child copy.
DO $$
DECLARE
    partition_table regclass;
BEGIN
    FOR partition_table IN
        SELECT relid FROM pg_partition_tree('events'::regclass)
        WHERE relid <> 'events'::regclass
    LOOP
        EXECUTE format('DROP TRIGGER IF EXISTS events_created_at_floor ON %s', partition_table);
    END LOOP;
END
$$;

DROP FUNCTION IF EXISTS events_created_at_floor_guard();
