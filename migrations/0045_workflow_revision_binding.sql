-- Preserve exact signed workflow revisions without changing legacy execution.
-- Existing rows remain NULL until a new signed definition is ingested.
-- NOT VALID skips validation scans under ACCESS EXCLUSIVE on existing tables.
-- These columns are new and every old row reads NULL. PostgreSQL still enforces
-- the checks on every subsequent INSERT/UPDATE. There is no later validation
-- job or historical revision inference; fresh schema uses validated checks.
ALTER TABLE workflows
    ADD COLUMN definition_event_id BYTEA,
    ADD CONSTRAINT workflows_definition_event_id_check
        CHECK (definition_event_id IS NULL OR octet_length(definition_event_id) = 32) NOT VALID;
ALTER TABLE workflow_runs
    ADD COLUMN definition_event_id BYTEA,
    ADD CONSTRAINT workflow_runs_definition_event_id_check
        CHECK (definition_event_id IS NULL OR octet_length(definition_event_id) = 32) NOT VALID;

-- A legacy writer does not mention definition_event_id. Column-targeted triggers
-- fire even for equal-value rewrites, where comparing OLD/NEW would invent
-- provenance. New writers rebind separately while still holding the row lock
-- in the signed-event transaction. Operational status/enabled updates keep it.
CREATE FUNCTION invalidate_workflow_revision() RETURNS TRIGGER AS $$
BEGIN
    NEW.definition_event_id := NULL;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER workflows_invalidate_revision
BEFORE UPDATE OF id, community_id, owner_pubkey, channel_id, name, definition, definition_hash
ON workflows
FOR EACH ROW EXECUTE FUNCTION invalidate_workflow_revision();
