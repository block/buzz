-- Scope the DM idempotency index to live channels.
--
-- `soft_delete_channel` sets `deleted_at` and leaves `participant_hash` in
-- place, while `find_dm_in` looks DMs up with `deleted_at IS NULL`. Reopening
-- the same participant set after a delete therefore finds nothing, inserts a
-- new row, and violates the unique index on the deleted row. The pair can
-- never open a DM again. A unique index that ignores deleted rows fixes this
-- without touching any data.
--
-- Contract-free swap: both statements run in this one transaction, so
-- `channels` is never without a uniqueness guarantee for live DMs. The drop
-- takes ACCESS EXCLUSIVE on `channels`; the budgets fail this migration
-- rather than queue relay traffic behind it. The new index is strictly weaker
-- than the old one (a subset of rows), so an older relay running against the
-- migrated database keeps working.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

DROP INDEX idx_channels_dm_hash;
CREATE UNIQUE INDEX idx_channels_dm_hash ON channels (community_id, participant_hash)
    WHERE participant_hash IS NOT NULL AND deleted_at IS NULL;
