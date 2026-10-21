CREATE FUNCTION protect_entity_deletion_approval()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP <> 'INSERT' THEN
        RAISE EXCEPTION 'entity deletion approval evidence is immutable';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM entity_deletion_requests
        WHERE id = NEW.request_id AND inventory_digest = NEW.inventory_digest
          AND status = 'inventoried' AND requested_by <> NEW.approved_by
    ) THEN
        RAISE EXCEPTION 'approval requires a frozen inventory and a different operator';
    END IF;
    RETURN NEW;
END;
$$;
