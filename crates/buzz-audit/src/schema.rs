/// DDL for the `audit_log` table. Passed to [`sqlx::raw_sql`] on startup.
pub const AUDIT_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS audit_log (
    seq          BIGINT       NOT NULL PRIMARY KEY,
    timestamp    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    event_id     VARCHAR(255) NOT NULL,
    event_kind   INT          NOT NULL,
    actor_pubkey VARCHAR(255) NOT NULL,
    action       VARCHAR(64)  NOT NULL,
    channel_id   BYTEA,
    metadata     JSONB        NOT NULL,
    prev_hash    VARCHAR(64)  NOT NULL,
    hash         VARCHAR(64)  NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_log_timestamp ON audit_log (timestamp);
CREATE INDEX IF NOT EXISTS idx_audit_log_actor ON audit_log (actor_pubkey);
CREATE INDEX IF NOT EXISTS idx_audit_log_channel ON audit_log (channel_id);
"#;
