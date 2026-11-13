-- Single-use App Attest enrollment challenges. Only the hash is stored.
CREATE TABLE push_gateway_challenges (
    id UUID PRIMARY KEY,
    challenge_hash BYTEA NOT NULL CHECK (length(challenge_hash) = 32),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX push_gateway_challenges_expiry ON push_gateway_challenges (expires_at);

-- The unauthenticated challenge route applies a deployment-global rolling
-- issuance quota. Keep its count query bounded as challenge volume grows.
CREATE INDEX push_gateway_challenges_created_at ON push_gateway_challenges (created_at);
