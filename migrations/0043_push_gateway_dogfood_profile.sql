-- The internal MVP accepts only the dogfood application profile. The legacy
-- profile names encoded APNs transport environment rather than a verified
-- application identity, so they cannot be mapped safely to dogfood authority.
-- Retire their delegations and installations before narrowing the constraint.
DELETE FROM push_gateway_delegations
WHERE installation_id IN (
    SELECT id
    FROM push_gateway_installations
    WHERE app_profile <> 'buzz-ios-dogfood'
);

DELETE FROM push_gateway_installations
WHERE app_profile <> 'buzz-ios-dogfood';

ALTER TABLE push_gateway_installations
    DROP CONSTRAINT push_gateway_installations_app_profile_check;
ALTER TABLE push_gateway_installations
    ADD CONSTRAINT push_gateway_installations_app_profile_check
    CHECK (app_profile = 'buzz-ios-dogfood');
