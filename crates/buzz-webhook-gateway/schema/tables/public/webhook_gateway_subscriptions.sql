CREATE TABLE webhook_gateway_subscriptions (
    id UUID PRIMARY KEY,
    integration_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    filter TEXT CHECK (filter IS NULL OR length(filter) BETWEEN 1 AND 4096),
    message_template TEXT CHECK (message_template IS NULL OR length(message_template) BETWEEN 1 AND 4096),
    revision BIGINT NOT NULL CHECK (revision > 0),
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX webhook_gateway_subscriptions_live ON webhook_gateway_subscriptions (integration_id) WHERE deleted_at IS NULL;
CREATE INDEX webhook_gateway_subscriptions_tombstones ON webhook_gateway_subscriptions (deleted_at) WHERE deleted_at IS NOT NULL;
