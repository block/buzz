-- Mesh status is a heartbeat carried in a reserved kind:30003 coordinate.
-- Only the live head has product value; retaining every superseded 45-second
-- payload creates unbounded physical history. Clean up existing history and
-- cover soft-delete writes from older relay binaries during rolling deploys.

DELETE FROM event_mentions mention
USING events status
WHERE mention.community_id = status.community_id
  AND mention.event_id = status.id
  AND status.kind = 30003
  AND status.d_tag LIKE 'buzz-mesh-member-status:%'
  AND status.deleted_at IS NOT NULL
  AND status.tags @> '[["k", "buzz-mesh-status"]]'::jsonb;

DELETE FROM events
WHERE kind = 30003
  AND d_tag LIKE 'buzz-mesh-member-status:%'
  AND deleted_at IS NOT NULL
  AND tags @> '[["k", "buzz-mesh-status"]]'::jsonb;

CREATE FUNCTION purge_soft_deleted_buzz_mesh_status() RETURNS trigger AS $$
BEGIN
    IF OLD.deleted_at IS NULL
       AND NEW.deleted_at IS NOT NULL
       AND NEW.kind = 30003
       AND NEW.d_tag LIKE 'buzz-mesh-member-status:%'
       AND NEW.tags @> '[["k", "buzz-mesh-status"]]'::jsonb THEN
        DELETE FROM events
        WHERE community_id = NEW.community_id
          AND created_at = NEW.created_at
          AND id = NEW.id;

        DELETE FROM event_mentions
        WHERE community_id = NEW.community_id AND event_id = NEW.id;
    END IF;

    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_events_purge_soft_deleted_buzz_mesh_status
    AFTER UPDATE OF deleted_at ON events
    FOR EACH ROW EXECUTE FUNCTION purge_soft_deleted_buzz_mesh_status();
