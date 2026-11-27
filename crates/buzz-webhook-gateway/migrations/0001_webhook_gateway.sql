-- The webhook gateway owns these tables in its dedicated database.
SET LOCAL lock_timeout = '5s';

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

CREATE TABLE webhook_gateway_integration_secrets (
    id UUID PRIMARY KEY,
    integration_id UUID NOT NULL,
    key_id TEXT NOT NULL CHECK (length(key_id) BETWEEN 1 AND 64),
    secret_ciphertext BYTEA NOT NULL CHECK (octet_length(secret_ciphertext) BETWEEN 1 AND 4096),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    retired_at TIMESTAMPTZ,
    grace_until TIMESTAMPTZ
);
CREATE INDEX webhook_gateway_integration_secrets_active ON webhook_gateway_integration_secrets (integration_id) WHERE retired_at IS NULL;
CREATE INDEX webhook_gateway_integration_secrets_grace ON webhook_gateway_integration_secrets (grace_until) WHERE grace_until IS NOT NULL;

CREATE TABLE webhook_gateway_applied_operations (
    operation_id UUID PRIMARY KEY,
    integration_id UUID NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) <= 64),
    response JSONB NOT NULL,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX webhook_gateway_applied_operations_retention ON webhook_gateway_applied_operations (applied_at);

CREATE TABLE webhook_gateway_deliveries (
    integration_id UUID NOT NULL,
    delivery_id TEXT NOT NULL CHECK (length(delivery_id) BETWEEN 1 AND 256),
    event_type TEXT NOT NULL CHECK (length(event_type) BETWEEN 1 AND 128),
    provider_timestamp TIMESTAMPTZ,
    headers JSONB NOT NULL,
    body BYTEA NOT NULL CHECK (octet_length(body) <= 16777216),
    state TEXT NOT NULL CHECK (state IN ('queued', 'accepted', 'failed')),
    event_id TEXT CHECK (event_id IS NULL OR length(event_id) = 64),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ,
    last_error TEXT,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (integration_id, delivery_id)
);
CREATE INDEX webhook_gateway_deliveries_due ON webhook_gateway_deliveries (next_attempt_at) WHERE state = 'queued';
CREATE INDEX webhook_gateway_deliveries_retention ON webhook_gateway_deliveries (completed_at) WHERE state <> 'queued';
CREATE INDEX webhook_gateway_deliveries_pending_order ON webhook_gateway_deliveries (integration_id, received_at, delivery_id) WHERE state = 'queued';

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
    group_id TEXT,
    group_position BIGINT,
    group_key_action TEXT,
    PRIMARY KEY (integration_id, delivery_id, subscription_id)
);
CREATE INDEX webhook_gateway_messages_due ON webhook_gateway_messages (next_attempt_at) WHERE state = 'queued';
CREATE INDEX webhook_gateway_messages_subscription ON webhook_gateway_messages (subscription_id) WHERE state = 'queued';
CREATE INDEX webhook_gateway_messages_retention ON webhook_gateway_messages (completed_at) WHERE state <> 'queued';
CREATE INDEX webhook_gateway_messages_group_pending ON webhook_gateway_messages (group_id, group_position) WHERE group_id IS NOT NULL AND state = 'queued';

CREATE TABLE webhook_gateway_message_groups (
    id TEXT PRIMARY KEY,
    last_position BIGINT NOT NULL,
    root_event_id TEXT,
    last_signed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX webhook_gateway_message_groups_retention ON webhook_gateway_message_groups (updated_at);
