CREATE TABLE nip_fi_profile_assertions (
    community_id UUID NOT NULL,
    issuer TEXT NOT NULL,
    pubkey TEXT NOT NULL,
    issued_at NUMERIC(30, 0) NOT NULL,
    PRIMARY KEY (community_id, issuer, pubkey)
);
