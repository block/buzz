-- Existing active leases use the retired nip-pl contract. Cut over while push
-- is disabled; do not silently convert or discard their authority.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM push_leases WHERE active) THEN
        RAISE EXCEPTION 'buzz-push-v1 requires no active legacy push leases. Retire legacy leases before upgrading.';
    END IF;
END
$$;
ALTER TABLE push_leases DROP COLUMN app_profile;
ALTER TABLE push_leases ADD CHECK (
    (active AND endpoint_hash IS NOT NULL AND endpoint_grant IS NOT NULL AND max_class IS NOT NULL AND subscriptions IS NOT NULL)
    OR (NOT active AND endpoint_hash IS NULL AND endpoint_grant IS NULL AND max_class IS NULL AND subscriptions IS NULL)
);
CREATE UNIQUE INDEX push_leases_endpoint_unique
    ON push_leases (community_id, author, endpoint_hash) WHERE active;
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
