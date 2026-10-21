-- target_community_id is operator provenance, including synthetic references.
-- It is not tenant data and intentionally has no FK to the communities table.
CREATE TABLE entity_deletion_stage_history (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id UUID NOT NULL REFERENCES entity_deletion_requests(id),
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    target_community_id UUID,
    stage TEXT NOT NULL CHECK (length(stage) BETWEEN 1 AND 64),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'failed', 'completed')),
    is_current BOOLEAN NOT NULL DEFAULT true,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    metadata_json JSONB NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(metadata_json) = 'object'),
    last_error TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT entity_deletion_stage_shape CHECK (
        (is_current = (status <> 'completed'))
        AND ((status = 'completed') = (finished_at IS NOT NULL))
        AND ((status = 'pending') = (started_at IS NULL))
        AND (finished_at IS NULL OR finished_at >= started_at)
    ),
    UNIQUE (request_id, sequence)
);
CREATE UNIQUE INDEX entity_deletion_stage_history_current
    ON entity_deletion_stage_history (request_id) WHERE is_current;
