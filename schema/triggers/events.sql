CREATE TRIGGER events_enqueue_push_match
AFTER INSERT ON events
FOR EACH ROW EXECUTE FUNCTION enqueue_push_match_job();

CREATE CONSTRAINT TRIGGER events_refresh_channel_ttl
AFTER INSERT ON events
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION refresh_channel_ttl_after_event_insert();
CREATE TRIGGER trg_events_guard_channel_roster_snapshot
    BEFORE INSERT ON events
    FOR EACH ROW EXECUTE FUNCTION guard_channel_roster_snapshot();

SELECT attach_community_write_fence('events');
