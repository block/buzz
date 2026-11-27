ALTER TABLE webhook_gateway_applied_operations
    ADD COLUMN request_fingerprint TEXT NOT NULL DEFAULT '' CHECK (length(request_fingerprint) <= 64);
ALTER TABLE webhook_gateway_applied_operations
    ALTER COLUMN request_fingerprint DROP DEFAULT;
