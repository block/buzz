CREATE TABLE personal_read_frontiers (
    community_id UUID NOT NULL,
    actor BYTEA NOT NULL,
    channel_id UUID NOT NULL,
    root_id BYTEA NOT NULL DEFAULT ''::bytea CHECK (octet_length(root_id) IN (0, 32)),
    through_timestamp TIMESTAMPTZ NOT NULL,
    -- Whole-channel cut covering every thread; channel rows only.
    threads_through_timestamp TIMESTAMPTZ
        CHECK (threads_through_timestamp IS NULL OR root_id = ''::bytea),
    PRIMARY KEY (community_id, actor, channel_id, root_id),
    FOREIGN KEY (community_id, actor)
        REFERENCES personal_read_accounts (community_id, actor) ON DELETE CASCADE,
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);
