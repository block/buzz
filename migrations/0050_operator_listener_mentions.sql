-- Deployment-global operator-listener mention delivery.
-- Listener registrations span communities; community_id on the outbox is event
-- provenance, not a tenant boundary.

-- Migration 0044 restored this function to the pre-0041 exclusion set. Extend
-- it here so migrated databases keep the outbox outside tenant fencing and
-- community-deletion catalog discovery, matching the desired-state schema.
CREATE OR REPLACE FUNCTION community_write_fence_excluded_table(target NAME) RETURNS BOOLEAN
LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT target::TEXT = ANY (ARRAY[
        'community_deletion_requests', 'community_deletion_approvals',
        'community_deletion_checkpoints', 'community_serving_write_leases',
        'community_deletion_executor_heartbeats', 'product_feedback',
        'rate_limit_violations', 'operator_listener_outbox'
    ]::TEXT[])
$$;

CREATE TABLE operator_listener_pubkeys (
    listener_pubkey BYTEA NOT NULL CHECK (length(listener_pubkey) = 32),
    target_pubkey   BYTEA NOT NULL CHECK (length(target_pubkey) = 32),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (listener_pubkey, target_pubkey)
);
CREATE INDEX operator_listener_pubkeys_target
    ON operator_listener_pubkeys (target_pubkey, listener_pubkey);
CREATE INDEX operator_listener_pubkeys_created_at
    ON operator_listener_pubkeys (created_at);

CREATE TABLE operator_listener_outbox (
    id                UUID NOT NULL PRIMARY KEY DEFAULT gen_random_uuid(),
    listener_pubkey   BYTEA NOT NULL CHECK (length(listener_pubkey) = 32),
    target_pubkey     BYTEA NOT NULL CHECK (length(target_pubkey) = 32),
    community_id      UUID NOT NULL,
    event_id          BYTEA NOT NULL CHECK (length(event_id) = 32),
    event_kind        INTEGER NOT NULL,
    event_created_at  TIMESTAMPTZ NOT NULL,
    state             TEXT NOT NULL DEFAULT 'pending'
                      CHECK (state IN ('pending', 'sending')),
    attempts          INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_until       TIMESTAMPTZ,
    claim_id          UUID,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (listener_pubkey, target_pubkey, community_id, event_id)
);
CREATE INDEX operator_listener_outbox_due
    ON operator_listener_outbox (next_attempt_at, created_at, id)
    WHERE state = 'pending';
CREATE INDEX operator_listener_outbox_recovery
    ON operator_listener_outbox (lease_until, created_at, id)
    WHERE state = 'sending';
CREATE INDEX operator_listener_outbox_created_at
    ON operator_listener_outbox (created_at);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('operator_listener_pubkeys', 'deployment-global target registrations for operator listeners'),
    ('operator_listener_outbox', 'deployment-global mention delivery queue; community_id is event provenance');
