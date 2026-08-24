-- Owner-authored commands for managed-agent workflow control state.
CREATE TYPE workflow_owner_command_status AS ENUM ('pending_agent', 'applied', 'rejected');

CREATE TABLE workflow_owner_commands (
    community_id UUID NOT NULL REFERENCES communities(id),
    command_id UUID NOT NULL,
    event_id BYTEA NOT NULL,
    owner_pubkey BYTEA NOT NULL,
    agent_pubkey BYTEA NOT NULL,
    workflow_id UUID NOT NULL,
    expected_revision BYTEA NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('update', 'enable', 'disable', 'retire')),
    proposed_yaml TEXT,
    status workflow_owner_command_status NOT NULL,
    resulting_revision BYTEA,
    receipt_event_id BYTEA,
    terminal_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, command_id),
    UNIQUE (community_id, event_id),
    FOREIGN KEY (community_id, workflow_id) REFERENCES workflows (community_id, id) ON DELETE CASCADE,
    CHECK ((operation = 'update') = (proposed_yaml IS NOT NULL)),
    CHECK ((status = 'applied') = (resulting_revision IS NOT NULL))
);

CREATE INDEX idx_workflow_owner_commands_agent_pending
    ON workflow_owner_commands (community_id, agent_pubkey, created_at)
    WHERE status = 'pending_agent';

SELECT attach_community_write_fence('workflow_owner_commands');
