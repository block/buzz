-- Operator-global intake for user-deletion requests.
CREATE TABLE entity_deletion_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_type TEXT NOT NULL DEFAULT 'user' CHECK (entity_type = 'user'),
    target TEXT NOT NULL CHECK (octet_length(target) >= 1 AND octet_length(target) <= 256 AND target = btrim(target)),
    status TEXT NOT NULL DEFAULT 'submitted' CHECK (status IN ('submitted', 'inventoried', 'approved', 'running', 'retry_wait', 'completed')),
    requested_by TEXT NOT NULL CHECK (length(trim(requested_by)) BETWEEN 1 AND 256),
    reason TEXT CHECK (octet_length(reason) <= 4096),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    inventory_manifest JSONB,
    inventory_digest TEXT,
    lease_owner TEXT,
    lease_generation BIGINT NOT NULL DEFAULT 0,
    lease_expires_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,
    retry_count INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ,
    last_error TEXT,
    completed_at TIMESTAMPTZ,
    UNIQUE (entity_type, target)
);
