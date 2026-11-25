CREATE TABLE community_serving_write_leases (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    community_id UUID NOT NULL,
    operation TEXT NOT NULL,
    owner TEXT NOT NULL,
    -- Verified media signer and its community owner at admission; owner above is the executor.
    actor_pubkey BYTEA CHECK (actor_pubkey IS NULL OR octet_length(actor_pubkey) = 32),
    actor_owner_pubkey BYTEA CHECK (actor_owner_pubkey IS NULL OR octet_length(actor_owner_pubkey) = 32),
    generation BIGINT NOT NULL DEFAULT 1 CHECK (generation > 0),
    -- Community fence generation observed when this lease was acquired.
    fence_generation BIGINT NOT NULL CHECK (fence_generation >= 0),
    -- Attributed uploads use infinity here so old reapers cannot erase uncertain work.
    lease_until TIMESTAMPTZ NOT NULL,
    media_lease_until TIMESTAMPTZ,
    media_write_started BOOLEAN NOT NULL DEFAULT false,
    CHECK ((actor_pubkey IS NULL) = (media_lease_until IS NULL)),
    heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX community_serving_write_leases_active
    ON community_serving_write_leases (community_id, lease_until);
