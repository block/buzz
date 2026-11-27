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
