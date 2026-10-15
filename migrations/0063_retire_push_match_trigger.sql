-- Push matching is produced by the application after event commit. Retire
-- only its compatibility trigger; channel-TTL refresh remains unchanged.
-- All event writers and rollback targets must support application enqueueing
-- before this migration runs. Published migrations remain checksum-frozen.
--
-- DROP TRIGGER locks the partitioned parent and its partitions. Bound the
-- lock acquisition wait. This does not bound statement duration or the time
-- ingestion can be blocked while these locks are held.
SET LOCAL lock_timeout = '5s';

DROP TRIGGER events_enqueue_push_match ON events;
DROP FUNCTION enqueue_push_match_job();
