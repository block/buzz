-- Bind each materialized workflow row to the exact owner-signed kind:30620
-- revision that produced it. Existing rows remain nullable until re-saved;
-- revision-bound execution fails closed when the revision is unavailable.
ALTER TABLE workflows
    ADD COLUMN definition_event_id BYTEA
        CHECK (definition_event_id IS NULL OR octet_length(definition_event_id) = 32);
