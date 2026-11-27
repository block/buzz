-- Channel subscriptions and their per-channel message outbox. The gateway
-- fans each delivery out to the subscriptions of its integration and sends
-- one kind-9 message per match to the community relay. A deleted
-- subscription is a tombstone so a stale write cannot resurrect it.
CREATE TABLE webhook_gateway_subscriptions (
    id UUID PRIMARY KEY,
    integration_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    filter TEXT CHECK (filter IS NULL OR length(filter) BETWEEN 1 AND 4096),
    revision BIGINT NOT NULL CHECK (revision > 0),
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX webhook_gateway_subscriptions_live ON webhook_gateway_subscriptions (integration_id) WHERE deleted_at IS NULL;
CREATE INDEX webhook_gateway_subscriptions_tombstones ON webhook_gateway_subscriptions (deleted_at) WHERE deleted_at IS NOT NULL;

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
    PRIMARY KEY (integration_id, delivery_id, subscription_id)
);
CREATE INDEX webhook_gateway_messages_due ON webhook_gateway_messages (next_attempt_at) WHERE state = 'queued';
CREATE INDEX webhook_gateway_messages_subscription ON webhook_gateway_messages (subscription_id) WHERE state = 'queued';
CREATE INDEX webhook_gateway_messages_retention ON webhook_gateway_messages (completed_at) WHERE state <> 'queued';
