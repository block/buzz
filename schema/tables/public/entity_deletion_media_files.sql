-- Operator-global recovery evidence, retained after community deletion.
-- One logical file per stage; JSON is bounded per file by the application.
CREATE TABLE entity_deletion_media_files (
    stage_id UUID NOT NULL,
    storage_namespace TEXT NOT NULL,
    file_hash TEXT NOT NULL CHECK (file_hash ~ '^[0-9a-f]{64}$'),
    status TEXT NOT NULL DEFAULT 'unknown'
        CHECK (status IN ('unknown', 'dont_delete', 'to_be_deleted', 'deleted')),
    evidence_json JSONB NOT NULL,
    delete_targets_json JSONB NOT NULL DEFAULT '[]'::jsonb
        CHECK (jsonb_typeof(delete_targets_json) = 'array'),
    reserved BOOLEAN NOT NULL DEFAULT false,
    reason TEXT,
    last_error TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (stage_id, storage_namespace, file_hash),
    CHECK (status NOT IN ('to_be_deleted', 'deleted') OR jsonb_array_length(delete_targets_json) > 0)
);
CREATE INDEX entity_deletion_media_reserved_hash_idx
    ON entity_deletion_media_files (file_hash) WHERE reserved;
