-- ── Workflow runs ─────────────────────────────────────────────────────────────

CREATE TABLE workflow_runs (
    community_id        UUID NOT NULL REFERENCES communities(id),
    id                  UUID NOT NULL DEFAULT gen_random_uuid(),
    workflow_id         UUID NOT NULL,
    status              run_status NOT NULL DEFAULT 'pending',
    trigger_event_id    BYTEA,
    current_step        INT NOT NULL DEFAULT 0,
    execution_trace     JSONB NOT NULL DEFAULT '[]',
    trigger_context     JSONB,
    trigger_source      TEXT
        CHECK (trigger_source IN ('event', 'schedule', 'webhook', 'manual')),
    cause_pubkey        BYTEA
        CHECK (length(cause_pubkey) = 32),
    started_at          TIMESTAMPTZ,
    completed_at        TIMESTAMPTZ,
    error_message       TEXT,
    error_code          TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, id),
    CONSTRAINT workflow_runs_cause_pubkey_matches_source
        CHECK ((trigger_source IN ('event', 'manual')) = (cause_pubkey IS NOT NULL)),
    FOREIGN KEY (community_id, workflow_id)
        REFERENCES workflows (community_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_workflow_runs_workflow ON workflow_runs (community_id, workflow_id);
CREATE INDEX idx_workflow_runs_status ON workflow_runs (community_id, status);
