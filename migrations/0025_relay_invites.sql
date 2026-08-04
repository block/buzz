-- Use-limited invite links: durable invite records for atomic redemption.
--
-- Stateless HMAC bearer tokens (v1) cannot enforce use limits: their signed
-- payload is immutable and no invite row exists to record consumption. This
-- migration introduces a durable `relay_invites` table that stores only the
-- SHA-256 hash of an opaque v2 code, never the reusable bearer secret itself.
--
-- Every lookup binds both (community_id, token_hash) so a code presented on
-- the wrong tenant host returns Invalid — there is no cross-tenant lookup by
-- hash alone. `FOR UPDATE` during claim serializes concurrent claims for one
-- invite across relay processes; membership insertion, join-policy evidence,
-- and use_count increment share a single commit so exactly one claimant can
-- win the final slot.
--
-- max_uses is optional: NULL means unlimited (preserving current behavior).
-- use_count is always incremented for new members, even when unlimited, for
-- observability. role is pinned to 'member' — invite links never grant admin.
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
