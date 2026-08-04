-- Older relay-generated workflow messages used their first `p` tag to record
-- the workflow owner. Because `p` is the Nostr mention tag, that derived index
-- entry routed every channel automation into the owner's Inbox. The signed
-- event remains immutable; remove only the incorrect derived mention row.
--
-- Relay workflow messages now use an `actor` tag for owner attribution. The
-- missing-actor guard limits this cleanup to the legacy representation, so
-- explicit mentions produced by the corrected relay remain indexed.
DELETE FROM event_mentions mention
USING events event
WHERE mention.community_id = event.community_id
  AND mention.event_id = event.id
  AND event.kind = 9
  AND EXISTS (
      SELECT 1
      FROM jsonb_array_elements(event.tags) tag
      WHERE jsonb_typeof(tag) = 'array'
        AND jsonb_array_length(tag) >= 2
        AND tag->>0 = 'buzz:workflow'
        AND tag->>1 = 'true'
  )
  AND NOT EXISTS (
      SELECT 1
      FROM jsonb_array_elements(event.tags) tag
      WHERE jsonb_typeof(tag) = 'array'
        AND jsonb_array_length(tag) >= 2
        AND tag->>0 = 'actor'
  )
  AND mention.pubkey_hex = (
      SELECT lower(tag->>1)
      FROM jsonb_array_elements(event.tags) WITH ORDINALITY AS item(tag, position)
      WHERE jsonb_typeof(tag) = 'array'
        AND jsonb_array_length(tag) >= 2
        AND tag->>0 = 'p'
      ORDER BY position
      LIMIT 1
  );
