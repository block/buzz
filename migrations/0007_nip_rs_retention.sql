-- Bound NIP-RS storage while preserving NIP-33 replay ordering.
--
-- The payload table previously retained every superseded kind:30078 event as a
-- soft-deleted row. Besides keeping the encrypted blob, search_tsv tokenized it
-- and the GIN index amplified it further. A compact ordering watermark retains
-- the only historical fact replacement needs without retaining user payloads.
-- The relay may still have old instances writing during a rolling deploy. Hold a
-- table-level writer lock for this transaction so the seed is a complete
-- high-water mark: without it, an old instance could insert between the seed
-- and purge, then a later NIP-09 deletion could reopen a replay window. Reads
-- remain available; inserts, updates, and deletes wait for migration commit.
LOCK TABLE events IN SHARE ROW EXCLUSIVE MODE;

CREATE TABLE parameterized_event_watermarks (
    community_id  UUID NOT NULL REFERENCES communities(id),
    kind          INT NOT NULL,
    pubkey        BYTEA NOT NULL,
    d_tag         TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL,
    event_id      BYTEA NOT NULL,
    PRIMARY KEY (community_id, kind, pubkey, d_tag)
);

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
            -- Let an exact duplicate reach the events uniqueness constraint so
            -- legacy `ON CONFLICT DO NOTHING` keeps its existing idempotence.
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
                RETURN NEW;
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
        DELETE FROM event_mentions
        WHERE community_id = NEW.community_id AND event_id = NEW.id;

        DELETE FROM events
        WHERE community_id = NEW.community_id
          AND created_at = NEW.created_at
          AND id = NEW.id;
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

-- Superseded read-state events normally have no p-tags, but malformed/legacy
-- rows can. Serve defensive mention cleanup without a per-replacement seq scan.
CREATE INDEX idx_event_mentions_community_event
    ON event_mentions (community_id, event_id);

-- Fail closed on legacy anomalies that would make a deleted tuple outrank a
-- live head. Seeding that tuple would freeze legitimate writes; ignoring it
-- would weaken replay protection. Operators must inspect and repair such a
-- coordinate before retrying the migration.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM events dead
        JOIN LATERAL (
            SELECT live.created_at, live.id
            FROM events live
            WHERE live.community_id = dead.community_id
              AND live.kind = dead.kind
              AND live.pubkey = dead.pubkey
              AND live.d_tag = dead.d_tag
              AND live.deleted_at IS NULL
            ORDER BY live.created_at DESC, live.id ASC
            LIMIT 1
        ) live ON TRUE
        WHERE dead.kind = 30078
          AND dead.deleted_at IS NOT NULL
          AND dead.d_tag ~ '^read-state:[0-9a-f]{32}$'
          AND EXISTS (
              SELECT 1
              FROM jsonb_array_elements(dead.tags) tag
              WHERE jsonb_typeof(tag) = 'array'
                AND jsonb_array_length(tag) = 2
                AND tag->>0 = 't'
                AND tag->>1 = 'read-state'
          )
          AND (dead.created_at > live.created_at
               OR (dead.created_at = live.created_at AND dead.id < live.id))
    ) THEN
        RAISE EXCEPTION 'NIP-RS retention blocked: deleted event outranks live head';
    END IF;
END $$;

-- Seed the greatest accepted tuple (newest created_at; lowest id wins ties)
-- from live and historical NIP-RS rows before removing payload history.
INSERT INTO parameterized_event_watermarks
    (community_id, kind, pubkey, d_tag, created_at, event_id)
SELECT DISTINCT ON (community_id, kind, pubkey, d_tag)
       community_id, kind, pubkey, d_tag, created_at, id
FROM events e
WHERE kind = 30078
  AND d_tag ~ '^read-state:[0-9a-f]{32}$'
  AND EXISTS (
      SELECT 1
      FROM jsonb_array_elements(e.tags) tag
      WHERE jsonb_typeof(tag) = 'array'
        AND jsonb_array_length(tag) = 2
        AND tag->>0 = 't'
        AND tag->>1 = 'read-state'
  )
ORDER BY community_id, kind, pubkey, d_tag, created_at DESC, id ASC;

-- Mentions are denormalized and do not have a foreign key to the partitioned
-- events table. Delete any defensive/legacy rows for the exact purge set first.
DELETE FROM event_mentions mention
USING events old
WHERE mention.community_id = old.community_id
  AND mention.event_id = old.id
  AND mention.event_created_at = old.created_at
  AND old.kind = 30078
  AND old.deleted_at IS NOT NULL
  AND old.d_tag ~ '^read-state:[0-9a-f]{32}$'
  AND EXISTS (
      SELECT 1
      FROM jsonb_array_elements(old.tags) tag
      WHERE jsonb_typeof(tag) = 'array'
        AND jsonb_array_length(tag) = 2
        AND tag->>0 = 't'
        AND tag->>1 = 'read-state'
  )
  AND EXISTS (
      SELECT 1
      FROM events live
      WHERE live.community_id = old.community_id
        AND live.kind = old.kind
        AND live.pubkey = old.pubkey
        AND live.d_tag = old.d_tag
        AND live.deleted_at IS NULL
        AND (live.created_at > old.created_at
             OR (live.created_at = old.created_at AND live.id < old.id))
  );

-- Purge only replacement history with a strictly dominating live head. Rows
-- deleted explicitly through NIP-09 have no live head and remain untouched.
DELETE FROM events old
WHERE old.kind = 30078
  AND old.deleted_at IS NOT NULL
  AND old.d_tag ~ '^read-state:[0-9a-f]{32}$'
  AND EXISTS (
      SELECT 1
      FROM jsonb_array_elements(old.tags) tag
      WHERE jsonb_typeof(tag) = 'array'
        AND jsonb_array_length(tag) = 2
        AND tag->>0 = 't'
        AND tag->>1 = 'read-state'
  )
  AND EXISTS (
      SELECT 1
      FROM events live
      WHERE live.community_id = old.community_id
        AND live.kind = old.kind
        AND live.pubkey = old.pubkey
        AND live.d_tag = old.d_tag
        AND live.deleted_at IS NULL
        AND (live.created_at > old.created_at
             OR (live.created_at = old.created_at AND live.id < old.id))
  );
