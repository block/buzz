CREATE TABLE media_uploaders (
    community_id UUID NOT NULL,
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    pubkey BYTEA NOT NULL CHECK (length(pubkey) = 32),
    PRIMARY KEY (community_id, sha256, pubkey)
);
