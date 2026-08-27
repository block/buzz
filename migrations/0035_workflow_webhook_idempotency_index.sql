-- no-transaction
CREATE UNIQUE INDEX CONCURRENTLY idx_workflow_runs_webhook_idempotency
    ON workflow_runs (community_id, workflow_id, webhook_idempotency_key_hash)
    WHERE webhook_idempotency_key_hash IS NOT NULL;
