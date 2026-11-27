CREATE TABLE webhook_gateway_integrations (
    id UUID PRIMARY KEY,
    community_host TEXT NOT NULL CHECK (community_host = lower(community_host) AND length(community_host) BETWEEN 1 AND 253),
    provider TEXT NOT NULL CHECK (provider IN ('github', 'pagerduty', 'sentry', 'incident_io')),
    delivery_mode TEXT NOT NULL DEFAULT 'channel_subscriptions' CHECK (delivery_mode IN ('channel_subscriptions', 'caller_selects_channel')),
    channel_expr TEXT CHECK (channel_expr IS NULL OR length(channel_expr) BETWEEN 1 AND 4096),
    message_expr TEXT CHECK (message_expr IS NULL OR length(message_expr) BETWEEN 1 AND 4096),
    message_data_expr TEXT CHECK (message_data_expr IS NULL OR length(message_data_expr) BETWEEN 1 AND 4096),
    group_key_expr TEXT CHECK (group_key_expr IS NULL OR length(group_key_expr) BETWEEN 1 AND 4096),
    group_key_action TEXT NOT NULL DEFAULT 'reply_in_thread' CHECK (group_key_action IN ('reply_in_thread', 'update_message')),
    revision BIGINT NOT NULL CHECK (revision > 0),
    enabled BOOLEAN NOT NULL DEFAULT true,
    disabled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX webhook_gateway_integrations_community ON webhook_gateway_integrations (community_host);
