-- Where each blob is shared: one row per channel an event referencing it was
-- stored in, or a NULL channel for a channel-less reference (avatars, custom
-- emoji, notes), which shares it with the whole community. Media reads admit a
-- reader who can see any share; rows are written with the referencing event.
CREATE TABLE media_shares (
    community_id UUID NOT NULL REFERENCES communities(id),
    sha256       TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    channel_id   UUID,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX idx_media_shares_channel
    ON media_shares (community_id, sha256, channel_id) WHERE channel_id IS NOT NULL;
CREATE UNIQUE INDEX idx_media_shares_public
    ON media_shares (community_id, sha256) WHERE channel_id IS NULL;

SELECT attach_community_write_fence('media_shares');
