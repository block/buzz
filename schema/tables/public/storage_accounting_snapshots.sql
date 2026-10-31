-- ── Storage accounting snapshot ─────────────────────────────────────────────
-- Deployment-global singleton produced by the isolated S3 accounting worker.

CREATE TABLE storage_accounting_snapshots (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    snapshot JSONB NOT NULL CHECK (jsonb_typeof(snapshot) = 'object'),
    completed_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    duration_ms BIGINT NOT NULL CHECK (duration_ms >= 0),
    max_objects BIGINT NOT NULL CHECK (max_objects > 0),
    code_sha TEXT NOT NULL CHECK (octet_length(code_sha) BETWEEN 1 AND 128)
);
