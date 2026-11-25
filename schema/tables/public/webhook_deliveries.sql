-- ── Webhook deliveries ───────────────────────────────────────────────────────
-- NIP-WH deliveries (kind 20701) accepted from a configured ingress gateway.
-- A delivery never enters `events`, so no read path can serve it. Keyed by the
-- signed event ID, so a gateway retry of the same event is a duplicate.
-- `pending` and `evaluating` rows are due at `next_attempt_at`; a claim moves it
-- to the end of the claim's lease, so an expired lease is due again. Finished
-- rows are deleted after the retention period.

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
