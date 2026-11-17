-- Retire the last foreign key in the gateway schema (docs/database.md Rule 2).
-- The gateway checks the reference in the writing transaction instead:
-- upsert_delegation locks the installation FOR UPDATE and rejects a missing or
-- revoked one, every installation delete removes its delegations in the same
-- transaction, and delivery rejects a delegation whose installation is gone.
-- Released gateways already do this. Their reaper can leave one already
-- expired delegation behind in a narrow race. Delivery cannot use it, and the
-- next reap deletes it, so they stay safe against this schema while it rolls out.
SET LOCAL lock_timeout = '5s';
ALTER TABLE push_gateway_delegations
    DROP CONSTRAINT push_gateway_delegations_installation_id_fkey;
