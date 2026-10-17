-- NIP-AR current heads and acceptance ledger are independent of event retention.
CREATE TABLE artifact_heads (
    community_id UUID NOT NULL REFERENCES communities(id),
    artifact_id UUID NOT NULL,
    event_id BYTEA NOT NULL CHECK (length(event_id) = 32),
    channel_id UUID NOT NULL,
    artifact_type TEXT NOT NULL,
    root BYTEA,
    deleted BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (community_id, artifact_id)
);
CREATE INDEX artifact_heads_event ON artifact_heads (community_id, event_id);

-- The relay does not expire events. Any future row retention or partition
-- retirement must skip payloads referenced by `artifact_heads.event_id`
-- (NIP-AR: expiring earlier revisions MUST NOT remove the current revision).
