SET LOCAL lock_timeout = '5s';

-- Synthetic request intake only; no deletion worker or user cleanup.

-- Operator-global intake for synthetic user-deletion requests.
CREATE TABLE entity_deletion_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_type TEXT NOT NULL DEFAULT 'user' CHECK (entity_type = 'user'),
    target TEXT NOT NULL CHECK (target ~ '^demo:[a-z0-9][a-z0-9_-]{0,63}$'),
    status TEXT NOT NULL DEFAULT 'submitted' CHECK (status = 'submitted'),
    requested_by TEXT NOT NULL CHECK (length(trim(requested_by)) BETWEEN 1 AND 256),
    reason TEXT CHECK (octet_length(reason) <= 4096),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (entity_type, target)
);

INSERT INTO _operator_global_tables (table_name, reason)
VALUES ('entity_deletion_requests', 'Operator-global synthetic user-deletion intake')
ON CONFLICT (table_name) DO NOTHING;
