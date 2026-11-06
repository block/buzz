SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Keep identity formats in the application, so public keys and account IDs
-- can share this target field. Requests remain restricted to the user entity type.
-- Existing demo requests remain valid.
ALTER TABLE entity_deletion_requests
    DROP CONSTRAINT entity_deletion_requests_target_check,
    ADD CONSTRAINT entity_deletion_requests_target_check
        CHECK (octet_length(target) >= 1 AND octet_length(target) <= 256 AND target = btrim(target)) NOT VALID;
