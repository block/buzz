-- Expand only: old relays ignore this nullable recovery token.
-- New roster writers set it atomically; existing members are not backfilled.
SET LOCAL lock_timeout = '5s';

ALTER TABLE relay_members ADD COLUMN auto_join_pending UUID;
