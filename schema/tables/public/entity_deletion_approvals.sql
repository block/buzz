-- Operator-global approval evidence; application transactions check the request.
CREATE TABLE entity_deletion_approvals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id UUID NOT NULL,
    inventory_digest TEXT NOT NULL CHECK (inventory_digest ~ '^[0-9a-f]{64}$'),
    approved_by TEXT NOT NULL CHECK (approved_by = btrim(approved_by) AND octet_length(approved_by) >= 1 AND octet_length(approved_by) <= 256),
    approved_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (request_id, inventory_digest, approved_by)
);
