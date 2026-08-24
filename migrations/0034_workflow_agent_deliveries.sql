-- Durable handoff from a workflow send_message step to a managed agent.
CREATE TYPE workflow_agent_delivery_status AS ENUM ('pending', 'claimed', 'delivered', 'failed', 'expired');

CREATE TABLE workflow_agent_deliveries (
    community_id UUID NOT NULL REFERENCES communities(id),
    id UUID NOT NULL,
    workflow_id UUID NOT NULL,
    run_id UUID NOT NULL,
    step_id VARCHAR(64) NOT NULL,
    definition_event_id BYTEA NOT NULL,
    message_event_id BYTEA NOT NULL,
    message_event_created_at TIMESTAMPTZ NOT NULL,
    channel_id UUID NOT NULL,
    target_pubkey BYTEA NOT NULL,
    status workflow_agent_delivery_status NOT NULL DEFAULT 'pending',
    attempt INT NOT NULL DEFAULT 0 CHECK (attempt BETWEEN 0 AND 3),
    claim_token UUID,
    claim_owner BYTEA,
    claim_expires_at TIMESTAMPTZ,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    delivered_at TIMESTAMPTZ,
    failed_at TIMESTAMPTZ,
    failure_code TEXT,
    failure_message TEXT,
    -- Immutable private execution snapshot used to verify rendering.
    execution_trace JSONB NOT NULL,
    trigger_context JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, id),
    UNIQUE (community_id, run_id, step_id, target_pubkey),
    FOREIGN KEY (community_id, workflow_id) REFERENCES workflows (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, run_id) REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, message_event_created_at, message_event_id)
        REFERENCES events (community_id, created_at, id) ON DELETE CASCADE
);

CREATE INDEX idx_workflow_agent_deliveries_pending
    ON workflow_agent_deliveries (community_id, target_pubkey, next_attempt_at, created_at)
    WHERE status IN ('pending', 'claimed');
CREATE INDEX idx_workflow_agent_deliveries_run
    ON workflow_agent_deliveries (community_id, run_id, step_id);

SELECT attach_community_write_fence('workflow_agent_deliveries');
