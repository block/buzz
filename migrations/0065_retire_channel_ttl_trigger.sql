-- Channel-TTL refresh is done by the application in the event transaction
-- (crates/buzz-db/src/store/event_follow_up.rs). Retire the trigger that
-- repeats that work, completing the follow-up trigger retirement begun by
-- 0063. All event writers and rollback targets must support application TTL
-- refresh before this migration runs. Published migrations remain
-- checksum-frozen.
--
-- DROP TRIGGER locks the partitioned parent and its partitions. Bound the
-- lock acquisition wait. This does not bound statement duration or the time
-- ingestion can be blocked while these locks are held.
SET LOCAL lock_timeout = '5s';

DROP TRIGGER events_refresh_channel_ttl ON events;
DROP FUNCTION refresh_channel_ttl_after_event_insert();
