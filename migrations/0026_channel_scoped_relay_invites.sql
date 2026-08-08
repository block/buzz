-- Optional channel scope for relay invite links.
--
-- A NULL channel_id preserves the existing community-wide invite behavior.
-- When present, claiming the invite atomically admits the identity to the
-- community and grants the read-only `guest` role in exactly this channel.
ALTER TABLE relay_invites
    ADD COLUMN channel_id UUID;

ALTER TABLE relay_invites
    ADD CONSTRAINT relay_invites_channel_fk
    FOREIGN KEY (community_id, channel_id)
    REFERENCES channels (community_id, id)
    ON DELETE CASCADE;

CREATE INDEX relay_invites_channel_idx
    ON relay_invites (community_id, channel_id)
    WHERE channel_id IS NOT NULL;
