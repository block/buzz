CREATE TABLE project_state_heads (
    community_id UUID NOT NULL REFERENCES communities(id),
    project_owner BYTEA NOT NULL CHECK (octet_length(project_owner) = 32),
    project_d_tag TEXT NOT NULL CHECK (project_d_tag <> ''),
    revision BIGINT NOT NULL CHECK (revision > 0),
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    identity_event_id BYTEA NOT NULL CHECK (octet_length(identity_event_id) = 32),
    last_event_id BYTEA NOT NULL CHECK (octet_length(last_event_id) = 32),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, project_owner, project_d_tag)
);
CREATE TABLE project_related_channels (
    community_id UUID NOT NULL,
    project_owner BYTEA NOT NULL CHECK (octet_length(project_owner) = 32),
    project_d_tag TEXT NOT NULL CHECK (project_d_tag <> ''),
    channel_id UUID NOT NULL,
    PRIMARY KEY (community_id, project_owner, project_d_tag, channel_id),
    FOREIGN KEY (community_id, project_owner, project_d_tag)
        REFERENCES project_state_heads (community_id, project_owner, project_d_tag)
        ON DELETE CASCADE
);
SELECT attach_community_write_fence('project_state_heads');
SELECT attach_community_write_fence('project_related_channels');
