-- Contract step for 0087's widened consent CHECK. Every existing row already
-- satisfies it; the scan does not block writes.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
ALTER TABLE archived_identities VALIDATE CONSTRAINT archived_identities_consent_path_check;
