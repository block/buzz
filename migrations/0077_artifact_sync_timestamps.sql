SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';

ALTER TABLE artifact_heads ADD COLUMN IF NOT EXISTS modified_at TIMESTAMPTZ NOT NULL DEFAULT 'epoch';
ALTER TABLE artifact_heads ALTER COLUMN modified_at SET DEFAULT clock_timestamp();
ALTER TABLE artifact_revisions ADD COLUMN IF NOT EXISTS source_channel_id UUID;
ALTER TABLE artifact_revisions ADD COLUMN IF NOT EXISTS modified_at TIMESTAMPTZ NOT NULL DEFAULT 'epoch';
ALTER TABLE artifact_revisions ALTER COLUMN modified_at SET DEFAULT clock_timestamp();
ALTER TABLE channel_members ADD COLUMN IF NOT EXISTS sync_modified_at TIMESTAMPTZ NOT NULL DEFAULT 'epoch';
ALTER TABLE channel_members ALTER COLUMN sync_modified_at SET DEFAULT clock_timestamp();
ALTER TABLE communities ADD COLUMN IF NOT EXISTS artifact_policy_modified_at TIMESTAMPTZ NOT NULL DEFAULT 'epoch';
