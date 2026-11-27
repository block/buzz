CREATE TABLE webhook_gateway_applied_operations (
    operation_id UUID PRIMARY KEY,
    integration_id UUID NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) <= 64),
    response JSONB NOT NULL,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX webhook_gateway_applied_operations_retention ON webhook_gateway_applied_operations (applied_at);
