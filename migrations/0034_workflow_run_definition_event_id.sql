-- Bind each new workflow run to the exact signed definition it executes.
-- Existing rows remain NULL and all resume/execution paths fail them closed.
ALTER TABLE workflow_runs
    ADD COLUMN definition_event_id BYTEA
        CHECK (definition_event_id IS NULL OR octet_length(definition_event_id) = 32);
