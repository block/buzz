SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- A NULL expires_at is an invite that never expires. Readers tolerate NULL
-- since the previous release; deploy it everywhere before this migration.
ALTER TABLE relay_invites ALTER COLUMN expires_at DROP NOT NULL;
