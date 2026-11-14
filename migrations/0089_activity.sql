-- Stored state for GET /buzz/v1/me/activity. See docs/buzz-v1-activity.md.
-- Expand only: two new tables and nullable or constant-default columns. No
-- trigger, function, foreign key or backfill. 0090 adds the indexes on the
-- existing tables.
--
-- channel_heads and thread_heads are caches of the event log that a read
-- verifies and rebuilds. They are written after the event commits and by the
-- bounded on-read fill.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE TABLE channel_heads (
    community_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    activity_at TIMESTAMPTZ,
    activity_id BYTEA CHECK (octet_length(activity_id) = 32),
    activity_pubkey BYTEA CHECK (octet_length(activity_pubkey) = 32),
    head_id BYTEA CHECK (octet_length(head_id) = 32),
    head_created_at TIMESTAMPTZ,
    head_received_at TIMESTAMPTZ,
    head_pubkey BYTEA CHECK (octet_length(head_pubkey) = 32),
    head_text TEXT,
    head_edit_at TIMESTAMPTZ,
    head_edit_id BYTEA CHECK (octet_length(head_edit_id) = 32),
    seen_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, channel_id)
);

CREATE TABLE thread_heads (
    community_id UUID NOT NULL,
    root_id BYTEA NOT NULL CHECK (octet_length(root_id) = 32),
    channel_id UUID NOT NULL,
    activity_at TIMESTAMPTZ,
    activity_id BYTEA CHECK (octet_length(activity_id) = 32),
    activity_created_at TIMESTAMPTZ,
    activity_pubkey BYTEA CHECK (octet_length(activity_pubkey) = 32),
    activity_text TEXT,
    reply_count INT NOT NULL DEFAULT 0,
    seen_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, root_id)
);
CREATE INDEX thread_heads_updated ON thread_heads (community_id, updated_at);

ALTER TABLE personal_read_accounts ADD COLUMN seq BIGINT NOT NULL DEFAULT 0;

-- A NULL read position means "never read": the account's start is the floor.
ALTER TABLE personal_read_frontiers
    ALTER COLUMN through_timestamp DROP NOT NULL,
    ADD COLUMN state TEXT NOT NULL DEFAULT 'default'
        CHECK (state IN ('default', 'following', 'muted')),
    ADD COLUMN marked_unread BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN seq BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN engaged_at TIMESTAMPTZ;

ALTER TABLE event_mentions
    ADD COLUMN received_at TIMESTAMPTZ,
    ADD COLUMN root_id BYTEA;
