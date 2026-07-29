-- Durable workflow execution ownership. A lease is fencing, not a liveness
-- hint: every worker mutation must present the current token.
ALTER TABLE workflow_runs
    ADD COLUMN IF NOT EXISTS definition_snapshot JSONB,
    ADD COLUMN IF NOT EXISTS definition_hash BYTEA,
    ADD COLUMN IF NOT EXISTS lease_owner TEXT,
    ADD COLUMN IF NOT EXISTS lease_token UUID,
    ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS heartbeat_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS attempt INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS available_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ADD COLUMN IF NOT EXISTS recovery_classification TEXT;

CREATE INDEX IF NOT EXISTS idx_workflow_runs_claimable
    ON workflow_runs (community_id, available_at, created_at)
    WHERE status IN ('pending', 'waiting_approval', 'failed', 'running');

CREATE INDEX IF NOT EXISTS idx_workflow_runs_lease_expiry
    ON workflow_runs (lease_expires_at)
    WHERE status = 'running';

CREATE UNIQUE INDEX IF NOT EXISTS uq_workflow_runs_trigger_identity
    ON workflow_runs (community_id, workflow_id, trigger_event_id)
    WHERE trigger_event_id IS NOT NULL;
