SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- What caused each run. NULL on rows written before this column existed; such
-- runs are never eligible to direct agents.
ALTER TABLE workflow_runs
    ADD COLUMN trigger_source TEXT
        CHECK (trigger_source IN ('event', 'schedule', 'webhook', 'manual')),
    ADD COLUMN cause_pubkey BYTEA
        CHECK (length(cause_pubkey) = 32),
    ADD CONSTRAINT workflow_runs_cause_pubkey_matches_source
        CHECK ((trigger_source IN ('event', 'manual')) = (cause_pubkey IS NOT NULL));
