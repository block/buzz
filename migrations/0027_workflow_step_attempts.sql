CREATE TABLE IF NOT EXISTS workflow_step_attempts (
    community_id       UUID NOT NULL,
    run_id              UUID NOT NULL,
    step_index          INTEGER NOT NULL,
    step_id             VARCHAR(64) NOT NULL,
    attempt             INTEGER NOT NULL,
    idempotency_key     TEXT NOT NULL,
    status              TEXT NOT NULL,
    started_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at        TIMESTAMPTZ,
    result              JSONB,
    error_message       TEXT,
    recovery_classification TEXT,
    PRIMARY KEY (community_id, run_id, step_index, attempt),
    UNIQUE (community_id, idempotency_key),
    FOREIGN KEY (community_id, run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workflow_step_attempts_recovery
    ON workflow_step_attempts (community_id, status, started_at);

CREATE TABLE IF NOT EXISTS workflow_action_effects (
    community_id   UUID NOT NULL,
    run_id         UUID NOT NULL,
    idempotency_key TEXT NOT NULL,
    event_id       BYTEA,
    status         TEXT NOT NULL DEFAULT 'reserved',
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, idempotency_key),
    FOREIGN KEY (community_id, run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE
);
