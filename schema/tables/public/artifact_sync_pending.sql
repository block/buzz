CREATE TABLE artifact_sync_pending (
    community_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    earliest_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, operation_id)
);

CREATE INDEX artifact_sync_pending_community
    ON artifact_sync_pending (community_id, earliest_at, operation_id);
