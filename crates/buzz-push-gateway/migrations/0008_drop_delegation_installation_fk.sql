-- Retire the last foreign key in the gateway schema (docs/database.md Rule 2).
-- The gateway checks the reference in the writing transaction instead:
-- upsert_delegation locks the installation FOR UPDATE and rejects a missing or
-- revoked one, the reaper locks expired installations before it deletes them
-- and their delegations (#9627), and delivery rejects a delegation whose
-- installation is gone. Every gateway and rollback target must run #9627 or
-- later before this migration runs; an older reaper can leave an orphaned
-- delegation once the foreign key is gone.
SET LOCAL lock_timeout = '5s';
ALTER TABLE push_gateway_delegations
    DROP CONSTRAINT push_gateway_delegations_installation_id_fkey;
