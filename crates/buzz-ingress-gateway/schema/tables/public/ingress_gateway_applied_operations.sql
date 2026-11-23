CREATE TABLE ingress_gateway_applied_operations (
    operation_id UUID PRIMARY KEY,
    integration_id UUID NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    response JSONB NOT NULL,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ingress_gateway_applied_operations_retention ON ingress_gateway_applied_operations (applied_at);
