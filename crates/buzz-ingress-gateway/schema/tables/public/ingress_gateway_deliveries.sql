CREATE TABLE ingress_gateway_deliveries (
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
CREATE INDEX ingress_gateway_deliveries_due ON ingress_gateway_deliveries (next_attempt_at) WHERE state = 'queued';
CREATE INDEX ingress_gateway_deliveries_retention ON ingress_gateway_deliveries (completed_at) WHERE state <> 'queued';
