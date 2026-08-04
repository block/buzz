-- Persist channel-scoped agent behavior so desktop UI, CLI, and external
-- adapters agree on reply placement and DM wake-up policy.

ALTER TABLE channels
    ADD COLUMN agent_reply_mode TEXT NOT NULL DEFAULT 'thread',
    ADD COLUMN dm_require_mention BOOLEAN NOT NULL DEFAULT TRUE,
    ADD CONSTRAINT chk_channels_agent_reply_mode
        CHECK (agent_reply_mode IN ('thread', 'inline'));
