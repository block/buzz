CREATE INDEX idx_users_agent_owner
    ON users (community_id, agent_owner_pubkey)
    WHERE agent_owner_pubkey IS NOT NULL;
