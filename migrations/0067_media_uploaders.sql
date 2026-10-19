-- Who has uploaded each blob in a community. Uploading the exact bytes proves
-- possession, so channel-scoped media reads can always admit an uploader even
-- when the same bytes are also attached somewhere they cannot read.
--
-- Additive. Nothing reads this table yet.
CREATE TABLE media_uploaders (
    community_id UUID NOT NULL REFERENCES communities(id),
    sha256       TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    pubkey       BYTEA NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, sha256, pubkey)
);

SELECT attach_community_write_fence('media_uploaders');
