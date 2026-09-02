-- Current per-channel override for collaborative Project related-channel commands.
-- The accepted kind:47010 events remain the durable history; this table is an
-- authoritative desired-state override used for atomic reads and writes.
CREATE TABLE project_related_channel_overrides (
    community_id UUID NOT NULL REFERENCES communities(id),
    project_owner BYTEA NOT NULL,
    project_d TEXT NOT NULL,
    channel_id UUID NOT NULL,
    present BOOLEAN NOT NULL,
    PRIMARY KEY (community_id, project_owner, project_d, channel_id),
    CONSTRAINT chk_project_related_channel_owner_len CHECK (LENGTH(project_owner) = 32),
    CONSTRAINT chk_project_related_channel_d_len CHECK (LENGTH(project_d) BETWEEN 1 AND 1024)
);

SELECT attach_community_write_fence('project_related_channel_overrides');
