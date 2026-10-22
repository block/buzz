-- Durable historical backfill cutoff, independent of the schema install tool.
ALTER TABLE communities
    ADD COLUMN media_shares_recording_started_at TIMESTAMPTZ NOT NULL DEFAULT now();

-- Channel-scoped media access: who may read each blob.
--
-- media_uploaders: who has uploaded each blob. Uploading the exact bytes
-- proves possession, so an uploader may share a blob into their own channels
-- even when the same bytes are also shared somewhere they cannot read.
CREATE TABLE media_uploaders (
    community_id UUID NOT NULL,
    sha256       TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    pubkey       BYTEA NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, sha256, pubkey)
);

-- media_shares: one grant per referencing event and blob. A NULL channel
-- shares with the whole community. Deleted or purged source events grant
-- nothing; event_created_at locates the source in the partitioned table.
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
