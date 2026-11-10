SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Validate separately from the write-blocking DDL transaction.
ALTER TABLE entity_deletion_requests
    VALIDATE CONSTRAINT entity_deletion_requests_target_check;
