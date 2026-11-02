SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE TABLE nip_fi_profile_heads (
    community_id UUID NOT NULL,
    pubkey TEXT NOT NULL,
    issuer TEXT NOT NULL,
    issued_at NUMERIC(30, 0) NOT NULL,
    event JSONB NOT NULL,
    PRIMARY KEY (community_id, pubkey)
);
