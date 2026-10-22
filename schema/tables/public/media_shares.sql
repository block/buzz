-- ── Media shares ──────────────────────────────────────────────────────────────
-- Each event's grant to a channel, or the whole community for a NULL channel.
-- Reads join the source event: deleted or purged events grant no access.
-- Keep its timestamp for an indexed lookup in the partitioned events table.

CREATE TABLE media_shares (
    community_id UUID NOT NULL,
    sha256       TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    channel_id   UUID,
    event_id     BYTEA NOT NULL,
    event_created_at TIMESTAMPTZ NOT NULL,
    target_event_id BYTEA,
    target_event_created_at TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((target_event_id IS NULL) = (target_event_created_at IS NULL)),
    PRIMARY KEY (community_id, sha256, event_id)
);
