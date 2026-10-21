CREATE TRIGGER protect_entity_deletion_approval
    BEFORE INSERT OR UPDATE OR DELETE ON entity_deletion_approvals
    FOR EACH ROW EXECUTE FUNCTION protect_entity_deletion_approval();
