-- Corporate identity bindings.
--
-- This is the relay-side foundation for mapping a corporate IdP subject to a
-- Nostr pubkey. It is intentionally not a full grant/session model: lifecycle
-- operations such as admin revocation, rotation workflows, and live connection
-- eviction are follow-up work, but the columns/indexes below preserve those
-- states without requiring a later destructive schema rewrite.

CREATE TABLE identity_bindings (
    community_id    UUID NOT NULL REFERENCES communities(id),
    uid             TEXT NOT NULL,
    pubkey          BYTEA NOT NULL,
    display_name    TEXT,
    source          TEXT NOT NULL CHECK (source IN ('jwt_npub', 'db_binding')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at      TIMESTAMPTZ,
    revoked_by      BYTEA,
    revoked_reason  TEXT,
    CONSTRAINT chk_identity_bindings_uid_not_empty CHECK (length(uid) > 0),
    CONSTRAINT chk_identity_bindings_pubkey_len CHECK (length(pubkey) = 32),
    CONSTRAINT chk_identity_bindings_revoked_by_len CHECK (revoked_by IS NULL OR length(revoked_by) = 32)
);

CREATE UNIQUE INDEX idx_identity_bindings_active_uid
    ON identity_bindings (community_id, uid)
    WHERE revoked_at IS NULL;

CREATE UNIQUE INDEX idx_identity_bindings_active_pubkey
    ON identity_bindings (community_id, pubkey)
    WHERE revoked_at IS NULL;

CREATE INDEX idx_identity_bindings_pubkey
    ON identity_bindings (community_id, pubkey);
