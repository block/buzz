-- Enforce NIP-RS retention across mixed relay versions.
--
-- Migration 0007 is already published and checksum-frozen. These database
-- triggers are additive so databases that applied 0007/0008 can upgrade safely,
-- while pre-PR relay binaries cannot bypass watermark or payload-retention rules.

-- Keep the invariant in PostgreSQL so it also covers pre-migration relay
-- binaries during a rolling deployment. Every conforming NIP-RS insert must
-- advance the watermark; an insert older than the greatest accepted tuple is
-- rejected even when no live row remains.
CREATE FUNCTION guard_nip_rs_watermark() RETURNS trigger AS $$
DECLARE
    advanced BOOLEAN;
BEGIN
    IF NEW.kind = 30078
       AND NEW.d_tag ~ '^read-state:[0-9a-f]{32}$'
       AND EXISTS (
           SELECT 1
           FROM jsonb_array_elements(NEW.tags) tag
           WHERE jsonb_typeof(tag) = 'array'
             AND jsonb_array_length(tag) = 2
             AND tag->>0 = 't'
             AND tag->>1 = 'read-state'
       ) THEN
        INSERT INTO parameterized_event_watermarks
            (community_id, kind, pubkey, d_tag, created_at, event_id)
        VALUES
            (NEW.community_id, NEW.kind, NEW.pubkey, NEW.d_tag, NEW.created_at, NEW.id)
        ON CONFLICT (community_id, kind, pubkey, d_tag) DO UPDATE SET
            created_at = EXCLUDED.created_at,
            event_id = EXCLUDED.event_id
        WHERE EXCLUDED.created_at > parameterized_event_watermarks.created_at
           OR (EXCLUDED.created_at = parameterized_event_watermarks.created_at
               AND EXCLUDED.event_id < parameterized_event_watermarks.event_id)
        RETURNING TRUE INTO advanced;

        IF NOT COALESCE(advanced, FALSE) THEN
            -- Exact equality is idempotent at the durable coordinate level,
            -- whether or not its payload is still live. Skip it in the trigger
            -- so concurrent physical deletion cannot create a resurrection
            -- window between an existence check and uniqueness enforcement.
            IF EXISTS (
                SELECT 1
                FROM parameterized_event_watermarks
                WHERE community_id = NEW.community_id
                  AND kind = NEW.kind
                  AND pubkey = NEW.pubkey
                  AND d_tag = NEW.d_tag
                  AND created_at = NEW.created_at
                  AND event_id = NEW.id
            ) THEN
                RETURN NULL;
            END IF;

            RAISE EXCEPTION 'stale NIP-RS event rejected by durable watermark'
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_events_nip_rs_watermark
    BEFORE INSERT ON events
    FOR EACH ROW EXECUTE FUNCTION guard_nip_rs_watermark();

-- NIP-RS payloads have no historical product value. Enforce physical removal
-- in the database when old relay binaries use their legacy soft-delete path,
-- including NIP-09 coordinate deletion during a mixed-version rollout.
CREATE FUNCTION purge_soft_deleted_nip_rs() RETURNS trigger AS $$
BEGIN
    IF OLD.deleted_at IS NULL
       AND NEW.deleted_at IS NOT NULL
       AND NEW.kind = 30078
       AND NEW.d_tag ~ '^read-state:[0-9a-f]{32}$'
       AND EXISTS (
           SELECT 1
           FROM jsonb_array_elements(NEW.tags) tag
           WHERE jsonb_typeof(tag) = 'array'
             AND jsonb_array_length(tag) = 2
             AND tag->>0 = 't'
             AND tag->>1 = 'read-state'
       ) THEN
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

CREATE TRIGGER trg_events_purge_soft_deleted_nip_rs
    AFTER UPDATE OF deleted_at ON events
    FOR EACH ROW EXECUTE FUNCTION purge_soft_deleted_nip_rs();

-- Mention indexing runs after the event transaction commits. Lock the live event
-- row while a mention is inserted so a concurrent hard delete cannot leave an
-- orphan behind; if deletion already won, silently skip the stale index row.
CREATE FUNCTION guard_event_mention_live() RETURNS trigger AS $$
BEGIN
    IF NEW.event_kind IS DISTINCT FROM 30078 THEN
        RETURN NEW;
    END IF;

    PERFORM 1
    FROM events
    WHERE community_id = NEW.community_id
      AND id = NEW.event_id
      AND created_at = NEW.event_created_at
      AND deleted_at IS NULL
    FOR KEY SHARE;

    IF NOT FOUND THEN
        RETURN NULL;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_event_mentions_require_live_event
    BEFORE INSERT ON event_mentions
    FOR EACH ROW EXECUTE FUNCTION guard_event_mention_live();
