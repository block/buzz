CREATE TABLE personal_read_frontiers (
    community_id UUID NOT NULL,
    actor BYTEA NOT NULL,
    channel_id UUID NOT NULL,
    root_id BYTEA NOT NULL DEFAULT ''::bytea CHECK (octet_length(root_id) IN (0, 32)),
    through_timestamp TIMESTAMPTZ NOT NULL,
    -- The message whose arrival is through_timestamp; NULL before 0058.
    through_message_id BYTEA CHECK (octet_length(through_message_id) = 32),
    -- A sticky thread unfollow; thread rows only.
    unfollowed BOOLEAN NOT NULL DEFAULT false CHECK (NOT unfollowed OR root_id <> ''::bytea),
    -- A follow intent: never dormant. Thread rows only.
    explicit BOOLEAN NOT NULL DEFAULT false,
    -- The actor's last involvement in the thread: posting, being mentioned,
    -- reading, following. An implicit follow older than the window is dormant.
    active_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, actor, channel_id, root_id),
    FOREIGN KEY (community_id, actor)
        REFERENCES personal_read_accounts (community_id, actor) ON DELETE CASCADE,
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);

-- Live follows, and every thread the actor was involved in, newest first.
CREATE INDEX idx_personal_read_frontiers_active
    ON personal_read_frontiers (community_id, actor, active_at DESC)
    WHERE root_id <> ''::bytea;
