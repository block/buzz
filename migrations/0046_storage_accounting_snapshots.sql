-- Last complete media-storage accounting result produced by the isolated
-- buzz-admin worker. The worker replaces this singleton row only after the
-- full S3 listing and fold succeed, so relay readers never observe a partial
-- snapshot.
CREATE TABLE storage_accounting_snapshots (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    snapshot JSONB NOT NULL CHECK (jsonb_typeof(snapshot) = 'object'),
    completed_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    duration_ms BIGINT NOT NULL CHECK (duration_ms >= 0),
    max_objects BIGINT NOT NULL CHECK (max_objects > 0),
    code_sha TEXT NOT NULL CHECK (octet_length(code_sha) BETWEEN 1 AND 128)
);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('storage_accounting_snapshots', 'deployment-global completed media accounting handoff');
