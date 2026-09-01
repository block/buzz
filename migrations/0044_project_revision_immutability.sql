-- Preserve actor-signed Project revisions across mixed relay versions.
--
-- Kind 47001 is a regular event and its full history is the portable audit
-- trail behind `project_revision_heads`. Relays from before that contract can
-- still route a NIP-09 deletion through the generic event soft-delete path.
-- Repair any row deleted during an upgrade window, then reject every future
-- deleted_at transition in PostgreSQL so an old pod cannot erase history.
-- Hold writers out across both statements so no soft-delete can land between
-- the repair and trigger installation.
LOCK TABLE events IN SHARE ROW EXCLUSIVE MODE;

UPDATE events
SET deleted_at = NULL
WHERE kind = 47001 AND deleted_at IS NOT NULL;

CREATE FUNCTION guard_project_revision_soft_delete() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'kind 47001 Project revisions are immutable'
        USING ERRCODE = 'check_violation';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_events_guard_project_revision_soft_delete
    BEFORE UPDATE OF deleted_at ON events
    FOR EACH ROW
    WHEN (
        OLD.kind = 47001
        AND OLD.deleted_at IS DISTINCT FROM NEW.deleted_at
    )
    EXECUTE FUNCTION guard_project_revision_soft_delete();
