CREATE TABLE personal_read_frontiers (
    community_id UUID NOT NULL,
    actor BYTEA NOT NULL,
    channel_id UUID NOT NULL,
    root_id BYTEA NOT NULL DEFAULT ''::bytea CHECK (octet_length(root_id) IN (0, 32)),
    -- NULL: never read. The account's started_at is the floor.
    through_timestamp TIMESTAMPTZ,
    -- The message whose arrival is through_timestamp; NULL before 0058.
    through_message_id BYTEA CHECK (octet_length(through_message_id) = 32),
    state TEXT NOT NULL DEFAULT 'default'
        CHECK (state IN ('default', 'following', 'muted')),
    -- Manual unread; the next mark_through clears it.
    marked_unread BOOLEAN NOT NULL DEFAULT false,
    -- personal_read_accounts.seq at the actor's last write to this row.
    seq BIGINT NOT NULL DEFAULT 0,
    -- When the actor last posted here, was replied to or was tagged. A thread
    -- in the default state goes quiet 30 days after it.
    engaged_at TIMESTAMPTZ,
    PRIMARY KEY (community_id, actor, channel_id, root_id),
    FOREIGN KEY (community_id, actor)
        REFERENCES personal_read_accounts (community_id, actor) ON DELETE CASCADE,
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);
CREATE INDEX personal_read_frontiers_seq ON personal_read_frontiers (community_id, actor, seq);
