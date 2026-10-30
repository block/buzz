SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';



CREATE TABLE nip_fi_profile_policies (
    issuer TEXT PRIMARY KEY,
    label TEXT,
    community_hosts TEXT[],
    refresh_seconds BIGINT NOT NULL CHECK (refresh_seconds BETWEEN 0 AND 86400),
    generation BIGINT NOT NULL DEFAULT 1
);

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

CREATE TABLE nip_fi_profile_assertions (
    community_id UUID NOT NULL,
    issuer TEXT NOT NULL,
    pubkey TEXT NOT NULL,
    issued_at NUMERIC(30, 0) NOT NULL,
    PRIMARY KEY (community_id, issuer, pubkey)
);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('nip_fi_profile_policies', 'deployment-wide publication policy epochs, retained after removal');

CREATE TABLE nip_fi_profile_config (
    singleton BOOLEAN PRIMARY KEY CHECK (singleton),
    revision BIGINT NOT NULL CHECK (revision >= 0),
    digest TEXT NOT NULL
);

INSERT INTO _operator_global_tables (table_name, reason) VALUES ('nip_fi_profile_config', 'monotonic operator publication configuration revision');

ALTER TABLE relay_members ADD COLUMN access_generation UUID;
ALTER TABLE relay_members ALTER COLUMN access_generation SET DEFAULT gen_random_uuid();

ALTER TABLE community_nip_fi_issuers ADD COLUMN access_generation UUID;
ALTER TABLE community_nip_fi_issuers ALTER COLUMN access_generation SET DEFAULT gen_random_uuid();

ALTER TABLE community_bans ADD COLUMN access_generation UUID;
