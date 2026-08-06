-- Explicit relay-verified identity revocation and rotation semantics.
--
-- principal: disables every key for the issuer-qualified principal.
-- key: revokes only this key; a different key still requires an explicit
--      operator rotation because ordinary authentication never replaces an
--      active binding.
-- rotation: records the old key retired by an authorized atomic rotation.

ALTER TABLE identity_bindings
    ADD COLUMN revocation_scope TEXT NOT NULL DEFAULT 'principal'
    CHECK (revocation_scope IN ('principal', 'key', 'rotation')),
    ADD COLUMN rotation_completed_at TIMESTAMPTZ,
    ADD COLUMN rotated_to_pubkey BYTEA,
    ADD COLUMN rotation_by BYTEA,
    ADD COLUMN rotation_reason TEXT,
    ADD CONSTRAINT chk_identity_bindings_rotation_state CHECK (
        (rotation_completed_at IS NULL
            AND rotated_to_pubkey IS NULL
            AND rotation_by IS NULL
            AND rotation_reason IS NULL)
        OR
        (rotation_completed_at IS NOT NULL
            AND rotated_to_pubkey IS NOT NULL
            AND length(rotated_to_pubkey) = 32
            AND (rotation_by IS NULL OR length(rotation_by) = 32)
            AND rotation_reason IS NOT NULL
            AND length(rotation_reason) > 0)
    );

CREATE INDEX idx_identity_bindings_revoked_principal
    ON identity_bindings (community_id, issuer, uid)
    WHERE revoked_at IS NOT NULL AND revocation_scope = 'principal';

-- Principal status is separate from key history so operators can disable a
-- principal before first enrollment and after a single-key revocation.
CREATE TABLE identity_principals (
    community_id    UUID NOT NULL REFERENCES communities(id),
    issuer          TEXT NOT NULL,
    uid             TEXT NOT NULL,
    disabled_at     TIMESTAMPTZ,
    disabled_by     BYTEA,
    disabled_reason TEXT,
    PRIMARY KEY (community_id, issuer, uid),
    CHECK (length(issuer) > 0),
    CHECK (length(uid) > 0),
    CHECK (disabled_by IS NULL OR length(disabled_by) = 32),
    CHECK ((disabled_at IS NULL) = (disabled_reason IS NULL))
);

-- Rows revoked before this lifecycle migration represented principal-level
-- disablement. Preserve that security state instead of allowing the same uid
-- to re-enroll with a fresh key after upgrade.
INSERT INTO identity_principals
    (community_id, issuer, uid, disabled_at, disabled_by, disabled_reason)
SELECT DISTINCT ON (community_id, issuer, uid)
    community_id,
    issuer,
    uid,
    revoked_at,
    revoked_by,
    COALESCE(NULLIF(revoked_reason, ''), 'legacy principal revocation')
FROM identity_bindings
WHERE revoked_at IS NOT NULL
ORDER BY community_id, issuer, uid, revoked_at ASC;

-- A revoked credential cannot be rebound to a different principal in the
-- same community. Explicit rotation may consume an old revoked key, but may
-- never select a revoked key as the replacement.
CREATE TABLE identity_revoked_keys (
    community_id UUID NOT NULL REFERENCES communities(id),
    pubkey       BYTEA NOT NULL,
    revoked_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_by   BYTEA,
    reason       TEXT NOT NULL,
    PRIMARY KEY (community_id, pubkey),
    CHECK (length(pubkey) = 32),
    CHECK (revoked_by IS NULL OR length(revoked_by) = 32),
    CHECK (length(reason) > 0)
);

-- Preserve every pre-migration revoked credential as a community-wide key
-- tombstone. This prevents a legacy-revoked key from binding to a different
-- issuer-qualified principal after upgrade.
INSERT INTO identity_revoked_keys
    (community_id, pubkey, revoked_at, revoked_by, reason)
SELECT DISTINCT ON (community_id, pubkey)
    community_id,
    pubkey,
    revoked_at,
    revoked_by,
    COALESCE(NULLIF(revoked_reason, ''), 'legacy key revocation')
FROM identity_bindings
WHERE revoked_at IS NOT NULL
ORDER BY community_id, pubkey, revoked_at ASC;
