CREATE FUNCTION protect_entity_deletion_stage()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'entity deletion stage history cannot be removed';
    END IF;
    IF OLD.status = 'completed' OR
        (NEW.id, NEW.request_id, NEW.sequence, NEW.target_community_id, NEW.stage)
        IS DISTINCT FROM
        (OLD.id, OLD.request_id, OLD.sequence, OLD.target_community_id, OLD.stage)
    THEN
        RAISE EXCEPTION 'completed history and stage identity are immutable';
    END IF;
    IF OLD.started_at IS NOT NULL AND NEW.started_at IS DISTINCT FROM OLD.started_at THEN
        RAISE EXCEPTION 'first stage start time is immutable';
    END IF;
    RETURN NEW;
END;
$$;
