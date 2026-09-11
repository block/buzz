-- Persist the authoritative enforcement target with the action row so that
-- crash-recovery can fire live side effects without re-deriving from mutable
-- sources (event rows, report rows) that may have changed or been purged.
--
-- enforcement_target_pubkey: the resolved target pubkey bytes at claim time,
--   when the action targets a pubkey (kick/ban/timeout). NULL for event/blob
--   targets where no pubkey is derived.
-- enforcement_channel_id: the channel the enforcement targets. Populated for
--   kick actions. NULL for community-wide actions.
--
-- Both columns mirror the values passed to claim_report's target_pubkey and
-- channel_id parameters. They are written once at claim time and never updated.

ALTER TABLE relay_admin_actions
    ADD COLUMN enforcement_target_pubkey BYTEA
        CHECK (enforcement_target_pubkey IS NULL OR length(enforcement_target_pubkey) = 32),
    ADD COLUMN enforcement_channel_id UUID;
