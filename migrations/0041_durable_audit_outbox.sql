-- Durable, fair handoff between accepted relay operations and the per-community
-- audit hash chain. Producers commit an intent here before replying; workers
-- claim only the oldest due row in each community, so one contended chain does
-- not block unrelated tenants. The worker appends audit_log and deletes this
-- row in one transaction.

-- Logical producer keys survive outbox delivery. This makes retry repair
-- idempotent both while an intent is pending and after it has been appended.
CREATE TABLE audit_delivery_keys (
    community_id UUID NOT NULL REFERENCES communities(id),
    dedupe_key   TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, dedupe_key)
);

CREATE TABLE audit_outbox (
    community_id   UUID NOT NULL REFERENCES communities(id),
    id             UUID NOT NULL DEFAULT gen_random_uuid(),
    enqueue_seq    BIGINT GENERATED ALWAYS AS IDENTITY,
    action         VARCHAR(64) NOT NULL,
    actor_pubkey   BYTEA,
    object_id      TEXT,
    detail         JSONB NOT NULL,
    enqueued_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    attempt_count  INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    PRIMARY KEY (community_id, id)
);

CREATE INDEX idx_audit_outbox_due
    ON audit_outbox (next_attempt_at, enqueue_seq);

CREATE INDEX idx_audit_outbox_community_order
    ON audit_outbox (community_id, enqueue_seq);

SELECT attach_community_write_fence('audit_outbox');
SELECT attach_community_write_fence('audit_delivery_keys');
