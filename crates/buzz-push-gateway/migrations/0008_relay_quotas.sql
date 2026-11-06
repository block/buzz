SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE TABLE push_gateway_relay_quotas (
    token_fingerprint BYTEA NOT NULL CHECK (length(token_fingerprint) = 32),
    relay_pubkey BYTEA NOT NULL CHECK (length(relay_pubkey) = 32),
    window_started_at TIMESTAMPTZ NOT NULL,
    admitted BIGINT NOT NULL CHECK (admitted >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (token_fingerprint, relay_pubkey)
);
