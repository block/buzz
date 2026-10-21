CREATE TRIGGER protect_entity_deletion_request
    BEFORE UPDATE ON entity_deletion_requests
    FOR EACH ROW EXECUTE FUNCTION protect_entity_deletion_request();
