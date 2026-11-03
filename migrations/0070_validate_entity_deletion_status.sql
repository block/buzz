SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Separate from the DDL transaction so validation does not block writers.
ALTER TABLE entity_deletion_requests
    VALIDATE CONSTRAINT entity_deletion_requests_status_check;
