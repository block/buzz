-- ── Audit log ─────────────────────────────────────────────────────────────────
-- Conformance: "Audit log and observability". Per-community hash chain:
-- uniqueness (community_id, seq) and (community_id, hash). One chain per tenant.
-- (Lane Audit/Dawn builds the chain logic; Lane 0 fixes the scoped schema.)

CREATE TABLE audit_log (
    community_id    UUID NOT NULL REFERENCES communities(id),
    seq             BIGINT NOT NULL,
    hash_version    SMALLINT NOT NULL DEFAULT 1 CHECK (hash_version IN (1, 2)),
    hash            BYTEA NOT NULL,
    prev_hash       BYTEA,
    action          VARCHAR(64) NOT NULL,
    actor_pubkey    BYTEA,
    object_id       TEXT,
    detail          JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, seq)
);

CREATE UNIQUE INDEX idx_audit_log_hash ON audit_log (community_id, hash);
