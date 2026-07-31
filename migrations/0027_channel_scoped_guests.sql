-- Identity-bound, channel-scoped guest access.
--
-- A relay guest is admitted to the community only through explicit rows in
-- relay_guest_channels. The grant table is separate from channel_members so a
-- stale channel membership cannot silently restore guest authority after the
-- relay member is removed and later re-invited.
--
-- Guest invites are always bound to one private, non-DM channel. The relay
-- revalidates that channel at claim and authentication time; these constraints
-- keep the durable representation unambiguous.

ALTER TABLE relay_members
    DROP CONSTRAINT relay_members_role_check,
    ADD CONSTRAINT relay_members_role_check
        CHECK (role IN ('owner', 'admin', 'member', 'guest'));

ALTER TABLE relay_invites
    DROP CONSTRAINT relay_invites_role_check;

ALTER TABLE relay_invites
    ADD COLUMN channel_id UUID,
    ADD COLUMN channel_generation BIGINT,
    ADD COLUMN revoked_at TIMESTAMPTZ,
    ADD COLUMN revoked_by TEXT;

ALTER TABLE channels
    ADD COLUMN guest_invite_generation BIGINT NOT NULL DEFAULT 0;

ALTER TABLE relay_invites
    ADD CONSTRAINT relay_invites_role_check
        CHECK (role IN ('member', 'guest'));

ALTER TABLE relay_invites
    ADD CONSTRAINT relay_invites_guest_shape_check
        CHECK (
            (role = 'member' AND channel_id IS NULL AND channel_generation IS NULL)
            OR (
                role = 'guest'
                AND channel_id IS NOT NULL
                AND channel_generation IS NOT NULL
                AND max_uses = 1
            )
        );

ALTER TABLE relay_invites
    ADD CONSTRAINT relay_invites_channel_fk
        FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE;

CREATE TABLE relay_guest_channels (
    community_id UUID        NOT NULL,
    guest_pubkey TEXT        NOT NULL,
    channel_id   UUID        NOT NULL,
    granted_by   TEXT        NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, guest_pubkey),
    FOREIGN KEY (community_id, guest_pubkey)
        REFERENCES relay_members (community_id, pubkey) ON DELETE CASCADE,
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);

CREATE INDEX relay_guest_channels_channel_idx
    ON relay_guest_channels (community_id, channel_id);

-- An administrator removal is different from a voluntary leave. Preserve the
-- removed identity so an old bearer invite cannot immediately recreate relay
-- membership. An explicit administrator add clears this row.
CREATE TABLE relay_member_invite_blocks (
    community_id UUID        NOT NULL,
    pubkey        TEXT        NOT NULL,
    removed_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, pubkey),
    FOREIGN KEY (community_id)
        REFERENCES communities (id) ON DELETE CASCADE,
    CHECK (pubkey ~ '^[0-9a-f]{64}$')
);
