-- The unauthenticated challenge route applies a deployment-global rolling
-- issuance quota. Keep its count query bounded as challenge volume grows.
CREATE INDEX push_gateway_challenges_created_at
    ON push_gateway_challenges (created_at);
