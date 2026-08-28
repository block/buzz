-- Durable child state for multi-agent workflow runs.
--
-- Existing workflow_runs remains the lifecycle authority. These tables add
-- phase CAS, idempotent tasks, versioned artifacts, resumable checkpoints,
-- and an append-only transition ledger without changing legacy run semantics.

CREATE TABLE workflow_run_state (
    community_id        UUID NOT NULL REFERENCES communities(id),
    run_id              UUID NOT NULL,
    phase               VARCHAR(128) NOT NULL DEFAULT 'created',
    state_version       BIGINT NOT NULL DEFAULT 0,
    manifest_hash       BYTEA,
    thread_root_event_id BYTEA,
    deadline            TIMESTAMPTZ,
    metadata            JSONB NOT NULL DEFAULT '{}',
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, run_id),
    FOREIGN KEY (community_id, run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE,
    CONSTRAINT chk_workflow_run_state_version CHECK (state_version >= 0),
    CONSTRAINT chk_workflow_run_manifest_hash CHECK (manifest_hash IS NULL OR LENGTH(manifest_hash) = 32),
    CONSTRAINT chk_workflow_run_thread_root CHECK (thread_root_event_id IS NULL OR LENGTH(thread_root_event_id) = 32)
);

CREATE TABLE workflow_run_tasks (
    community_id        UUID NOT NULL REFERENCES communities(id),
    id                  UUID NOT NULL DEFAULT gen_random_uuid(),
    run_id              UUID NOT NULL,
    task_key            VARCHAR(128) NOT NULL,
    phase               VARCHAR(128) NOT NULL,
    agent_pubkey        BYTEA,
    status              VARCHAR(32) NOT NULL DEFAULT 'pending',
    attempt             INT NOT NULL DEFAULT 0,
    max_attempts        INT NOT NULL DEFAULT 1,
    input               JSONB NOT NULL DEFAULT '{}',
    output_schema       JSONB,
    idempotency_key     VARCHAR(255) NOT NULL,
    parent_task_id      UUID,
    depends_on          JSONB NOT NULL DEFAULT '[]',
    not_before          TIMESTAMPTZ,
    started_at          TIMESTAMPTZ,
    completed_at        TIMESTAMPTZ,
    error_code          TEXT,
    error_message       TEXT,
    version             BIGINT NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, id),
    UNIQUE (community_id, run_id, id),
    UNIQUE (community_id, run_id, task_key),
    UNIQUE (community_id, run_id, idempotency_key),
    FOREIGN KEY (community_id, run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, run_id, parent_task_id)
        REFERENCES workflow_run_tasks (community_id, run_id, id) ON DELETE NO ACTION,
    CONSTRAINT chk_workflow_task_agent CHECK (agent_pubkey IS NULL OR LENGTH(agent_pubkey) = 32),
    CONSTRAINT chk_workflow_task_status CHECK (status IN ('pending','assigned','running','waiting','retry_scheduled','completed','failed','cancelled','blocked')),
    CONSTRAINT chk_workflow_task_attempts CHECK (attempt >= 0 AND max_attempts > 0 AND attempt <= max_attempts),
    CONSTRAINT chk_workflow_task_version CHECK (version >= 0),
    CONSTRAINT chk_workflow_task_dependencies CHECK (jsonb_typeof(depends_on) = 'array')
);

CREATE TABLE workflow_run_artifacts (
    community_id        UUID NOT NULL REFERENCES communities(id),
    id                  UUID NOT NULL DEFAULT gen_random_uuid(),
    run_id              UUID NOT NULL,
    task_id             UUID,
    kind                VARCHAR(128) NOT NULL,
    version             INT NOT NULL DEFAULT 1,
    content_type        VARCHAR(255) NOT NULL,
    uri                 TEXT,
    sha256              BYTEA NOT NULL,
    inline_content      JSONB,
    metadata            JSONB NOT NULL DEFAULT '{}',
    created_by          BYTEA,
    idempotency_key     VARCHAR(255) NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, id),
    UNIQUE (community_id, run_id, id),
    UNIQUE (community_id, run_id, idempotency_key),
    FOREIGN KEY (community_id, run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, run_id, task_id)
        REFERENCES workflow_run_tasks (community_id, run_id, id) ON DELETE CASCADE,
    CONSTRAINT chk_workflow_artifact_version CHECK (version > 0),
    CONSTRAINT chk_workflow_artifact_hash CHECK (LENGTH(sha256) = 32),
    CONSTRAINT chk_workflow_artifact_creator CHECK (created_by IS NULL OR LENGTH(created_by) = 32),
    CONSTRAINT chk_workflow_artifact_payload CHECK (uri IS NOT NULL OR inline_content IS NOT NULL)
);
CREATE UNIQUE INDEX idx_workflow_artifact_task_version
    ON workflow_run_artifacts (community_id, run_id, task_id, kind, version)
    WHERE task_id IS NOT NULL;

CREATE TABLE workflow_run_checkpoints (
    community_id        UUID NOT NULL REFERENCES communities(id),
    id                  UUID NOT NULL DEFAULT gen_random_uuid(),
    run_id              UUID NOT NULL,
    task_id             UUID NOT NULL,
    sequence            BIGINT NOT NULL,
    state               JSONB NOT NULL,
    artifact_id         UUID,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, id),
    UNIQUE (community_id, run_id, task_id, sequence),
    FOREIGN KEY (community_id, run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, run_id, task_id)
        REFERENCES workflow_run_tasks (community_id, run_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, run_id, artifact_id)
        REFERENCES workflow_run_artifacts (community_id, run_id, id) ON DELETE NO ACTION,
    CONSTRAINT chk_workflow_checkpoint_sequence CHECK (sequence >= 0)
);

CREATE TABLE workflow_run_transitions (
    community_id        UUID NOT NULL REFERENCES communities(id),
    id                  UUID NOT NULL DEFAULT gen_random_uuid(),
    run_id              UUID NOT NULL,
    sequence            BIGINT NOT NULL,
    from_phase          VARCHAR(128),
    to_phase            VARCHAR(128) NOT NULL,
    from_status         VARCHAR(32),
    to_status           VARCHAR(32) NOT NULL,
    reason              TEXT,
    actor_pubkey        BYTEA,
    metadata            JSONB NOT NULL DEFAULT '{}',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, id),
    UNIQUE (community_id, run_id, sequence),
    FOREIGN KEY (community_id, run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE,
    CONSTRAINT chk_workflow_transition_sequence CHECK (sequence >= 0),
    CONSTRAINT chk_workflow_transition_actor CHECK (actor_pubkey IS NULL OR LENGTH(actor_pubkey) = 32)
);

CREATE INDEX idx_workflow_tasks_run_status ON workflow_run_tasks (community_id, run_id, status);
CREATE INDEX idx_workflow_tasks_agent_status ON workflow_run_tasks (community_id, agent_pubkey, status) WHERE agent_pubkey IS NOT NULL;
CREATE INDEX idx_workflow_artifacts_run ON workflow_run_artifacts (community_id, run_id, created_at);
CREATE INDEX idx_workflow_checkpoints_task ON workflow_run_checkpoints (community_id, run_id, task_id, sequence DESC);
CREATE INDEX idx_workflow_transitions_run ON workflow_run_transitions (community_id, run_id, sequence);

SELECT attach_community_write_fence('workflow_run_state');
SELECT attach_community_write_fence('workflow_run_tasks');
SELECT attach_community_write_fence('workflow_run_artifacts');
SELECT attach_community_write_fence('workflow_run_checkpoints');
SELECT attach_community_write_fence('workflow_run_transitions');
