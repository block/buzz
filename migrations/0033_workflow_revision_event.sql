-- Cross-author workflow revisions use the accepted request event id instead
-- of the NIP-33 author coordinate as their compare-and-swap token.
SET LOCAL lock_timeout = '5s';

ALTER TABLE workflows ADD COLUMN revision_event_id BYTEA;

WITH live_definitions AS (
    SELECT DISTINCT ON (community_id, pubkey, d_tag)
        community_id,
        pubkey,
        d_tag,
        id
    FROM events
    WHERE kind = 30620
      AND deleted_at IS NULL
      AND d_tag IS NOT NULL
    ORDER BY community_id, pubkey, d_tag, created_at DESC, id ASC
)
UPDATE workflows AS workflow
SET revision_event_id = definition.id
FROM live_definitions AS definition
WHERE definition.community_id = workflow.community_id
  AND definition.pubkey = workflow.owner_pubkey
  AND definition.d_tag = workflow.id::text;

ALTER TABLE workflows
    ADD CONSTRAINT workflows_revision_event_id_length
    CHECK (revision_event_id IS NULL OR octet_length(revision_event_id) = 32);
