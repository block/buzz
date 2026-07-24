-- ── User groups ──────────────────────────────────────────────────────────────
-- Slack-style, community-shared groups of relay-member pubkeys. Group UUIDs
-- are wire identifiers and may repeat in different communities, so every key
-- and relationship is led by community_id.

CREATE TABLE user_groups (
    community_id UUID NOT NULL REFERENCES communities(id),
    id           UUID NOT NULL DEFAULT gen_random_uuid(),
    handle       TEXT NOT NULL,
    name         TEXT NOT NULL,
    description  TEXT,
    created_by   TEXT NOT NULL,
    snapshot_version BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM clock_timestamp())::BIGINT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at   TIMESTAMPTZ,
    PRIMARY KEY (community_id, id),
    CONSTRAINT chk_user_groups_id_not_nil
        CHECK (id <> '00000000-0000-0000-0000-000000000000'::uuid)
);

-- A deleted group releases its mention handle for reuse in the same
-- community while retaining the tombstoned row for state publication/audit.
CREATE UNIQUE INDEX idx_user_groups_active_handle
    ON user_groups (community_id, handle)
    WHERE deleted_at IS NULL;

CREATE INDEX idx_user_groups_created_by
    ON user_groups (community_id, created_by)
    WHERE deleted_at IS NULL;

CREATE TABLE user_group_members (
    community_id UUID NOT NULL REFERENCES communities(id),
    group_id     UUID NOT NULL,
    pubkey       TEXT NOT NULL,
    added_by     TEXT NOT NULL,
    added_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, group_id, pubkey),
    FOREIGN KEY (community_id, group_id)
        REFERENCES user_groups (community_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_user_group_members_pubkey
    ON user_group_members (community_id, pubkey);

CREATE TABLE user_group_default_channels (
    community_id UUID NOT NULL REFERENCES communities(id),
    group_id     UUID NOT NULL,
    channel_id   UUID NOT NULL,
    PRIMARY KEY (community_id, group_id, channel_id),
    FOREIGN KEY (community_id, group_id)
        REFERENCES user_groups (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_user_group_default_channels_channel
    ON user_group_default_channels (community_id, channel_id);
