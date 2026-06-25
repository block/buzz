-- Tighten the default channel_add_policy for new agents from 'anyone' to
-- 'owner_only'. Existing rows are unaffected by this migration; the startup
-- clamp in main.rs (BUZZ_AGENT_SHARING_DISABLED) handles retroactive clamping
-- for relays that need it.
ALTER TABLE users
    ALTER COLUMN channel_add_policy SET DEFAULT 'owner_only';
