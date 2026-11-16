-- Workflows a retirement moved from active to archived, with the updated_at it
-- wrote, so release restores only workflows unchanged since then.
CREATE TABLE identity_retirement_workflows (
    community_id    UUID NOT NULL,
    workflow_id     UUID NOT NULL,
    owner_pubkey    BYTEA NOT NULL CHECK (length(owner_pubkey) = 32),
    action_id       UUID NOT NULL,
    archived_at     TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, workflow_id)
);

CREATE INDEX idx_identity_retirement_workflows_owner
    ON identity_retirement_workflows (community_id, owner_pubkey);
