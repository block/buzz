CREATE TRIGGER protect_entity_deletion_stage
    BEFORE UPDATE OR DELETE ON entity_deletion_stage_history
    FOR EACH ROW EXECUTE FUNCTION protect_entity_deletion_stage();
