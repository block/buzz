-- Durable community archival state. Archived hosts remain reserved by the existing
-- full unique index and continue to count toward owner quotas.
ALTER TABLE communities ADD COLUMN archived_at TIMESTAMPTZ;
