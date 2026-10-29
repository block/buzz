SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- #645: per-channel export policy. A channel flagged no_bridge declares
-- "my events must not be replicated off this relay" — the kannaka-hive-bridge
-- already reads a `no-bridge` tag on kind:39000 and fails closed on channels
-- it cannot resolve; this column is what finally lets a channel emit it.
-- Default false: existing channels are unaffected.
ALTER TABLE channels
    ADD COLUMN no_bridge BOOLEAN NOT NULL DEFAULT FALSE;
