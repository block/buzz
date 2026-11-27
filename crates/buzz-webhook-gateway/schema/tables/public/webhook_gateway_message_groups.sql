CREATE TABLE webhook_gateway_message_groups (
    id TEXT PRIMARY KEY,
    last_position BIGINT NOT NULL,
    root_event_id TEXT,
    last_signed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX webhook_gateway_message_groups_retention ON webhook_gateway_message_groups (updated_at);
