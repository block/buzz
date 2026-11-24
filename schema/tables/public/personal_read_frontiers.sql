CREATE TABLE personal_read_frontiers (
    community_id UUID NOT NULL,
    actor BYTEA NOT NULL,
    channel_id UUID NOT NULL,
    root_id BYTEA NOT NULL DEFAULT ''::bytea CHECK (octet_length(root_id) IN (0, 32)),
    through_timestamp TIMESTAMPTZ NOT NULL,
    -- The message whose arrival is through_timestamp; NULL before 0058.
    through_message_id BYTEA CHECK (octet_length(through_message_id) = 32),
    PRIMARY KEY (community_id, actor, channel_id, root_id)
);
