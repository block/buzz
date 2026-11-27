SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '60s';

CREATE TABLE webhook_gateway_message_groups (
    id TEXT PRIMARY KEY,
    last_position BIGINT NOT NULL,
    root_event_id TEXT,
    last_signed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX webhook_gateway_message_groups_retention ON webhook_gateway_message_groups (updated_at);

ALTER TABLE webhook_gateway_messages
    ADD COLUMN group_id TEXT,
    ADD COLUMN group_position BIGINT,
    ADD COLUMN group_key_action TEXT;
CREATE INDEX webhook_gateway_messages_group_pending ON webhook_gateway_messages (group_id, group_position) WHERE group_id IS NOT NULL AND state = 'queued';
CREATE INDEX webhook_gateway_deliveries_pending_order ON webhook_gateway_deliveries (integration_id, received_at, delivery_id) WHERE state = 'queued';
