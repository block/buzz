-- Chunk content is immutable once written; the only permitted update is the
-- one-way deleted_at stamp. New chunks are permitted only while the request is
-- fenced and its destructive manifest remains unfrozen. Removal is permitted
-- only while the destructive manifest has not yet frozen (a retried partial
-- freeze rewrites its chunks) or once the request has passed logical
-- verification (terminal cleanup).
CREATE FUNCTION protect_community_deletion_manifest_keys()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    frozen_at TIMESTAMPTZ;
    request_stage TEXT;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.request_id IS DISTINCT FROM OLD.request_id
            OR NEW.chunk_no IS DISTINCT FROM OLD.chunk_no
            OR NEW.prefix IS DISTINCT FROM OLD.prefix
            OR NEW.keys IS DISTINCT FROM OLD.keys
            OR OLD.deleted_at IS NOT NULL
        THEN
            RAISE EXCEPTION 'community deletion manifest key chunks are immutable'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
        RETURN NEW;
    END IF;
    SELECT destructive_storage_frozen_at, stage
      INTO frozen_at, request_stage
      FROM community_deletion_requests
     WHERE id = CASE WHEN TG_OP = 'INSERT' THEN NEW.request_id ELSE OLD.request_id END
     FOR UPDATE;
    IF TG_OP = 'INSERT' THEN
        IF FOUND AND frozen_at IS NULL AND request_stage = 'fenced' THEN
            RETURN NEW;
        END IF;
        RAISE EXCEPTION 'community deletion manifest key chunks require an unfrozen fenced request'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NOT FOUND
        OR frozen_at IS NULL
        OR request_stage IN ('logically_verified', 'retention_pending')
    THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'community deletion manifest key chunks cannot be removed mid-execution'
        USING ERRCODE = 'integrity_constraint_violation';
END;
$$;
