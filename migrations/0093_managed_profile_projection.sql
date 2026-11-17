SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE nip_fi_profile_heads ADD COLUMN managed_event JSONB, ADD COLUMN subject TEXT;
