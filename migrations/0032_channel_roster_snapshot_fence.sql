-- Prevent mixed-version relay pods from publishing a stale NIP-29 member
-- snapshot after a newer canonical roster has been committed.
--
-- Old binaries already serialize kind 39002 replacement on the replacement
-- advisory key. This trigger adds the channel-membership key at INSERT time,
-- after that canonical key, and validates every p tag against the current
-- active membership set. New binaries take both keys in the same order before
-- capture and replacement. Thus old and new writers remain compatible during
-- a rolling deploy.
CREATE OR REPLACE FUNCTION guard_channel_roster_snapshot()
RETURNS TRIGGER AS $$
DECLARE
    canonical_members BYTEA[];
    snapshot_members BYTEA[];
BEGIN
    IF NEW.kind <> 39002 OR NEW.channel_id IS NULL THEN
        RETURN NEW;
    END IF;

    PERFORM pg_advisory_xact_lock(hashtextextended(
        'buzz_channel_membership:' || NEW.community_id::text || ':' || NEW.channel_id::text,
        0
    ));

    SELECT COALESCE(array_agg(cm.pubkey ORDER BY cm.pubkey), ARRAY[]::BYTEA[])
      INTO canonical_members
      FROM channel_members cm
     WHERE cm.community_id = NEW.community_id
       AND cm.channel_id = NEW.channel_id
       AND cm.removed_at IS NULL;

    -- A roster is canonical only when every p tag is a 32-byte hex pubkey,
    -- no member is duplicated, and the normalized tag set exactly matches the
    -- active membership set. Ignoring malformed tags would let an otherwise
    -- complete roster smuggle invalid discovery data through the fence.
    IF EXISTS (
        SELECT 1
          FROM jsonb_array_elements(NEW.tags) tag
         WHERE tag->>0 = 'p'
           AND (jsonb_array_length(tag) < 2 OR COALESCE(tag->>1, '') !~ '^[0-9a-fA-F]{64}$')
    ) THEN
        RAISE EXCEPTION 'kind 39002 roster contains an invalid p tag'
            USING ERRCODE = '23514';
    END IF;

    SELECT COALESCE(array_agg(member ORDER BY member), ARRAY[]::BYTEA[])
      INTO snapshot_members
      FROM (
          SELECT decode(tag->>1, 'hex') AS member
            FROM jsonb_array_elements(NEW.tags) tag
           WHERE tag->>0 = 'p'
      ) snapshot;

    IF snapshot_members IS DISTINCT FROM canonical_members THEN
        RAISE EXCEPTION 'kind 39002 roster does not match canonical channel membership'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_events_guard_channel_roster_snapshot ON events;
CREATE TRIGGER trg_events_guard_channel_roster_snapshot
    BEFORE INSERT ON events
    FOR EACH ROW EXECUTE FUNCTION guard_channel_roster_snapshot();
