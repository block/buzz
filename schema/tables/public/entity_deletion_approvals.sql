CREATE TABLE entity_deletion_approvals (
    request_id UUID PRIMARY KEY,
    inventory_digest BYTEA NOT NULL CHECK (length(inventory_digest) = 32),
    approved_by TEXT NOT NULL CHECK (length(trim(approved_by)) BETWEEN 1 AND 256),
    approved_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (request_id, inventory_digest)
        REFERENCES entity_deletion_requests(id, inventory_digest)
);
