-- FounderPortal collaboration bridge persistence.
--
-- Ownership boundary: this schema stores cross-system identifiers and mapping
-- lifecycle only. It is not authoritative for FounderPortal tenant membership
-- or Buzz community/channel membership and contains no credentials, roles,
-- permissions, entity IDs, or HR data.

CREATE SCHEMA IF NOT EXISTS founderportal_bridge;

CREATE TABLE founderportal_bridge.community_mappings (
    tenant_id TEXT PRIMARY KEY CHECK (tenant_id = btrim(tenant_id) AND tenant_id <> ''),
    buzz_community_id UUID,
    status TEXT NOT NULL CHECK (status IN (
        'provisioning', 'active', 'revoking', 'revoked', 'error', 'archived'
    )),
    last_error_code TEXT CHECK (
        last_error_code IS NULL OR (
            last_error_code = btrim(last_error_code)
            AND last_error_code <> ''
            AND length(last_error_code) <= 128
        )
    ),
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT community_mapping_lifecycle_shape CHECK (
        (status = 'provisioning' AND buzz_community_id IS NULL)
        OR (status IN ('active', 'revoking', 'revoked', 'archived') AND buzz_community_id IS NOT NULL)
        OR status = 'error'
    )
);

CREATE UNIQUE INDEX community_mappings_buzz_community_uidx
    ON founderportal_bridge.community_mappings (buzz_community_id)
    WHERE buzz_community_id IS NOT NULL;

CREATE TABLE founderportal_bridge.identity_mappings (
    tenant_id TEXT NOT NULL CHECK (tenant_id = btrim(tenant_id) AND tenant_id <> ''),
    actor_type TEXT NOT NULL CHECK (actor_type IN ('human', 'agent')),
    actor_id TEXT NOT NULL CHECK (actor_id = btrim(actor_id) AND actor_id <> ''),
    buzz_pubkey TEXT NOT NULL CHECK (
        buzz_pubkey = lower(buzz_pubkey)
        AND buzz_pubkey ~ '^[0-9a-f]{64}$'
    ),
    status TEXT NOT NULL CHECK (status IN (
        'provisioning', 'active', 'revoking', 'revoked', 'error'
    )),
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, actor_type, actor_id),
    UNIQUE (tenant_id, buzz_pubkey)
);

COMMENT ON SCHEMA founderportal_bridge IS
    'FounderPortal-owned collaboration bridge identifiers and convergence state; not Buzz domain authority.';
COMMENT ON TABLE founderportal_bridge.community_mappings IS
    'One FounderPortal tenant to at most one Buzz community mapping.';
COMMENT ON TABLE founderportal_bridge.identity_mappings IS
    'One tenant-scoped FounderPortal human or agent actor to one Buzz public key; contains public keys only.';

-- These bridge-owned tables use FounderPortal tenant_id as their partition
-- boundary and intentionally do not use Buzz community_id as tenant authority.
INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('community_mappings', 'FounderPortal bridge-owned global mapping registry; tenant_id is the external authority boundary'),
    ('identity_mappings', 'FounderPortal bridge-owned global mapping registry; tenant_id is the external authority boundary');
