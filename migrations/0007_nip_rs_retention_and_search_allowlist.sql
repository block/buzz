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

-- Seed the exact NIP-33 winner (newest created_at; lowest id wins ties) from
-- both live and historical NIP-RS rows before removing any payload history.
INSERT INTO parameterized_event_watermarks
    (community_id, kind, pubkey, d_tag, created_at, event_id)
SELECT DISTINCT ON (community_id, kind, pubkey, d_tag)
       community_id, kind, pubkey, d_tag, created_at, id
FROM events
WHERE kind = 30078
  AND d_tag ~ '^read-state:[0-9a-f]{32}$'
  AND tags @> '[["t", "read-state"]]'::jsonb
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
  AND old.tags @> '[["t", "read-state"]]'::jsonb
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
  AND old.tags @> '[["t", "read-state"]]'::jsonb
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

-- Search is an explicit product surface, not the default for every stored kind.
-- Profiles, stream messages, forum posts, and forum comments are the complete
-- set requested by current profile/message search clients. New kinds must opt in
-- deliberately rather than silently indexing private or operational payloads.
ALTER TABLE events DROP COLUMN search_tsv;
ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (
    CASE WHEN kind IN (0, 9, 40002, 45001, 45003)
         THEN to_tsvector('simple', content)
         ELSE NULL::tsvector
    END
) STORED;
CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
