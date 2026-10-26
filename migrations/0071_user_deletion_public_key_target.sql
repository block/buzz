SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Expand intake to canonical public keys. Existing demo requests remain valid.
ALTER TABLE entity_deletion_requests
    DROP CONSTRAINT entity_deletion_requests_target_check,
    ADD CONSTRAINT entity_deletion_requests_target_check
        CHECK (target ~ '^demo:[a-z0-9][a-z0-9_-]{0,63}$' OR target ~ '^[0-9a-f]{64}$') NOT VALID;
