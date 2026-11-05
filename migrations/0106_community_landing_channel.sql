-- The community's landing channel and its custom welcome message.
-- Only a relay owner or admin sets them, with kind:9002 ["landing", "true"]
-- and ["welcome", text] on the channel; the relay also adopts or creates a
-- `general` landing channel on its own when none is designated. One row per
-- community holds the designation, so "exactly one landing channel" holds by
-- construction and a move is a single-row update. NIP-11 `landing_channel` is
-- the authoritative read; the landing channel's kind:39000 carries a bare
-- `landing` tag (and `welcome` when customized) as a projection.
-- Both columns are nullable with no default, so existing rows need no
-- rewrite and the deployed relay, which never reads them, is unaffected.
SET LOCAL lock_timeout = '5s';

ALTER TABLE communities
    ADD COLUMN landing_channel_id UUID,
    ADD COLUMN welcome_message TEXT;
