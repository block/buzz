-- Host aliases: map additional hostnames onto an existing community without
-- touching `communities.host`. Conformance: row zero (`resolve_host`) still
-- checks `communities.host` first (crates/buzz-relay/src/tenant.rs); this
-- table is consulted only as a fallback, so an alias can never shadow or
-- outrank a primary host, and it never rewrites `TenantContext::host()` — the
-- request's own host stays authoritative for NIP-98 `u`-URL / NIP-42 `relay`
-- checks.
--
-- Motivating case: in-cluster clients (e.g. Blox workstations) reach the
-- relay through a cluster-local Service DNS name that the Envoy mesh routes
-- by Host header, distinct from the community's externally-facing host. An
-- alias lets that cluster-local host resolve to the same community without
-- renaming the community's canonical host, which would rebind
-- `TenantContext::host()` and break existing NIP-98/NIP-42 checks for
-- external clients still using the original host.
--
-- Like `communities`, this table is listed in `_operator_global_tables`:
-- host lookup must be globally unique across all communities (a host maps to
-- at most one community, alias or primary), so its unique index cannot lead
-- with `community_id`.
CREATE TABLE community_host_aliases (
    host            VARCHAR(255) NOT NULL,
    community_id    UUID NOT NULL REFERENCES communities(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_community_host_aliases_host ON community_host_aliases (lower(host));
CREATE INDEX idx_community_host_aliases_community_id ON community_host_aliases (community_id);

-- Guard both directions so an alias can never collide with a primary
-- `communities.host`, regardless of insertion order: a new/updated alias
-- cannot claim an existing primary host, and a new/renamed community cannot
-- claim an existing alias host. Cross-table checks can't be expressed as a
-- CHECK constraint, hence triggers (mirrors `channels_community_id_immutable`
-- in migration 0001).
CREATE FUNCTION community_host_aliases_no_primary_collision() RETURNS TRIGGER AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM communities WHERE lower(host) = lower(NEW.host)) THEN
        RAISE EXCEPTION 'host % is already a community primary host', NEW.host
            USING ERRCODE = 'unique_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_community_host_aliases_no_primary_collision
    BEFORE INSERT OR UPDATE ON community_host_aliases
    FOR EACH ROW EXECUTE FUNCTION community_host_aliases_no_primary_collision();

CREATE FUNCTION communities_no_alias_collision() RETURNS TRIGGER AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM community_host_aliases WHERE lower(host) = lower(NEW.host)) THEN
        RAISE EXCEPTION 'host % is already a community host alias', NEW.host
            USING ERRCODE = 'unique_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_communities_no_alias_collision
    BEFORE INSERT OR UPDATE ON communities
    FOR EACH ROW EXECUTE FUNCTION communities_no_alias_collision();

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('community_host_aliases', 'host alias map spans communities by design; global host uniqueness is enforced against communities.host via trigger, mirroring the communities registry itself');
