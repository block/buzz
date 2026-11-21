-- Marks a channel whose rows are all at or after its creation, less the write
-- path's created_at tolerance, so an oldest-first page skips the partitions
-- before it. The relay sets it true when it creates a channel. Existing rows
-- stay NULL, which, like false, keeps the read below the channel's floor: a
-- restore or backfill may have stored older rows there. No default, so no
-- existing row is marked.
SET LOCAL lock_timeout = '5s';
ALTER TABLE channels ADD COLUMN history_from_creation BOOLEAN;
