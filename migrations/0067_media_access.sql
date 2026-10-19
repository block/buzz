-- Channel-scoped media access: who may read each blob.
--
-- media_uploaders: who has uploaded each blob. Uploading the exact bytes
-- proves possession, so an uploader may share a blob into their own channels
-- even when the same bytes are also shared somewhere they cannot read.
CREATE TABLE media_uploaders (
    community_id UUID NOT NULL REFERENCES communities(id),
    sha256       TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    pubkey       BYTEA NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, sha256, pubkey)
);

-- media_shares: one row per channel a blob is shared into, written with the
-- event that references it, or a NULL channel for a channel-less reference
-- (avatars, custom emoji, notes), which shares it with the whole community.
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

SELECT attach_community_write_fence('media_uploaders');
SELECT attach_community_write_fence('media_shares');
