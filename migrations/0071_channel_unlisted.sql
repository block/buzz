-- Whether an open channel is left out of channel listings for non-members.
-- An unlisted open channel can still be read and joined by anyone who names
-- it by id (a shared link); it is only left out of what lists open channels:
-- the implicit scope of a read that does not name a channel, and the channel
-- list. Members still list it. The flag is stored on its own and matters only
-- while the channel is open: private channels are already invisible to
-- non-members. Published in kind:39000 as an `unlisted` tag. The constant
-- default fills existing rows without a table rewrite and keeps every existing
-- channel listed.
SET LOCAL lock_timeout = '5s';

ALTER TABLE channels
    ADD COLUMN unlisted BOOLEAN NOT NULL DEFAULT FALSE;
