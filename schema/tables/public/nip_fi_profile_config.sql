CREATE TABLE nip_fi_profile_config (
    singleton BOOLEAN PRIMARY KEY CHECK (singleton),
    revision BIGINT NOT NULL CHECK (revision >= 0),
    digest TEXT NOT NULL
);
