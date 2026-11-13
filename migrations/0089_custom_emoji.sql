-- Expand: the community's custom emoji catalog, one row per shortcode.
-- New table only; the currently deployed relay never reads or writes it.
-- All writers use community admission (docs/database.md Rule 1).
SET LOCAL lock_timeout = '5s';
CREATE TABLE custom_emoji (
    community_id UUID NOT NULL,
    shortcode TEXT NOT NULL,
    -- Position of this row's latest write in the community's change order.
    change BIGINT NOT NULL,
    -- Image URL; NULL once the shortcode is removed.
    url TEXT,
    -- Owner, or whoever removed it.
    pubkey BYTEA NOT NULL CHECK (length(pubkey) = 32),
    -- The kind:30180 event behind this row.
    event_id BYTEA NOT NULL CHECK (length(event_id) = 32),
    PRIMARY KEY (community_id, shortcode)
);
CREATE UNIQUE INDEX idx_custom_emoji_change ON custom_emoji (community_id, change);
