-- Operator-global workflow history. Retained after community deletion as
-- orchestration evidence; target_community_id is provenance, not tenant ownership.
CREATE TABLE entity_deletion_stage_history (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id UUID NOT NULL,
    target_community_id UUID,
    stage TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'failed', 'completed')),
    is_current BOOLEAN NOT NULL DEFAULT true,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    metadata_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_error TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX entity_deletion_one_current_stage
    ON entity_deletion_stage_history (request_id) WHERE is_current = true;
CREATE INDEX entity_deletion_stage_history_request_idx
    ON entity_deletion_stage_history (request_id);
