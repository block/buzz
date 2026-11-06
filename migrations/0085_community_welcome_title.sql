-- The custom title of the community's welcome message, beside the message
-- itself (`welcome_message`, migration 0084). Only a relay owner or admin
-- sets it, with kind:9002 ["welcome_title", text] on the landing channel;
-- the landing channel's kind:39000 carries `welcome_title` while it is
-- customized. NULL means the client's default title.
-- Nullable with no default, so existing rows need no rewrite and the
-- deployed relay, which never reads it, is unaffected.
SET LOCAL lock_timeout = '5s';

ALTER TABLE communities
    ADD COLUMN welcome_title TEXT;
