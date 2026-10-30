CREATE TABLE nip_fi_profile_policies (
    issuer TEXT PRIMARY KEY,
    label TEXT,
    community_hosts TEXT[],
    refresh_seconds BIGINT NOT NULL CHECK (refresh_seconds BETWEEN 0 AND 86400),
    generation BIGINT NOT NULL DEFAULT 1
);
