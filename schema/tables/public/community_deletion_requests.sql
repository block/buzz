-- ── Whole-community deletion control plane (migration 0029) ─────────────────
CREATE TABLE community_deletion_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    community_id UUID NOT NULL REFERENCES communities(id),
    community_host TEXT NOT NULL,
    stage TEXT NOT NULL DEFAULT 'submitted' CHECK (stage IN (
        'submitted', 'inventoried', 'approved', 'fenced', 'drained',
        'bindings_removed', 'postgres_purged', 'cache_purged',
        'logically_verified', 'retention_pending', 'aborted'
    )),
    requested_by TEXT NOT NULL,
    request_origin TEXT NOT NULL DEFAULT 'operator'
        CHECK (request_origin IN ('operator', 'owner')),
    owner_pubkey TEXT,
    mediating_operator_pubkey TEXT,
    acknowledgement_version INTEGER,
    reason TEXT,
    schema_manifest JSONB,
    storage_manifest JSONB,
    destructive_storage_manifest JSONB,
    destructive_storage_frozen_at TIMESTAMPTZ,
    inventory_manifest JSONB,
    inventory_digest BYTEA CHECK (inventory_digest IS NULL OR length(inventory_digest) = 32),
    inventory_frozen_at TIMESTAMPTZ,
    fence_generation BIGINT CHECK (fence_generation IS NULL OR fence_generation > 0),
    lease_owner TEXT,
    lease_generation BIGINT NOT NULL DEFAULT 0 CHECK (lease_generation >= 0),
    lease_until TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    retry_count INTEGER NOT NULL DEFAULT 0 CHECK (retry_count >= 0),
    retry_stage TEXT CHECK (retry_stage IS NULL OR retry_stage IN (
        'submitted', 'approved', 'fenced', 'drained', 'bindings_removed',
        'postgres_purged', 'cache_purged', 'logically_verified'
    )),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT,
    last_error_at TIMESTAMPTZ,
    blocked_at TIMESTAMPTZ,
    blocked_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    pre_quiesce_archived_at TIMESTAMPTZ,
    quiescing_started_at TIMESTAMPTZ,
    aborted_by TEXT,
    abort_reason TEXT,
    aborted_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    CHECK ((blocked_at IS NULL) = (blocked_reason IS NULL)),
    CHECK ((stage = 'aborted') = (aborted_at IS NOT NULL)),
    CHECK ((aborted_at IS NULL) = (aborted_by IS NULL)),
    CHECK ((aborted_at IS NULL) = (abort_reason IS NULL)),
    CHECK ((inventory_frozen_at IS NULL) = (inventory_digest IS NULL)),
    CONSTRAINT community_deletion_owner_provenance CHECK (
        (request_origin = 'operator'
            AND owner_pubkey IS NULL
            AND mediating_operator_pubkey IS NULL
            AND acknowledgement_version IS NULL)
        OR
        (request_origin = 'owner'
            AND NOT (owner_pubkey IS NULL)
            AND NOT (mediating_operator_pubkey IS NULL)
            AND NOT (acknowledgement_version IS NULL)
            AND owner_pubkey ~ '^[0-9a-f]{64}$'
            AND mediating_operator_pubkey ~ '^[0-9a-f]{64}$'
            AND acknowledgement_version BETWEEN 1 AND 32767
            AND requested_by = owner_pubkey)
    ),
    UNIQUE (id, community_id, inventory_digest)
);
CREATE UNIQUE INDEX community_deletion_requests_active_community
    ON community_deletion_requests (community_id)
    WHERE stage <> 'aborted';
CREATE INDEX community_deletion_requests_runnable
    ON community_deletion_requests (next_attempt_at, created_at)
    WHERE blocked_at IS NULL
      AND stage IN ('approved', 'fenced', 'drained', 'bindings_removed',
                    'postgres_purged', 'cache_purged', 'logically_verified');
CREATE INDEX community_deletion_requests_lease
    ON community_deletion_requests (lease_until) WHERE lease_owner IS NOT NULL;
CREATE INDEX community_deletion_requests_owner_preparable
    ON community_deletion_requests (next_attempt_at, created_at)
    WHERE request_origin = 'owner'
      AND stage = 'submitted'
      AND blocked_at IS NULL;
CREATE INDEX community_deletion_requests_owner_quota_reservations
    ON community_deletion_requests (owner_pubkey)
    INCLUDE (community_id, completed_at)
    WHERE request_origin = 'owner'
      AND stage <> 'aborted';
