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
