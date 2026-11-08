CREATE TABLE media_publications (
    community_id UUID NOT NULL,
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    PRIMARY KEY (community_id, sha256)
);
