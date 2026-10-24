-- ── Relay invites (use-limited invite links) ──────────────────────────────────
-- Conformance: durable invite records for atomic redemption, community-scoped.
-- Stores only SHA-256(code) as 32-byte BYTEA; never the reusable bearer code.
-- PK and UNIQUE both lead with community_id. max_uses NULL = unlimited.

CREATE TABLE relay_invites (
    community_id  UUID        NOT NULL REFERENCES communities(id),
    id           UUID        NOT NULL DEFAULT gen_random_uuid(),
    token_hash   BYTEA       NOT NULL CHECK (length(token_hash) = 32),
    role         TEXT        NOT NULL DEFAULT 'member' CHECK (role = 'member'),
    max_uses     INTEGER     CHECK (max_uses BETWEEN 1 AND 10000),
    use_count    INTEGER     NOT NULL DEFAULT 0 CHECK (use_count >= 0),
    expires_at   TIMESTAMPTZ NOT NULL,
    created_by   TEXT        NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, id),
    UNIQUE (community_id, token_hash),
    CHECK (max_uses IS NULL OR use_count <= max_uses)
);

CREATE INDEX relay_invites_expires_at_idx ON relay_invites (expires_at);
