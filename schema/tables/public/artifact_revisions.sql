-- Every accepted revision ID, so replays stay idempotent after redaction or
-- retention.
CREATE TABLE artifact_revisions (
    community_id UUID NOT NULL REFERENCES communities(id),
    event_id BYTEA NOT NULL CHECK (length(event_id) = 32),
    artifact_id UUID NOT NULL,
    PRIMARY KEY (community_id, event_id)
);
