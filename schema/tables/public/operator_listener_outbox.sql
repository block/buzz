CREATE TABLE operator_listener_outbox (
    id                UUID NOT NULL PRIMARY KEY DEFAULT gen_random_uuid(),
    listener_pubkey   BYTEA NOT NULL CHECK (length(listener_pubkey) = 32),
    target_pubkey     BYTEA NOT NULL CHECK (length(target_pubkey) = 32),
    community_id      UUID NOT NULL,
    event_id          BYTEA NOT NULL CHECK (length(event_id) = 32),
    event_kind        INTEGER NOT NULL,
    event_created_at  TIMESTAMPTZ NOT NULL,
    state             TEXT NOT NULL DEFAULT 'pending'
                      CHECK (state IN ('pending', 'sending')),
    attempts          INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_until       TIMESTAMPTZ,
    claim_id          UUID,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (listener_pubkey, target_pubkey, community_id, event_id)
);
CREATE INDEX operator_listener_outbox_due
    ON operator_listener_outbox (next_attempt_at, created_at, id)
    WHERE state = 'pending';
CREATE INDEX operator_listener_outbox_recovery
    ON operator_listener_outbox (lease_until, created_at, id)
    WHERE state = 'sending';
CREATE INDEX operator_listener_outbox_created_at
    ON operator_listener_outbox (created_at);
