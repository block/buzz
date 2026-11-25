-- ── Webhook records ──────────────────────────────────────────────────────────
-- One row per (delivery, subscription) the worker has decided: the revision it
-- evaluated, and the signed record event when the filter matched. Written in
-- the transaction that stores the record, so a retry fans out the same event
-- instead of signing a new one.

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
