-- NIP-WH delivery queue and per-subscription outcomes; see
-- schema/tables/public/webhook_deliveries.sql and webhook_records.sql.
-- Their writers use community admission (docs/database.md). New, empty tables.
SET LOCAL lock_timeout = '5s';
CREATE TABLE webhook_deliveries (
    community_id      UUID        NOT NULL,
    delivery_event_id BYTEA       NOT NULL CHECK (length(delivery_event_id) = 32),
    integration_id    UUID        NOT NULL,
    provider          TEXT        NOT NULL,
    event             JSONB       NOT NULL,
    received_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    state             TEXT        NOT NULL DEFAULT 'pending'
                      CHECK (state IN ('pending', 'evaluating', 'done', 'failed')),
    attempts          INT         NOT NULL DEFAULT 0,
    next_attempt_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    claim_id          UUID,
    last_error        TEXT,
    PRIMARY KEY (community_id, delivery_event_id)
);
CREATE INDEX webhook_deliveries_queue ON webhook_deliveries (next_attempt_at)
    WHERE state IN ('pending', 'evaluating');
CREATE INDEX webhook_deliveries_community_queue ON webhook_deliveries (community_id, next_attempt_at)
    WHERE state IN ('pending', 'evaluating');
CREATE INDEX webhook_deliveries_finished ON webhook_deliveries (community_id, received_at)
    WHERE state IN ('done', 'failed');

CREATE TABLE webhook_records (
    community_id          UUID        NOT NULL,
    delivery_event_id     BYTEA       NOT NULL CHECK (length(delivery_event_id) = 32),
    subscription_id       UUID        NOT NULL,
    subscription_revision BYTEA       NOT NULL CHECK (length(subscription_revision) = 32),
    outcome               TEXT        NOT NULL
                          CHECK (outcome IN ('matched', 'error', 'skipped_channel', 'skipped_signer')),
    record_event_id       BYTEA       CHECK (length(record_event_id) = 32),
    last_error            TEXT,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, delivery_event_id, subscription_id)
);
