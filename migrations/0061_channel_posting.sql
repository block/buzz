-- Who may write in a channel. `everyone` keeps the existing rule (members,
-- plus anyone when the channel is open). `members` is an announce channel:
-- only owners, admins, members and bots may write; guests and non-members
-- may only read, join (as guest) and leave. Published in kind:39000 as the
-- NIP-29 `restricted` flag. The constant default fills existing rows without
-- a table rewrite, and keeps today's behavior for every existing channel.
SET LOCAL lock_timeout = '5s';

ALTER TABLE channels
    ADD COLUMN posting TEXT NOT NULL DEFAULT 'everyone'
        CHECK (posting IN ('everyone', 'members'));
