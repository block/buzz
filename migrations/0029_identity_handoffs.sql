-- Public-key-bound identity handoffs (v3).
--
-- This is intentionally separate from relay_invites. A v3 code can never be
-- interpreted as a generic v2 invite, which keeps mixed-version rollout and
-- rollback fail-closed. Only SHA-256(code) is persisted; the bearer code is
-- returned once by the mint path.
--
-- Incarnations are also stored only as a domain-separated digest. Revocation
-- fences intentionally outlive the 30-day handoff diagnostic window so a
-- delayed mint cannot resurrect a destructively revoked link incarnation.
CREATE TABLE identity_handoff_revoked_incarnations (
    community_id     UUID        NOT NULL REFERENCES communities(id),
    incarnation_hash BYTEA       NOT NULL CHECK (length(incarnation_hash) = 32),
    revoked_at       TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, incarnation_hash)
);

CREATE TABLE identity_handoffs (
    community_id     UUID        NOT NULL REFERENCES communities(id),
    id               UUID        NOT NULL DEFAULT gen_random_uuid(),
    token_hash       BYTEA       NOT NULL CHECK (length(token_hash) = 32),
    expected_pubkey  TEXT        NOT NULL CHECK (
        expected_pubkey = lower(expected_pubkey)
        AND length(expected_pubkey) = 64
        AND expected_pubkey ~ '^[0-9a-f]{64}$'
    ),
    incarnation_hash BYTEA       NOT NULL CHECK (length(incarnation_hash) = 32),
    state            TEXT        NOT NULL DEFAULT 'active' CHECK (
        state IN ('active', 'claimed', 'superseded', 'invalidated', 'expired')
    ),
    created_by       TEXT        NOT NULL CHECK (length(created_by) BETWEEN 1 AND 256),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    expires_at       TIMESTAMPTZ NOT NULL,
    terminal_at      TIMESTAMPTZ,
    PRIMARY KEY (community_id, id),
    UNIQUE (community_id, token_hash),
    CHECK (expires_at > created_at),
    CHECK (
        (state = 'active' AND terminal_at IS NULL)
        OR (state <> 'active' AND terminal_at IS NOT NULL)
    )
);

-- Database backstop for the one-live-handoff product decision. Application
-- transactions still serialize on the community/pubkey advisory lock before
-- normalizing or superseding rows.
CREATE UNIQUE INDEX identity_handoffs_one_active_pubkey_idx
    ON identity_handoffs (community_id, expected_pubkey)
    WHERE state = 'active';

CREATE INDEX identity_handoffs_terminal_retention_idx
    ON identity_handoffs (terminal_at)
    WHERE terminal_at IS NOT NULL;
