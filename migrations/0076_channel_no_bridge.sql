SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- #645: per-channel export policy. A channel flagged no_bridge declares
-- "my events must not be replicated off this relay" — the kannaka-hive-bridge
-- already reads a `no-bridge` tag on kind:39000 and fails closed on channels
-- it cannot resolve; this column is what finally lets a channel emit it.
-- Default false: existing channels are unaffected.
--
-- IF NOT EXISTS: before the 2026-10-30 upstream sync this migration shipped
-- as 0027 on the fork, so a relay that ran the fork already has the column
-- (and a `_sqlx_migrations` row 27 that must be deleted so upstream's 0027
-- applies). Re-adding must be a no-op there.
ALTER TABLE channels
    ADD COLUMN IF NOT EXISTS no_bridge BOOLEAN NOT NULL DEFAULT FALSE;
