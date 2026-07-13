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
