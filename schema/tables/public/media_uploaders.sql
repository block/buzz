-- ── Media uploaders ───────────────────────────────────────────────────────────
-- Who has uploaded each blob in a community. Uploading the exact bytes proves
-- possession, so channel-scoped media reads can always admit an uploader.

CREATE TABLE media_uploaders (
    community_id UUID NOT NULL REFERENCES communities(id),
    sha256       TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    pubkey       BYTEA NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, sha256, pubkey)
);
