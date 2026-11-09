SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Retain old receipt intent and INSERT compatibility without a data rewrite.
ALTER TABLE community_auth_upgrades
    ADD COLUMN requested_requires_nip_fi BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN expected_policy_generation BIGINT,
    ADD COLUMN expected_requires_nip_fi BOOLEAN;
