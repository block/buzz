-- One stored row per channel for sidebar lists: last activity and the newest
-- top-level message. A cache of the event log that a read verifies and rebuilds;
-- written after the event commits and by the bounded on-read fill; no trigger, function, foreign key or backfill.
-- See docs/buzz-v1-channel-heads.md.
SET LOCAL lock_timeout = '5s';
CREATE TABLE channel_heads (
    community_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    activity_at TIMESTAMPTZ,
    activity_pubkey BYTEA CHECK (octet_length(activity_pubkey) = 32),
    head_id BYTEA CHECK (octet_length(head_id) = 32),
    head_at TIMESTAMPTZ,
    head_pubkey BYTEA CHECK (octet_length(head_pubkey) = 32),
    head_text TEXT,
    head_edit_at TIMESTAMPTZ,
    head_edit_id BYTEA CHECK (octet_length(head_edit_id) = 32),
    -- False until the on-read fill has merged the channel's stored history.
    seen_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, channel_id)
);
