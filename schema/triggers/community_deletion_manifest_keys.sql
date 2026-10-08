CREATE TRIGGER community_deletion_manifest_keys_guard
BEFORE INSERT OR UPDATE OR DELETE ON community_deletion_manifest_keys
FOR EACH ROW
EXECUTE FUNCTION protect_community_deletion_manifest_keys();
