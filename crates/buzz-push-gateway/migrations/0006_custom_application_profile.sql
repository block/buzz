-- Add the operator-owned universal Apple application without changing the
-- existing dogfood profile or rewriting enrolled authority.
ALTER TABLE push_gateway_installations
    DROP CONSTRAINT push_gateway_installations_app_profile_check;
ALTER TABLE push_gateway_installations
    ADD CONSTRAINT push_gateway_installations_app_profile_check
    CHECK (app_profile IN ('buzz-ios-dogfood', 'buzz-ios-custom'));
