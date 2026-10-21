CREATE FUNCTION protect_entity_deletion_request()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF (NEW.id, NEW.entity_type, NEW.target, NEW.requested_by, NEW.reason, NEW.created_at)
        IS DISTINCT FROM
        (OLD.id, OLD.entity_type, OLD.target, OLD.requested_by, OLD.reason, OLD.created_at)
    THEN
        RAISE EXCEPTION 'entity deletion target and provenance are immutable';
    END IF;
    IF OLD.inventory_frozen_at IS NOT NULL AND
        (NEW.inventory_manifest, NEW.inventory_digest, NEW.inventory_frozen_at)
        IS DISTINCT FROM
        (OLD.inventory_manifest, OLD.inventory_digest, OLD.inventory_frozen_at)
    THEN
        RAISE EXCEPTION 'entity deletion inventory is immutable';
    END IF;
    RETURN NEW;
END;
$$;
