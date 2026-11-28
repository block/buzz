-- Expand the NIP-IA consent check for Operator archive.
-- Migration 0110 validates it separately from this DDL lock.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- NIP-IA `consent=relay`: archived by an operator action, not a signed request.
-- Such rows store '' in request_event_id; the column stays NOT NULL because
-- the deployed relay decodes it as a non-null string.
ALTER TABLE archived_identities
    DROP CONSTRAINT archived_identities_consent_path_check,
    ADD CONSTRAINT archived_identities_consent_path_check
        CHECK (consent_path IN ('self', 'owner', 'admin', 'relay')) NOT VALID;
