-- Allow multiple public hostnames to resolve to one durable community.
-- Aliases participate in row-zero host binding and cannot point at an
-- archived community because the resolver checks the parent lifecycle state.
CREATE TABLE community_host_aliases (
    host         VARCHAR(255) PRIMARY KEY,
    community_id UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_community_host_aliases_lower_host
    ON community_host_aliases (lower(host));

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('community_host_aliases', 'operator-managed aliases for the tenant host registry');
