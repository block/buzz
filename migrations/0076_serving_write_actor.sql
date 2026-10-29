SET LOCAL lock_timeout = '5s';

-- Additive: legacy community-only writers may leave this NULL. Roll out actor
-- admission to every relay before enabling real user deletion. Existing lease
-- expiry/reaping and whole-community deletion retain their current behavior.
ALTER TABLE community_serving_write_leases ADD COLUMN actor_pubkey TEXT;

CREATE INDEX community_serving_write_leases_actor_active
    ON community_serving_write_leases (actor_pubkey, community_id, lease_until)
    WHERE actor_pubkey IS NOT NULL;
