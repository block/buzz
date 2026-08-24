-- Additional connection hosts for an existing community.
--
-- The primary host remains in communities.host. Aliases are explicit and
-- resolve to the same community id, allowing a deployment to expose both a
-- LAN address and a public tunnel without creating a second tenant.
CREATE TABLE community_host_aliases (
    host         VARCHAR(255) PRIMARY KEY,
    community_id UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_community_host_aliases_host
    ON community_host_aliases (lower(host));

CREATE INDEX idx_community_host_aliases_community
    ON community_host_aliases (community_id);
