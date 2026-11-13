-- The community's custom emoji catalog, one row per shortcode (NIP-CE).
-- `change` orders every write in the community, so clients read only what
-- changed since the last change they hold. A removed shortcode keeps its row
-- with a NULL url.
CREATE TABLE custom_emoji (
    community_id UUID NOT NULL,
    shortcode TEXT NOT NULL,
    change BIGINT NOT NULL,
    url TEXT,
    pubkey BYTEA NOT NULL CHECK (length(pubkey) = 32),
    event_id BYTEA NOT NULL CHECK (length(event_id) = 32),
    PRIMARY KEY (community_id, shortcode)
);

CREATE UNIQUE INDEX idx_custom_emoji_change ON custom_emoji (community_id, change);
