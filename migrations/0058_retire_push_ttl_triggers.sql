-- Retire the event follow-up triggers. The application now enqueues push
-- match jobs and refreshes channel TTL deadlines in the event transaction
-- (crates/buzz-db/src/store/event_follow_up.rs), so these triggers only
-- repeat that work.
--
-- Dropping a trigger on the partitioned parent also drops its partition
-- clones. Published migrations 0018, 0022, 0023, 0024 and 0040 stay
-- checksum-frozen.

DROP TRIGGER events_enqueue_push_match ON events;
DROP TRIGGER events_refresh_channel_ttl ON events;

DROP FUNCTION enqueue_push_match_job();
DROP FUNCTION refresh_channel_ttl_after_event_insert();
