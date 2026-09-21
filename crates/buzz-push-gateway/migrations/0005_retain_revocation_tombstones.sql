-- Revoked installations remain as retry tombstones until authority expiry.
-- Restrict ownership uniqueness to live rows so a replacement can enroll
-- without deleting the old installation's idempotency state.
ALTER TABLE push_gateway_installations
    DROP CONSTRAINT push_gateway_installations_app_attest_key_id_key;
ALTER TABLE push_gateway_installations
    DROP CONSTRAINT push_gateway_installations_app_profile_token_fingerprint_key;

CREATE UNIQUE INDEX push_gateway_installations_active_app_attest_key
    ON push_gateway_installations (app_attest_key_id)
    WHERE revoked_at IS NULL;

CREATE UNIQUE INDEX push_gateway_installations_active_profile_token
    ON push_gateway_installations (app_profile, token_fingerprint)
    WHERE revoked_at IS NULL;
