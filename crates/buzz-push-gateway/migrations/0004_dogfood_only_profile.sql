-- The internal MVP now exposes only the dogfood application profile. Retire
-- dormant App Store authority before narrowing the server-owned registry.
DELETE FROM push_gateway_delegations
WHERE installation_id IN (
    SELECT id FROM push_gateway_installations
    WHERE app_profile = 'buzz-ios-app-store'
);

DELETE FROM push_gateway_installations
WHERE app_profile = 'buzz-ios-app-store';

ALTER TABLE push_gateway_installations
    DROP CONSTRAINT push_gateway_installations_app_profile_check;
ALTER TABLE push_gateway_installations
    ADD CONSTRAINT push_gateway_installations_app_profile_check
    CHECK (app_profile = 'buzz-ios-dogfood');
