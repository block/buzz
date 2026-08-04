-- Defer DM visibility for recipients until the first real message.
--
-- Opening a DM (kind:41010) fires on click, before anything is sent. Without
-- this flag the recipient's client immediately renders an empty DM window.
-- Non-creator participants are now created with `hidden_at` set and
-- `hidden_pending = TRUE`; the first stored message flips both off in a single
-- UPDATE (race-free under concurrent first messages). Explicit user hides
-- (kind:41012) keep `hidden_pending = FALSE` so they stay sticky and are never
-- auto-revealed by later messages.
ALTER TABLE channel_members
    ADD COLUMN hidden_pending BOOLEAN NOT NULL DEFAULT FALSE;
