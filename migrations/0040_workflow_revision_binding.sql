-- Preserve exact signed workflow revisions without changing legacy execution.
-- Existing rows remain NULL until an explicit, provenance-safe migration.
ALTER TABLE workflows
    ADD COLUMN definition_event_id BYTEA
        CHECK (definition_event_id IS NULL OR octet_length(definition_event_id) = 32);
ALTER TABLE workflow_runs
    ADD COLUMN definition_event_id BYTEA
        CHECK (definition_event_id IS NULL OR octet_length(definition_event_id) = 32);
