-- NIP-CM channel-wide mentions (@channel).
--
-- One row per notifying event, NOT one row per member: the mentions feed
-- resolves the audience at read time by joining channel_members, so a
-- 5000-member channel costs a single row here and the roster is never
-- denormalized into the event or this table.
--
-- @here is deliberately absent: it is live-only (no persistence, no
-- retroactive badge). `mode` is still stored so the column can carry future
-- persistent modes without a second table.
CREATE TABLE channel_notifications (
    community_id     UUID NOT NULL REFERENCES communities(id),
    channel_id       UUID NOT NULL,
    event_id         BYTEA NOT NULL CHECK (length(event_id) = 32),
    mode             TEXT NOT NULL CHECK (mode IN ('channel')),
    event_created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, event_id),
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_channel_notifications_channel_created
    ON channel_notifications (community_id, channel_id, event_created_at DESC);
