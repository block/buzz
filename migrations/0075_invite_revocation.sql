SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE relay_invites ADD COLUMN revoked_at TIMESTAMPTZ;
ALTER TABLE relay_invites ADD COLUMN legacy_admission BOOLEAN NOT NULL DEFAULT true;
