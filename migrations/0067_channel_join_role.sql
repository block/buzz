-- The role a person gets in a channel when nobody picks one: a self-join
-- (kind:9021, or a kind:9000 self-add), or a kind:9000 add with no role tag.
-- `member` keeps today's behavior. `guest` on an open channel makes an
-- announce channel: anyone can follow and read it, and guests cannot write.
-- This never changes what a role can do. Published in kind:39000 as an
-- explicit `join_role` tag, and as the NIP-29 `restricted` flag on open
-- channels with `guest`. The constant default fills existing rows without a
-- table rewrite, and keeps today's join behavior for every existing channel.
SET LOCAL lock_timeout = '5s';

ALTER TABLE channels
    ADD COLUMN join_role TEXT NOT NULL DEFAULT 'member'
        CHECK (join_role IN ('member', 'guest'));
