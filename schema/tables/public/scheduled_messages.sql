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
    PRIMARY KEY (community_id, owner, id),
    UNIQUE (community_id, owner, event_id)
);
CREATE INDEX scheduled_messages_due ON scheduled_messages (next_attempt_at)
    WHERE status = 'scheduled';
CREATE INDEX scheduled_messages_owner ON scheduled_messages (community_id, owner, status, send_at);

CREATE INDEX scheduled_messages_reservation ON scheduled_messages (send_at) WHERE status='scheduled';
