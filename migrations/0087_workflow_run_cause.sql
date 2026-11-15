SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- What caused each run. NULL on rows written before these columns existed;
-- such runs are never eligible to direct agents, and neither is a stored
-- source without the pubkey it requires, or with one it forbids. The checks are added NOT VALID
-- so this write-blocking DDL does not scan existing rows; 0088 validates them.
ALTER TABLE workflow_runs
    ADD COLUMN cause_source TEXT,
    ADD COLUMN cause_pubkey BYTEA,
    ADD CONSTRAINT workflow_runs_cause_source_check
        CHECK (cause_source IN ('event', 'schedule', 'webhook', 'manual')) NOT VALID,
    ADD CONSTRAINT workflow_runs_cause_pubkey_check
        CHECK (length(cause_pubkey) = 32) NOT VALID;
