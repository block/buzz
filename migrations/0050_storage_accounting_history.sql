-- Append-only per-community history of completed media-storage accounting
-- runs. The singleton in storage_accounting_snapshots keeps only the latest
-- fold, which makes usage-over-time (metering, billing, trend) queries
-- impossible; this table records one row per community per completed run,
-- written in the same transaction as the singleton replacement so history
-- always corresponds to a snapshot that actually published.
--
-- community_id is provenance, not ownership (the product_feedback precedent):
-- the accounting worker derives community UUIDs from S3 sidecar keys, which
-- may reference communities the relay database no longer knows, so there is
-- deliberately no foreign key. Community deletion severs provenance
-- (community_id = NULL) instead of purging rows, keeping fleet-level history
-- intact; see the exclusion below and the deletion executor's
-- clear-provenance step.
CREATE TABLE storage_accounting_history (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    completed_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    community_id UUID,
    logical_bytes BIGINT NOT NULL CHECK (logical_bytes >= 0),
    logical_objects BIGINT NOT NULL CHECK (logical_objects >= 0),
    code_sha TEXT NOT NULL CHECK (octet_length(code_sha) BETWEEN 1 AND 128)
);

CREATE INDEX idx_storage_accounting_history_community
    ON storage_accounting_history (community_id, completed_at DESC);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('storage_accounting_history', 'deployment-global per-community storage accounting history; community_id is provenance only');

-- Keep community deletion's exact-catalog validation green: tables carrying
-- community_id must either be fenced tenant tables (purged with the tenant)
-- or listed here. History is operator accounting evidence that outlives any
-- one tenant, exactly like product_feedback and rate_limit_violations.
CREATE OR REPLACE FUNCTION community_write_fence_excluded_table(target NAME) RETURNS BOOLEAN
LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT target::TEXT = ANY (ARRAY[
        'community_deletion_requests', 'community_deletion_approvals',
        'community_deletion_checkpoints', 'community_serving_write_leases',
        'community_deletion_executor_heartbeats', 'product_feedback',
        'rate_limit_violations', 'storage_accounting_history'
    ]::TEXT[])
$$;
