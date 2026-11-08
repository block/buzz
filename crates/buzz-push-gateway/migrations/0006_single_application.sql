-- The new capability starts with fresh installation authority. Do not silently
-- retain fingerprints made with the removed application-profile namespace.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM push_gateway_installations) THEN
        RAISE EXCEPTION 'buzz-push-v1 requires an empty gateway authority store. Stop old gateway replicas and provision a fresh dedicated database before upgrading.';
    END IF;
END
$$;
ALTER TABLE push_gateway_installations DROP COLUMN app_profile;
CREATE UNIQUE INDEX push_gateway_installations_active_token
    ON push_gateway_installations (token_fingerprint) WHERE revoked_at IS NULL;
