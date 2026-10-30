CREATE TABLE nip_fi_profile_heads (
    community_id UUID NOT NULL,
    pubkey TEXT NOT NULL,
    issuer TEXT NOT NULL,
    policy_generation BIGINT NOT NULL,
    issued_at NUMERIC(30, 0) NOT NULL,
    database_binding BOOLEAN NOT NULL DEFAULT TRUE,
    event JSONB NOT NULL,
    active BOOLEAN NOT NULL,
    retract_before BIGINT,
    broadcast_pending BOOLEAN NOT NULL DEFAULT TRUE,
    PRIMARY KEY (community_id, pubkey)
);
