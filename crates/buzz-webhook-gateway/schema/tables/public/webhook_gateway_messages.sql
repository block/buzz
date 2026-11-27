CREATE TABLE webhook_gateway_messages (
    integration_id UUID NOT NULL,
    delivery_id TEXT NOT NULL CHECK (length(delivery_id) BETWEEN 1 AND 256),
    subscription_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    content TEXT NOT NULL CHECK (length(content) <= 16384),
    state TEXT NOT NULL CHECK (state IN ('queued', 'accepted', 'failed')),
    event_id TEXT CHECK (event_id IS NULL OR length(event_id) = 64),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ,
    last_error TEXT,
    received_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    signed_at TIMESTAMPTZ,
    group_id TEXT,
    group_position BIGINT,
    group_key_action TEXT,
    PRIMARY KEY (integration_id, delivery_id, subscription_id)
);
CREATE INDEX webhook_gateway_messages_due ON webhook_gateway_messages (next_attempt_at) WHERE state = 'queued';
CREATE INDEX webhook_gateway_messages_subscription ON webhook_gateway_messages (subscription_id) WHERE state = 'queued';
CREATE INDEX webhook_gateway_messages_retention ON webhook_gateway_messages (completed_at) WHERE state <> 'queued';
CREATE INDEX webhook_gateway_messages_group_pending ON webhook_gateway_messages (group_id, group_position) WHERE group_id IS NOT NULL AND state = 'queued';
