-- Extend the durable, provider-neutral protected-domain marker with
-- authorization invalidation authority.
--
-- Generations are allocated under a per-community row lock. Receipts make
-- retries idempotent, while selector floors retain the strongest committed
-- fail-closed effect. Redis fan-out is only a hint to read these tables.

CREATE TABLE authorization_invalidation_receipts (
    community_id        UUID NOT NULL REFERENCES communities(id),
    event_id            UUID NOT NULL,
    generation          BIGINT NOT NULL CHECK (generation > 0),
    request_fingerprint BYTEA NOT NULL CHECK (length(request_fingerprint) = 32),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, event_id),
    UNIQUE (community_id, generation)
);

CREATE TABLE authorization_invalidation_floors (
    community_id          UUID NOT NULL REFERENCES communities(id),
    selector_kind         TEXT NOT NULL CHECK (selector_kind IN (
        'principal_fingerprint',
        'nostr_key',
        'binding',
        'session',
        'domain',
        'policy_version',
        'delegated_owner'
    )),
    selector_fingerprint  BYTEA NOT NULL CHECK (length(selector_fingerprint) = 32),
    generation            BIGINT NOT NULL CHECK (generation > 0),
    sticky_deny           BOOLEAN NOT NULL DEFAULT FALSE,
    binding_version_floor BIGINT CHECK (binding_version_floor > 0),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, selector_kind, selector_fingerprint),
    FOREIGN KEY (community_id, generation)
        REFERENCES authorization_invalidation_receipts (community_id, generation),
    CHECK ((selector_kind = 'binding') = (binding_version_floor IS NOT NULL))
);

CREATE INDEX idx_authorization_invalidation_floors_generation
    ON authorization_invalidation_floors (community_id, generation);
