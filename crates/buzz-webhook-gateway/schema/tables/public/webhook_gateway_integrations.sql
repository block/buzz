CREATE TABLE webhook_gateway_integrations (
    id UUID PRIMARY KEY,
    community_host TEXT NOT NULL CHECK (community_host = lower(community_host) AND length(community_host) BETWEEN 1 AND 253),
    provider TEXT NOT NULL CHECK (provider IN ('github', 'pagerduty', 'sentry', 'incident_io')),
    revision BIGINT NOT NULL CHECK (revision > 0),
    enabled BOOLEAN NOT NULL DEFAULT true,
    disabled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX webhook_gateway_integrations_community ON webhook_gateway_integrations (community_host);
