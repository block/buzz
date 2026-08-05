-- O4 has no authority model for community archive, restore, or physical
-- teardown. Keep those transitions unavailable for activated Enforce domains
-- so a stale database restore cannot resurrect a protected community. Off and
-- observational communities retain their existing lifecycle behavior.

-- The protected-domain marker must exist before this migration installs the
-- lifecycle guard. Later invalidation migrations extend this durable marker;
-- they must not be prerequisites for the guard that protects it.
CREATE TABLE authorization_invalidation_domains (
    community_id UUID NOT NULL REFERENCES communities(id),
    generation   BIGINT NOT NULL DEFAULT 0 CHECK (generation >= 0),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id)
);

CREATE FUNCTION deny_unwitnessed_protected_community_lifecycle() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM authorization_invalidation_domains
        WHERE community_id = OLD.id
    ) THEN
        IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'protected community lifecycle requires a separately witnessed authority model'
                USING ERRCODE = 'check_violation';
        END IF;
        IF NEW.archived_at IS DISTINCT FROM OLD.archived_at THEN
            RAISE EXCEPTION 'protected community lifecycle requires a separately witnessed authority model'
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER protected_community_lifecycle_guard
    BEFORE UPDATE OF archived_at OR DELETE ON communities
    FOR EACH ROW EXECUTE FUNCTION deny_unwitnessed_protected_community_lifecycle();
