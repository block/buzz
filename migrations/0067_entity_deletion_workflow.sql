-- Durable synthetic user-deletion workflows; existing community state is unchanged.

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

-- Operator-global workflow state. Initially used only by synthetic user demos.
CREATE TABLE entity_deletion_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_type TEXT NOT NULL DEFAULT 'user' CHECK (entity_type = 'user'),
    target TEXT NOT NULL CHECK (target ~ '^demo:[a-z0-9][a-z0-9_-]{0,63}$'),
    status TEXT NOT NULL DEFAULT 'submitted'
        CHECK (status IN ('submitted', 'inventoried', 'approved', 'running', 'demo_completed')),
    requested_by TEXT NOT NULL CHECK (length(trim(requested_by)) BETWEEN 1 AND 256),
    reason TEXT,
    inventory_manifest JSONB,
    inventory_digest BYTEA CHECK (inventory_digest IS NULL OR length(inventory_digest) = 32),
    inventory_frozen_at TIMESTAMPTZ,
    lease_owner TEXT,
    lease_generation BIGINT NOT NULL DEFAULT 0 CHECK (lease_generation >= 0),
    lease_until TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    retry_count INTEGER NOT NULL DEFAULT 0 CHECK (retry_count >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT,
    last_error_at TIMESTAMPTZ,
    blocked_at TIMESTAMPTZ,
    blocked_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    CONSTRAINT entity_deletion_request_shape CHECK (
        ((lease_owner IS NULL) = (lease_until IS NULL))
        AND ((blocked_at IS NULL) = (blocked_reason IS NULL))
        AND ((status = 'demo_completed') = (completed_at IS NOT NULL))
        AND (status <> 'demo_completed' OR lease_owner IS NULL)
        AND ((status = 'submitted') = (inventory_manifest IS NULL))
        AND ((inventory_manifest IS NULL) = (inventory_digest IS NULL))
        AND ((inventory_manifest IS NULL) = (inventory_frozen_at IS NULL))
    ),
    UNIQUE (entity_type, target),
    UNIQUE (id, inventory_digest)
);
CREATE INDEX entity_deletion_requests_runnable
    ON entity_deletion_requests (next_attempt_at, created_at, id)
    WHERE status IN ('approved', 'running') AND blocked_at IS NULL;

-- target_community_id is operator provenance, including synthetic references.
-- It is not tenant data and intentionally has no FK to the communities table.
CREATE TABLE entity_deletion_stage_history (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id UUID NOT NULL REFERENCES entity_deletion_requests(id),
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    target_community_id UUID,
    stage TEXT NOT NULL CHECK (length(stage) BETWEEN 1 AND 64),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'failed', 'completed')),
    is_current BOOLEAN NOT NULL DEFAULT true,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    metadata_json JSONB NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(metadata_json) = 'object'),
    last_error TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT entity_deletion_stage_shape CHECK (
        (is_current = (status <> 'completed'))
        AND ((status = 'completed') = (finished_at IS NOT NULL))
        AND ((status = 'pending') = (started_at IS NULL))
        AND (finished_at IS NULL OR finished_at >= started_at)
    ),
    UNIQUE (request_id, sequence)
);
CREATE UNIQUE INDEX entity_deletion_stage_history_current
    ON entity_deletion_stage_history (request_id) WHERE is_current;

CREATE TABLE entity_deletion_approvals (
    request_id UUID PRIMARY KEY,
    inventory_digest BYTEA NOT NULL CHECK (length(inventory_digest) = 32),
    approved_by TEXT NOT NULL CHECK (length(trim(approved_by)) BETWEEN 1 AND 256),
    approved_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (request_id, inventory_digest)
        REFERENCES entity_deletion_requests(id, inventory_digest)
);

CREATE TABLE entity_deletion_executor_heartbeats (
    executor_id TEXT PRIMARY KEY,
    mode TEXT NOT NULL CHECK (mode IN ('run', 'drain')),
    request_id UUID REFERENCES entity_deletion_requests(id),
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    draining BOOLEAN NOT NULL DEFAULT false,
    stopped_at TIMESTAMPTZ
);

CREATE TRIGGER protect_entity_deletion_request
    BEFORE UPDATE ON entity_deletion_requests
    FOR EACH ROW EXECUTE FUNCTION protect_entity_deletion_request();

CREATE TRIGGER protect_entity_deletion_stage
    BEFORE UPDATE OR DELETE ON entity_deletion_stage_history
    FOR EACH ROW EXECUTE FUNCTION protect_entity_deletion_stage();

CREATE TRIGGER protect_entity_deletion_approval
    BEFORE INSERT OR UPDATE OR DELETE ON entity_deletion_approvals
    FOR EACH ROW EXECUTE FUNCTION protect_entity_deletion_approval();

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('entity_deletion_requests', 'deployment entity deletion workflow and synthetic demo state');

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('entity_deletion_stage_history', 'deployment entity deletion workflow and synthetic demo state');

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('entity_deletion_approvals', 'deployment entity deletion workflow and synthetic demo state');

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('entity_deletion_executor_heartbeats', 'deployment entity deletion workflow and synthetic demo state');
