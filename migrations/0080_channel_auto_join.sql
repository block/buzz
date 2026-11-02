-- Whether the relay joins every new community member into this channel.
-- Only a relay owner or admin sets it, with kind:9002 ["auto_join", "true"|
-- "false"]; it is accepted only on an open, unarchived stream channel.
-- Published in kind:39000 as a bare `auto_join` tag while it is effective
-- (set, open and unarchived), so what the event says is what the relay does.
-- The constant default fills existing rows without a table rewrite: no
-- channel is a default channel until someone asks for it.
SET LOCAL lock_timeout = '5s';

ALTER TABLE channels
    ADD COLUMN auto_join BOOLEAN NOT NULL DEFAULT false;
