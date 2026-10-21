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
