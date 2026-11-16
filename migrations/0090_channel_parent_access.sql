-- What a channel's parents' people get in it (`crate::channel_tree`):
-- `member` caps roles found above at member, today's behavior and the
-- default for every existing row; `guest` caps them at guest, so they read
-- but cannot post; `none` stops the walk, so only the channel's own rows
-- (and, while it is open, its own join_role) grant anything, and its
-- children are cut off from the channels above it too. Set with kind:9002
-- ["parent_access", <value>]; published in kind:39000 when not `member`.
-- The constant default fills existing rows without a table rewrite, as 0067
-- does, under the lock and statement budgets of docs/database.md Rule 6.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';
SET LOCAL search_path = public, pg_catalog;

ALTER TABLE channels
    ADD COLUMN IF NOT EXISTS parent_access TEXT NOT NULL DEFAULT 'member'
        CHECK (parent_access IN ('member', 'guest', 'none'));
