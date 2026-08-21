-- Additional authorities that resolve to an existing community.
CREATE TABLE community_hosts (
    community_id UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    host VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, host),
    CONSTRAINT chk_community_hosts_host_not_empty CHECK (btrim(host) <> '')
);

CREATE UNIQUE INDEX idx_community_hosts_lower_host ON community_hosts (lower(host));

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('community_hosts', 'tenant authority aliases; community_id identifies the target tenant but host uniqueness is deployment-wide');

-- Serialize writes for a normalized authority and prevent it from appearing
-- in both the canonical and alias maps, including under concurrent writers.
CREATE FUNCTION guard_community_host_collision() RETURNS trigger AS $$
BEGIN
    PERFORM pg_advisory_xact_lock(hashtextextended(lower(NEW.host), 0));
    IF TG_TABLE_NAME = 'communities' THEN
        IF EXISTS (SELECT 1 FROM community_hosts WHERE lower(host) = lower(NEW.host)) THEN
            RAISE EXCEPTION 'community host already exists as an alias: %', NEW.host
                USING ERRCODE = 'unique_violation';
        END IF;
    ELSIF EXISTS (SELECT 1 FROM communities WHERE lower(host) = lower(NEW.host)) THEN
        RAISE EXCEPTION 'community host already exists as a canonical host: %', NEW.host
            USING ERRCODE = 'unique_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER guard_communities_host_collision
BEFORE INSERT OR UPDATE OF host ON communities
FOR EACH ROW EXECUTE FUNCTION guard_community_host_collision();

CREATE TRIGGER guard_community_hosts_collision
BEFORE INSERT OR UPDATE OF host ON community_hosts
FOR EACH ROW EXECUTE FUNCTION guard_community_host_collision();
