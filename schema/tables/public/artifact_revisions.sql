-- Every accepted revision ID, so replays stay idempotent after redaction or
-- retention.
CREATE TABLE artifact_revisions (
    community_id UUID NOT NULL REFERENCES communities(id),
    event_id BYTEA NOT NULL CHECK (length(event_id) = 32),
    artifact_id UUID NOT NULL,
    source_channel_id UUID,
    modified_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, event_id)
);

CREATE INDEX idx_artifact_revisions_artifact
    ON artifact_revisions (community_id, artifact_id);
CREATE INDEX artifact_revisions_moves ON artifact_revisions (community_id, modified_at, event_id)
    WHERE source_channel_id IS NOT NULL;
