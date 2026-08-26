-- Durable, payload-free authority for webhook-triggered workflow runs.
-- The body and secret stay in transient trigger context; this table retains
-- only an opaque invocation identity and its tenant/workflow/run binding.
CREATE TABLE workflow_webhook_invocations (
    community_id UUID NOT NULL REFERENCES communities(id),
    invocation_id UUID NOT NULL,
    workflow_id UUID NOT NULL,
    workflow_run_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, invocation_id),
    FOREIGN KEY (community_id, workflow_id)
        REFERENCES workflows (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, workflow_run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE NO ACTION
);

CREATE INDEX idx_workflow_webhook_invocations_created_at
    ON workflow_webhook_invocations (created_at);

SELECT attach_community_write_fence('workflow_webhook_invocations');
