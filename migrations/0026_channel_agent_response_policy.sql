-- Per-channel agent response policy. Mention-only preserves the existing
-- behavior for every current channel; owners/admins may opt a channel into
-- delivering untagged messages to all agent members.
ALTER TABLE channels
    ADD COLUMN agent_response_policy TEXT NOT NULL DEFAULT 'mentions',
    ADD CONSTRAINT channels_agent_response_policy_check
        CHECK (agent_response_policy IN ('mentions', 'all'));
