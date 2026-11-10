SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Validate separately from the write-blocking DDL transaction.
ALTER TABLE workflow_runs
    VALIDATE CONSTRAINT workflow_runs_cause_source_check,
    VALIDATE CONSTRAINT workflow_runs_cause_pubkey_check;
