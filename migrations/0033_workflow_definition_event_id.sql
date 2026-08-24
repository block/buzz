-- Bind each materialized workflow row to the exact owner-signed kind:30620
-- revision that produced it. Existing rows remain nullable until re-saved;
-- workflow doorbells fail closed when the revision is unavailable.
ALTER TABLE workflows ADD COLUMN definition_event_id BYTEA;
