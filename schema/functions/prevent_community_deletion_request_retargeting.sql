CREATE OR REPLACE FUNCTION prevent_community_deletion_request_retargeting()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.community_host IS DISTINCT FROM OLD.community_host
    THEN
        RAISE EXCEPTION 'community deletion target identity is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NEW.request_origin IS DISTINCT FROM OLD.request_origin
        OR NEW.owner_pubkey IS DISTINCT FROM OLD.owner_pubkey
        OR NEW.mediating_operator_pubkey IS DISTINCT FROM OLD.mediating_operator_pubkey
        OR NEW.acknowledgement_version IS DISTINCT FROM OLD.acknowledgement_version
    THEN
        RAISE EXCEPTION 'community deletion request provenance is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.inventory_frozen_at IS NOT NULL AND (
        NEW.schema_manifest IS DISTINCT FROM OLD.schema_manifest
        OR NEW.storage_manifest IS DISTINCT FROM OLD.storage_manifest
        OR NEW.inventory_manifest IS DISTINCT FROM OLD.inventory_manifest
        OR NEW.inventory_digest IS DISTINCT FROM OLD.inventory_digest
        OR NEW.inventory_frozen_at IS DISTINCT FROM OLD.inventory_frozen_at
    ) THEN
        RAISE EXCEPTION 'frozen community deletion inventory is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.destructive_storage_frozen_at IS NOT NULL AND (
        NEW.destructive_storage_manifest IS DISTINCT FROM OLD.destructive_storage_manifest
        OR NEW.destructive_storage_frozen_at IS DISTINCT FROM OLD.destructive_storage_frozen_at
    ) THEN
        RAISE EXCEPTION 'frozen destructive storage manifest is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END;
$$;
