-- ── Workflow waits ───────────────────────────────────────────────────────────

CREATE TABLE workflow_waits (
    community_id UUID NOT NULL,
    run_id       UUID NOT NULL,
    message_id   TEXT NOT NULL,
    wait_on      TEXT NOT NULL,
    emoji        TEXT,
    expires_at   TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, run_id)
);

CREATE INDEX idx_workflow_waits_message ON workflow_waits (community_id, message_id);
