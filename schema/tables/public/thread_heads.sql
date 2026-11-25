-- One shared row per thread with at least one reply: its newest reply. A
-- cache of the event log that a read verifies and rebuilds.
-- See docs/buzz-v1-activity.md.
CREATE TABLE thread_heads (
    community_id UUID NOT NULL,
    root_id BYTEA NOT NULL CHECK (octet_length(root_id) = 32),
    channel_id UUID NOT NULL,
    -- Relay arrival (events.received_at) of the newest reply.
    activity_at TIMESTAMPTZ,
    activity_id BYTEA CHECK (octet_length(activity_id) = 32),
    activity_created_at TIMESTAMPTZ,
    activity_pubkey BYTEA CHECK (octet_length(activity_pubkey) = 32),
    activity_text TEXT,
    -- Author time of the newest stored reply the row accounts for. NULL until
    -- the on-read fill has merged the thread's stored history.
    seen_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, root_id)
);
