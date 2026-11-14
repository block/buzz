-- One shared row per conversation for GET /buzz/v1/me/activity: last activity
-- at any depth and the newest top-level message. A cache of the event log
-- that a read verifies and rebuilds. See docs/buzz-v1-activity.md.
CREATE TABLE channel_heads (
    community_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    -- Relay arrival (events.received_at) of the newest message at any depth.
    activity_at TIMESTAMPTZ,
    activity_id BYTEA CHECK (octet_length(activity_id) = 32),
    activity_pubkey BYTEA CHECK (octet_length(activity_pubkey) = 32),
    head_id BYTEA CHECK (octet_length(head_id) = 32),
    -- Author time, for display and the partition-pruned join to the event.
    head_created_at TIMESTAMPTZ,
    -- Relay arrival: channel unread compares this with the read position.
    head_received_at TIMESTAMPTZ,
    head_pubkey BYTEA CHECK (octet_length(head_pubkey) = 32),
    head_text TEXT,
    head_edit_at TIMESTAMPTZ,
    head_edit_id BYTEA CHECK (octet_length(head_edit_id) = 32),
    -- Author time of the newest stored event the row accounts for. NULL until
    -- the on-read fill has merged the channel's stored history.
    seen_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, channel_id)
);
