-- Durable caller idempotency for channel messages. The receipt and the event
-- are written in one transaction so a retry can always return a canonical ID.
CREATE TABLE message_idempotency_receipts (
    community_id UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    author_pubkey BYTEA NOT NULL CHECK (octet_length(author_pubkey) = 32),
    channel_id UUID NOT NULL,
    idempotency_key BYTEA NOT NULL CHECK (octet_length(idempotency_key) = 32),
    semantic_digest BYTEA NOT NULL CHECK (octet_length(semantic_digest) = 32),
    event_id BYTEA NOT NULL CHECK (octet_length(event_id) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, author_pubkey, channel_id, idempotency_key)
);
