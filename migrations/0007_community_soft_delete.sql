-- Soft-delete communities while allowing a deleted hostname to be provisioned
-- as a new, isolated tenant. Child rows remain attached to the deleted UUID.
ALTER TABLE communities
    ADD COLUMN deleted_at TIMESTAMPTZ;

DROP INDEX idx_communities_host;

CREATE UNIQUE INDEX idx_communities_host
    ON communities (lower(host))
    WHERE deleted_at IS NULL;
