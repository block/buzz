-- Replay ledger for relay delivery request ids, retained until expiry.
-- finished is false while the admitted attempt is still being delivered.
CREATE TABLE push_gateway_delivery_request_replays (
    relay_pubkey BYTEA NOT NULL CHECK (length(relay_pubkey) = 32),
    request_id UUID NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    finished BOOLEAN NOT NULL DEFAULT true,
    PRIMARY KEY (relay_pubkey, request_id)
);

CREATE INDEX push_gateway_delivery_request_replays_expiry ON push_gateway_delivery_request_replays (expires_at);
