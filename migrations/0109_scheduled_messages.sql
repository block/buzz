SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
CREATE TABLE scheduled_messages (
    community_id UUID NOT NULL,
    owner BYTEA NOT NULL,
    id UUID NOT NULL,
    revision BIGINT NOT NULL DEFAULT 1,
    event JSONB NOT NULL,
    event_id BYTEA NOT NULL,
    send_at BIGINT NOT NULL,
    next_attempt_at BIGINT NOT NULL,
    status TEXT NOT NULL DEFAULT 'scheduled',
    attempts INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    prune_at TIMESTAMPTZ,
    admission_issuer TEXT,
    admission_owner BYTEA,
    admission_issued_second BIGINT,
    admitted_at TIMESTAMPTZ,
    PRIMARY KEY (community_id, owner, id),
    UNIQUE (community_id, owner, event_id)
);
CREATE INDEX scheduled_messages_due ON scheduled_messages (next_attempt_at)
    WHERE status = 'scheduled';
CREATE INDEX scheduled_messages_owner ON scheduled_messages (community_id, owner, status, send_at);

CREATE INDEX scheduled_messages_federated_pending ON scheduled_messages (admission_issuer, owner)
    WHERE status = 'scheduled' AND admission_issuer IS NOT NULL;
CREATE INDEX scheduled_messages_federated_agent_pending ON scheduled_messages (admission_issuer, admission_owner)
    WHERE status = 'scheduled' AND admission_owner IS NOT NULL;

CREATE INDEX scheduled_messages_terminal_expiry ON scheduled_messages (prune_at) WHERE prune_at IS NOT NULL;

CREATE TABLE scheduled_message_disconnects (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    issuer TEXT NOT NULL,
    target BYTEA NOT NULL,
    revoke_through BIGINT,
    recorded_at TIMESTAMPTZ NOT NULL,
    deny_until TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX scheduled_message_disconnects_target ON scheduled_message_disconnects (issuer, target);
CREATE INDEX scheduled_message_disconnects_expiry ON scheduled_message_disconnects (issuer, expires_at);
